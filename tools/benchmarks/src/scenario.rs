//! Stable, machine-oriented scenario registry for `closure-v1`.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// Validated stable scenario identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ScenarioId(String);

impl ScenarioId {
    /// Validates and owns a scenario identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_scenario_id(&value)?;
        Ok(Self(value))
    }

    /// Borrows the stable machine identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScenarioId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ScenarioId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A lifecycle dimension is distinct from the scenario identifier.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    /// Includes construction and initially empty caches.
    Cold,
    /// Reuses a previously initialized value or cache.
    Warm,
    /// Reuses prepared/bound semantic state.
    Prepared,
    /// Creates a fresh planning/compiler context per operation.
    Fresh,
    /// Reuses a shared logical or physical plan.
    SharedPlan,
    /// No cold/warm distinction applies to the steady-state operation.
    SteadyState,
}

impl Lifecycle {
    /// Stable serialized spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Prepared => "prepared",
            Self::Fresh => "fresh",
            Self::SharedPlan => "shared-plan",
            Self::SteadyState => "steady-state",
        }
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Unit represented by one operation's `work_units` value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkUnitKind {
    /// Whole benchmark operations.
    Operation,
    /// Evaluated expressions.
    Evaluation,
    /// Expression/tree nodes.
    Node,
    /// Pipeline stages.
    Stage,
    /// Model fields.
    Field,
    /// Encoded bytes.
    Byte,
    /// Data rows.
    Row,
    /// Vector candidates.
    Candidate,
}

impl WorkUnitKind {
    /// Stable serialized spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operation => "operation",
            Self::Evaluation => "evaluation",
            Self::Node => "node",
            Self::Stage => "stage",
            Self::Field => "field",
            Self::Byte => "byte",
            Self::Row => "row",
            Self::Candidate => "candidate",
        }
    }
}

/// Whether a workload is a primary closure signal or a scaling diagnostic.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioTier {
    /// Required primary closure workload.
    Canonical,
    /// Required workload-shape/scaling diagnostic.
    Scaling,
}

/// Frozen scenario metadata. Display names are intentionally not identifiers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScenarioDefinition {
    /// Stable machine identifier.
    pub id: &'static str,
    /// Lifecycle boundary measured by this entry.
    pub lifecycle: Lifecycle,
    /// Human-facing label; may improve without changing the workload ABI.
    pub display_name: &'static str,
    /// Work unit reported for normalized throughput.
    pub work_unit_kind: WorkUnitKind,
    /// Primary or scaling workload.
    pub tier: ScenarioTier,
}

impl ScenarioDefinition {
    /// Produces an owned, validated ID for use in observations.
    pub fn scenario_id(self) -> ScenarioId {
        ScenarioId::new(self.id).expect("the static closure-v1 registry is validated by tests")
    }
}

macro_rules! scenario {
    ($id:literal, $lifecycle:ident, $name:literal, $unit:ident, $tier:ident) => {
        ScenarioDefinition {
            id: $id,
            lifecycle: Lifecycle::$lifecycle,
            display_name: $name,
            work_unit_kind: WorkUnitKind::$unit,
            tier: ScenarioTier::$tier,
        }
    };
}

/// Entire required `closure-v1` registry in canonical execution order.
pub const CLOSURE_V1_SCENARIOS: &[ScenarioDefinition] = &[
    scenario!(
        "expr.construct.basic",
        Cold,
        "Expression construction",
        Operation,
        Canonical
    ),
    scenario!(
        "expr.prepare.basic",
        Cold,
        "Expression preparation",
        Operation,
        Canonical
    ),
    scenario!(
        "expr.eval.prepared",
        Prepared,
        "Prepared expression evaluation",
        Evaluation,
        Canonical
    ),
    scenario!(
        "expr.identity",
        Cold,
        "Cold expression identity",
        Operation,
        Canonical
    ),
    scenario!(
        "expr.identity",
        Warm,
        "Warm expression identity",
        Operation,
        Canonical
    ),
    scenario!(
        "pipeline.lower",
        Cold,
        "Cold pipeline lowering",
        Operation,
        Canonical
    ),
    scenario!(
        "pipeline.lower",
        Warm,
        "Warm pipeline lowering",
        Operation,
        Canonical
    ),
    scenario!(
        "pipeline.identity",
        Cold,
        "Cold pipeline identity",
        Operation,
        Canonical
    ),
    scenario!(
        "pipeline.identity",
        Warm,
        "Warm pipeline identity",
        Operation,
        Canonical
    ),
    scenario!(
        "postgres.explain",
        Fresh,
        "Fresh PostgreSQL explain",
        Operation,
        Canonical
    ),
    scenario!(
        "postgres.explain",
        SharedPlan,
        "Shared-plan PostgreSQL explain",
        Operation,
        Canonical
    ),
    scenario!(
        "postgres.place",
        Prepared,
        "Prepared PostgreSQL placement",
        Operation,
        Canonical
    ),
    scenario!(
        "postgres.compile.sql",
        Prepared,
        "PostgreSQL SQL compilation",
        Operation,
        Canonical
    ),
    scenario!(
        "wire.decode.v1.84b",
        Prepared,
        "Pinned 84-byte wire-v1 decode",
        Byte,
        Canonical
    ),
    scenario!(
        "model.define.runtime",
        Cold,
        "Runtime model definition",
        Field,
        Canonical
    ),
    scenario!(
        "mongodb.compile",
        Prepared,
        "MongoDB compilation",
        Operation,
        Canonical
    ),
    scenario!(
        "migration.plan",
        Fresh,
        "Migration diff and plan",
        Operation,
        Canonical
    ),
    scenario!(
        "vector.search.exact",
        Prepared,
        "Exact vector search",
        Candidate,
        Canonical
    ),
    scenario!(
        "memory.execute",
        Prepared,
        "Memory pipeline execution",
        Row,
        Canonical
    ),
    scenario!(
        "expr.eval.depth.8",
        Prepared,
        "Expression evaluation at depth 8",
        Node,
        Scaling
    ),
    scenario!(
        "expr.eval.depth.64",
        Prepared,
        "Expression evaluation at depth 64",
        Node,
        Scaling
    ),
    scenario!(
        "expr.eval.depth.256",
        Prepared,
        "Expression evaluation at depth 256",
        Node,
        Scaling
    ),
    scenario!(
        "pipeline.lower.stages.4",
        Cold,
        "Pipeline lowering with 4 stages",
        Stage,
        Scaling
    ),
    scenario!(
        "pipeline.lower.stages.32",
        Cold,
        "Pipeline lowering with 32 stages",
        Stage,
        Scaling
    ),
    scenario!(
        "pipeline.lower.stages.256",
        Cold,
        "Pipeline lowering with 256 stages",
        Stage,
        Scaling
    ),
    scenario!(
        "model.define.width.4",
        Cold,
        "Runtime model with 4 fields",
        Field,
        Scaling
    ),
    scenario!(
        "model.define.width.64",
        Cold,
        "Runtime model with 64 fields",
        Field,
        Scaling
    ),
    scenario!(
        "model.define.width.1024",
        Cold,
        "Runtime model with 1024 fields",
        Field,
        Scaling
    ),
    scenario!(
        "wire.decode.typed.1k",
        Prepared,
        "Typed wire decode near 1 KiB",
        Byte,
        Scaling
    ),
    scenario!(
        "wire.decode.typed.16k",
        Prepared,
        "Typed wire decode near 16 KiB",
        Byte,
        Scaling
    ),
    scenario!(
        "memory.execute.rows.1",
        Prepared,
        "Memory execution with 1 row",
        Row,
        Scaling
    ),
    scenario!(
        "memory.execute.rows.16",
        Prepared,
        "Memory execution with 16 rows",
        Row,
        Scaling
    ),
    scenario!(
        "memory.execute.rows.256",
        Prepared,
        "Memory execution with 256 rows",
        Row,
        Scaling
    ),
    scenario!(
        "memory.execute.rows.4096",
        Prepared,
        "Memory execution with 4096 rows",
        Row,
        Scaling
    ),
    scenario!(
        "memory.execute.rows.100000",
        Prepared,
        "Memory execution with 100000 rows",
        Row,
        Scaling
    ),
];

/// Finds one exact compound scenario key.
#[must_use]
pub fn find_scenario(id: &str, lifecycle: Lifecycle) -> Option<&'static ScenarioDefinition> {
    CLOSURE_V1_SCENARIOS
        .iter()
        .find(|scenario| scenario.id == id && scenario.lifecycle == lifecycle)
}

/// Finds and copies one exact compound scenario key for workload registration.
#[must_use]
pub fn scenario_definition(id: &'static str, lifecycle: Lifecycle) -> Option<ScenarioDefinition> {
    find_scenario(id, lifecycle).copied()
}

/// Validates a registry and returns a diagnostic suitable for contract checks.
pub fn validate_registry(registry: &[ScenarioDefinition]) -> Result<(), String> {
    if registry.is_empty() {
        return Err("scenario registry must not be empty".into());
    }
    let mut compound_keys = BTreeSet::new();
    for scenario in registry {
        validate_scenario_id(scenario.id)?;
        if scenario.display_name.trim().is_empty() {
            return Err(format!(
                "scenario `{}` has an empty display name",
                scenario.id
            ));
        }
        if !compound_keys.insert((scenario.id, scenario.lifecycle)) {
            return Err(format!(
                "duplicate scenario key `{}` / `{}`",
                scenario.id, scenario.lifecycle
            ));
        }
    }
    Ok(())
}

fn validate_scenario_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("scenario ID must not be empty".into());
    }
    if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
        return Err(format!("invalid scenario ID `{value}`: empty path segment"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
    }) {
        return Err(format!(
            "invalid scenario ID `{value}`: use lowercase ASCII letters, digits, dots, and hyphens"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_registry_has_valid_unique_compound_keys() {
        validate_registry(CLOSURE_V1_SCENARIOS).unwrap();
        assert!(find_scenario("expr.identity", Lifecycle::Cold).is_some());
        assert!(find_scenario("expr.identity", Lifecycle::Warm).is_some());
        assert!(find_scenario("wire.decode.v1.84b", Lifecycle::Prepared).is_some());
        assert!(find_scenario("memory.execute.rows.100000", Lifecycle::Prepared).is_some());
    }

    #[test]
    fn scenario_id_deserialization_enforces_machine_format() {
        assert!(serde_json::from_str::<ScenarioId>("\"expr.eval.depth.64\"").is_ok());
        assert!(serde_json::from_str::<ScenarioId>("\"Expression Eval\"").is_err());
        assert!(ScenarioId::new("pipeline..lower").is_err());
    }

    #[test]
    fn lifecycle_is_not_smuggled_into_shared_identity_ids() {
        let identities = CLOSURE_V1_SCENARIOS
            .iter()
            .filter(|scenario| scenario.id == "expr.identity")
            .collect::<Vec<_>>();
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].lifecycle, Lifecycle::Cold);
        assert_eq!(identities[1].lifecycle, Lifecycle::Warm);
    }
}
