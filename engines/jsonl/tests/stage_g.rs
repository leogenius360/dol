use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use dol::core::value::{Datum, Value};
use dol::prelude::*;
use dol_conformance::{semantic, validate_engine_basics};
use dol_engine::{DataStream, Engine, ExecutionOptions, ExecutionRow};
use dol_jsonl::{JsonlEngine, JsonlLimits};
use dol_memory::MemoryEngine;

#[derive(Clone, Debug, PartialEq, dol::Model)]
#[dol(key = "stage-g/event", name = "StageGEvent")]
struct Event {
    #[dol(identity)]
    id: u64,
    active: bool,
    score: i64,
    tags: Vec<String>,
    #[dol(optional)]
    nickname: Option<String>,
}

fn event(id: u64, active: bool, score: i64, tags: &[&str], nickname: Option<&str>) -> Event {
    Event {
        id,
        active,
        score,
        tags: tags.iter().map(|value| (*value).to_owned()).collect(),
        nickname: nickname.map(ToOwned::to_owned),
    }
}

fn fixture() -> DataSet<Event> {
    DataSet::try_new([
        event(1, true, 10, &["a", "b"], Some("one")),
        event(2, true, 30, &["c"], None),
        event(3, false, 20, &[], Some("three")),
    ])
    .unwrap()
}

fn json_fixture() -> &'static str {
    concat!(
        "{\"id\":1,\"active\":true,\"score\":10,\"tags\":[\"a\",\"b\"],\"nickname\":\"one\"}\n",
        "{\"id\":2,\"active\":true,\"score\":30,\"tags\":[\"c\"],\"nickname\":null}\n",
        "{\"id\":3,\"active\":false,\"score\":20,\"tags\":[],\"nickname\":\"three\"}\n",
    )
}

#[test]
fn jsonl_advertises_only_exact_incremental_capabilities() {
    let temp = TempDir::new("capabilities");
    temp.write("events.jsonl", json_fixture());
    let engine = JsonlEngine::new(temp.path()).unwrap();
    validate_engine_basics(&engine).unwrap();

    let capabilities = engine.capabilities();
    assert!(capabilities.pipeline.source.is_native());
    assert!(capabilities.pipeline.filter.is_native());
    assert!(capabilities.pipeline.project.is_native());
    assert!(capabilities.pipeline.unnest.is_native());
    assert!(capabilities.pipeline.slice.is_native());
    assert!(!capabilities.pipeline.sort.is_supported());
    assert!(!capabilities.pipeline.distinct.is_supported());
    assert!(!capabilities.pipeline.aggregate.is_supported());
    assert!(!capabilities.pipeline.window.is_supported());
    assert!(!capabilities.pipeline.join.is_supported());
    assert!(!capabilities.pipeline.set.is_supported());
    assert!(!capabilities.pipeline.exists.is_supported());
    assert!(!capabilities.writes.insert.is_supported());
}

#[test]
fn parameterized_incremental_execution_matches_memory_reference() {
    let temp = TempDir::new("differential");
    temp.write("events.jsonl", json_fixture());

    let mut jsonl = JsonlEngine::new(temp.path()).unwrap();
    jsonl.bind::<Event>("events.jsonl").unwrap();

    let mut memory = MemoryEngine::new();
    memory.load(&fixture()).unwrap();

    let minimum = Parameter::<i64>::new("minimum");
    let parameters = Parameters::new().with(&minimum, 15_i64);
    let pipeline = Pipeline::<Event>::from_model()
        .filter(Event::active.eq(true).and(Event::score.gt(&minimum)))
        .select((Event::id, Event::score))
        .offset(0)
        .limit(1);
    let plan = pipeline.logical_plan().unwrap();

    let mut jsonl_stream = jsonl
        .execute(&pipeline, &parameters, &ExecutionOptions::default())
        .unwrap();
    let jsonl_rows = semantic::collect_checked(&mut jsonl_stream, plan.output()).unwrap();

    let mut memory_stream = memory
        .execute(&pipeline, &parameters, &ExecutionOptions::default())
        .unwrap();
    let memory_rows = semantic::collect_checked(&mut memory_stream, plan.output()).unwrap();

    assert_eq!(jsonl_rows.as_slice(), memory_rows.as_slice());
    assert_eq!(
        jsonl_rows.as_slice(),
        &[ExecutionRow::Value(Datum::Value(Value::Tuple(vec![
            Datum::Value(Value::UInt(2)),
            Datum::Value(Value::Int(30)),
        ])))]
    );
}

#[test]
fn unnest_is_lazy_and_respects_pull_batching_and_cancellation() {
    let temp = TempDir::new("unnest");
    temp.write("events.jsonl", json_fixture());
    let mut engine = JsonlEngine::new(temp.path()).unwrap();
    engine.bind::<Event>("events.jsonl").unwrap();

    let pipeline = Pipeline::<Event>::from_model().select(Event::tags).unnest();
    let options = ExecutionOptions {
        batch_rows: 1,
        ..ExecutionOptions::default()
    };
    let mut stream = engine
        .execute(&pipeline, &Parameters::new(), &options)
        .unwrap();
    assert_eq!(stream.next_batch().unwrap().unwrap().rows().len(), 1);
    assert_eq!(stream.next_batch().unwrap().unwrap().rows().len(), 1);
    assert_eq!(stream.next_batch().unwrap().unwrap().rows().len(), 1);
    assert!(stream.next_batch().unwrap().is_none());
    assert!(stream.next_batch().unwrap().is_none());

    let mut cancelled = engine
        .execute(&pipeline, &Parameters::new(), &options)
        .unwrap();
    cancelled.cancel();
    assert!(cancelled.next_batch().unwrap().is_none());
}

#[test]
fn missing_null_unknown_and_duplicate_keys_are_not_silently_conflated() {
    let temp = TempDir::new("strict-json");
    temp.write(
        "events.jsonl",
        concat!(
            "{\"id\":1,\"active\":true,\"score\":10,\"tags\":[]}\n",
            "{\"id\":2,\"active\":true,\"score\":20,\"tags\":[],\"nickname\":null}\n",
        ),
    );
    let mut engine = JsonlEngine::new(temp.path()).unwrap();
    engine.bind::<Event>("events.jsonl").unwrap();

    let missing = Pipeline::<Event>::from_model()
        .filter(Event::nickname.is_missing())
        .select(Event::id);
    let mut stream = engine
        .execute(&missing, &Parameters::new(), &ExecutionOptions::default())
        .unwrap();
    let rows =
        semantic::collect_checked(&mut stream, missing.logical_plan().unwrap().output()).unwrap();
    assert_eq!(
        rows.as_slice(),
        &[ExecutionRow::Value(Datum::Value(Value::UInt(1)))]
    );

    let duplicate = TempDir::new("duplicate-key");
    duplicate.write(
        "events.jsonl",
        "{\"id\":1,\"id\":2,\"active\":true,\"score\":10,\"tags\":[]}\n",
    );
    let mut duplicate_engine = JsonlEngine::new(duplicate.path()).unwrap();
    duplicate_engine.bind::<Event>("events.jsonl").unwrap();
    let mut duplicate_stream = duplicate_engine
        .execute(
            &Pipeline::<Event>::from_model(),
            &Parameters::new(),
            &ExecutionOptions::default(),
        )
        .unwrap();
    let error = duplicate_stream.next_batch().unwrap_err();
    assert_eq!(error.code(), "JSONL-DECODE-001");

    let unknown = TempDir::new("unknown-key");
    unknown.write(
        "events.jsonl",
        "{\"id\":1,\"active\":true,\"score\":10,\"tags\":[],\"extra\":1}\n",
    );
    let mut unknown_engine = JsonlEngine::new(unknown.path()).unwrap();
    unknown_engine.bind::<Event>("events.jsonl").unwrap();
    let mut unknown_stream = unknown_engine
        .execute(
            &Pipeline::<Event>::from_model(),
            &Parameters::new(),
            &ExecutionOptions::default(),
        )
        .unwrap();
    assert_eq!(
        unknown_stream.next_batch().unwrap_err().code(),
        "JSONL-DECODE-003"
    );
}

#[test]
fn input_limits_and_root_confinement_are_enforced_before_unbounded_work() {
    let temp = TempDir::new("limits");
    temp.write("events.jsonl", json_fixture());
    let limits = JsonlLimits {
        max_line_bytes: 32,
        ..JsonlLimits::default()
    };
    let invalid_depth = JsonlLimits {
        max_nesting_depth: 65,
        ..JsonlLimits::default()
    };
    let error = JsonlEngine::with_limits(temp.path(), invalid_depth).unwrap_err();
    assert_eq!(error.code(), "JSONL-LIMIT-000");

    let mut engine = JsonlEngine::with_limits(temp.path(), limits).unwrap();
    engine.bind::<Event>("events.jsonl").unwrap();
    let mut stream = engine
        .execute(
            &Pipeline::<Event>::from_model(),
            &Parameters::new(),
            &ExecutionOptions::default(),
        )
        .unwrap();
    assert_eq!(stream.next_batch().unwrap_err().code(), "JSONL-LIMIT-002");

    let parent = temp.path().parent().unwrap();
    let outside_name = format!("dol-jsonl-outside-{}.jsonl", temp.unique());
    let outside = parent.join(&outside_name);
    fs::write(&outside, json_fixture()).unwrap();
    let mut confined = JsonlEngine::new(temp.path()).unwrap();
    let error = confined
        .bind::<Event>(Path::new("..").join(&outside_name))
        .unwrap_err();
    assert_eq!(error.code(), "JSONL-SOURCE-002");
    fs::remove_file(outside).unwrap();

    let replaced = TempDir::new("revalidate");
    replaced.write("events.jsonl", json_fixture());
    let mut revalidated = JsonlEngine::new(replaced.path()).unwrap();
    revalidated.bind::<Event>("events.jsonl").unwrap();
    fs::remove_file(replaced.path().join("events.jsonl")).unwrap();
    fs::create_dir(replaced.path().join("events.jsonl")).unwrap();
    let error = revalidated
        .execute(
            &Pipeline::<Event>::from_model(),
            &Parameters::new(),
            &ExecutionOptions::default(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "JSONL-SOURCE-003");
}

#[test]
fn unsupported_global_or_multisource_semantics_fail_at_placement() {
    let temp = TempDir::new("unsupported");
    temp.write("events.jsonl", json_fixture());
    let mut engine = JsonlEngine::new(temp.path()).unwrap();
    engine.bind::<Event>("events.jsonl").unwrap();

    let sorted = Pipeline::<Event>::from_model().order_by(Event::score);
    let error = engine
        .execute(&sorted, &Parameters::new(), &ExecutionOptions::default())
        .unwrap_err();
    assert_eq!(error.code(), "ENGINE-PLACEMENT-001");
}

struct TempDir {
    path: PathBuf,
    unique: u128,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("dol-jsonl-{label}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path, unique }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn unique(&self) -> u128 {
        self.unique
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.path.join(name), contents).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
