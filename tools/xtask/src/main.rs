#![forbid(unsafe_code)]

mod perf;

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().collect::<Vec<_>>();
    let command = arguments.get(1).map_or("help", String::as_str);
    let command_arguments = arguments.get(2..).unwrap_or_default();
    let command_arguments = if command_arguments
        .first()
        .is_some_and(|argument| argument == "--")
    {
        &command_arguments[1..]
    } else {
        command_arguments
    };

    match command {
        "check" => check(),
        "fmt" => cargo(["fmt", "--all", "--check"]),
        "lint" => cargo([
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ]),
        "test" => cargo(["test", "--workspace", "--all-features"]),
        "docs" => docs(),
        "api" => api_audit(),
        "adoption" => adoption(),
        "security" => security(),
        "metrics" => metrics().map_err(|error| error.to_string()),
        "conformance" => cargo(["test", "--package", "dol-conformance"]),
        "postgres-up" => postgres_up(),
        "postgres-live" => postgres_live(),
        "postgres-live-external" => postgres_live_external(),
        "postgres-down" => postgres_down(),
        "mongodb-live-external" => mongodb_live_external(),
        "bench" => canonical_bench(command_arguments),
        "bench-explore" => exploratory_bench(command_arguments),
        "bench-check" => bench_check(command_arguments),
        "perf-compare" => perf::compare(command_arguments),
        "perf-a-a-smoke" => perf::aa_smoke(command_arguments),
        "perf-profile" => perf::profile(command_arguments),
        "fuzz-check" => fuzz_check(),
        "fuzz" => fuzz(),
        "ci" => ci(),
        "help" | "-h" | "--help" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`; run `cargo xtask help`")),
    }
}

fn check() -> Result<(), String> {
    cargo(["fmt", "--all", "--check"])?;
    cargo(["check", "--workspace", "--all-targets", "--all-features"])?;
    fuzz_check()?;
    cargo([
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(["test", "--workspace", "--all-features"])?;
    docs()?;
    api_audit()?;
    adoption()?;
    metrics().map_err(|error| error.to_string())
}

fn docs() -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.env("RUSTDOCFLAGS", "-D warnings").args([
        "doc",
        "--workspace",
        "--all-features",
        "--no-deps",
    ]);
    run_prepared_command("cargo", &mut command)
}

fn api_audit() -> Result<(), String> {
    cargo(["check", "--package", "dol", "--no-default-features"])?;
    cargo([
        "check",
        "--package",
        "dol",
        "--no-default-features",
        "--features",
        "derive",
    ])?;
    cargo([
        "check",
        "--package",
        "dol",
        "--no-default-features",
        "--features",
        "engine",
    ])?;
    cargo([
        "check",
        "--package",
        "dol",
        "--no-default-features",
        "--features",
        "migrate",
    ])?;
    cargo([
        "check",
        "--package",
        "dol",
        "--no-default-features",
        "--features",
        "wire",
    ])?;
    cargo([
        "check",
        "--package",
        "dol",
        "--no-default-features",
        "--features",
        "ml",
    ])?;
    cargo(["check", "--package", "dol", "--all-features"])
}

fn adoption() -> Result<(), String> {
    cargo(["check", "--workspace", "--examples", "--all-features"])?;
    cargo(["test", "--package", "dol", "--doc", "--all-features"])
}

fn ci() -> Result<(), String> {
    check()?;
    security()
}

fn security() -> Result<(), String> {
    require_cargo_deny()?;
    cargo_external("deny", ["check"])?;
    cargo_external(
        "deny",
        [
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--config",
            "deny.toml",
            "--locked",
            "check",
        ],
    )
}

fn fuzz() -> Result<(), String> {
    if cfg!(windows) {
        return Err(
            "`cargo-fuzz`/libFuzzer is not supported on native Windows. Run \
             `cargo xtask fuzz` from Linux or macOS; on Windows use WSL2, or rely on \
             the repository's Linux fuzz CI job"
                .into(),
        );
    }

    require_nightly()?;
    require_cargo_fuzz()?;
    cargo_nightly(["fuzz", "run", "model_definition", "--", "-runs=1000"])?;
    cargo_nightly(["fuzz", "run", "type_definition", "--", "-runs=1000"])?;
    cargo_nightly(["fuzz", "run", "expression_semantics", "--", "-runs=1000"])?;
    cargo_nightly(["fuzz", "run", "wire_decode", "--", "-runs=1000"])
}

fn fuzz_check() -> Result<(), String> {
    cargo(["fmt", "--manifest-path", "fuzz/Cargo.toml", "--", "--check"])?;
    cargo([
        "clippy",
        "--manifest-path",
        "fuzz/Cargo.toml",
        "--bins",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])
}

const POSTGRES_TEST_PROJECT: &str = "dol-postgres-test";
const POSTGRES_TEST_DEFAULT_PORT: u16 = 55_432;
const POSTGRES_TEST_CA_CONTAINER: &str = "/var/lib/dol-postgres-test/tls/ca.crt";

fn postgres_up() -> Result<(), String> {
    require_docker_compose()?;
    require_docker_engine()?;
    let port = postgres_test_port()?;
    postgres_up_on_port(port)
}

fn postgres_up_on_port(port: u16) -> Result<(), String> {
    let state_dir = postgres_test_state_dir()?;
    if state_dir.exists() {
        fs::remove_dir_all(&state_dir)
            .map_err(|error| format!("cannot reset PostgreSQL test state directory: {error}"))?;
    }
    fs::create_dir_all(&state_dir)
        .map_err(|error| format!("cannot create PostgreSQL test state directory: {error}"))?;

    let mut up = docker_compose_command()?;
    up.env("DOL_POSTGRES_TEST_PORT", port.to_string())
        .args(["up", "--detach", "--build"]);
    run_prepared_command("docker compose", &mut up)?;
    wait_for_postgres()?;
    verify_postgres_rejects_plaintext()?;

    let mut export_ca = docker_compose_command()?;
    let output = export_ca
        .args(["exec", "-T", "postgres", "cat", POSTGRES_TEST_CA_CONTAINER])
        .output()
        .map_err(|error| format!("failed to export generated PostgreSQL test CA: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot export generated PostgreSQL test CA; `docker compose` exited with {}",
            output.status
        ));
    }
    if output.stdout.is_empty() {
        return Err("generated PostgreSQL test CA is empty".into());
    }

    fs::write(postgres_test_ca_path()?, output.stdout)
        .map_err(|error| format!("cannot persist generated PostgreSQL test CA: {error}"))?;
    fs::write(postgres_test_port_path()?, port.to_string())
        .map_err(|error| format!("cannot persist PostgreSQL test port: {error}"))?;

    println!(
        "PostgreSQL test fixture is ready on localhost:{port}; run `cargo xtask postgres-live`"
    );
    Ok(())
}

fn postgres_live() -> Result<(), String> {
    ensure_managed_postgres_fixture()?;
    let port = read_managed_postgres_port()?;
    let ca = postgres_test_ca_path()?;
    if !ca.is_file() {
        return Err(
            "managed PostgreSQL test CA is missing; run `cargo xtask postgres-up` first".into(),
        );
    }
    let url = format!("postgresql://dol_test:dol-test-only@localhost:{port}/dol?sslmode=require");
    println!("Running live PostgreSQL conformance against managed fixture on localhost:{port}");

    let mut command = postgres_live_command();
    command
        .env("DOL_POSTGRES_TEST_URL", url)
        .env("DOL_POSTGRES_TEST_CA", ca);
    match run_prepared_command("cargo", &mut command) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("PostgreSQL live conformance failed; managed fixture log tail follows:");
            let _ = print_postgres_test_logs();
            Err(error)
        }
    }
}

fn ensure_managed_postgres_fixture() -> Result<(), String> {
    require_docker_compose()?;
    require_docker_engine()?;

    let ca_ready = postgres_test_ca_path()?.is_file();
    let state_port = if ca_ready {
        read_managed_postgres_port().ok()
    } else {
        None
    };
    let running = managed_postgres_is_running()?;

    if state_port.is_some() && running {
        wait_for_postgres()?;
        return Ok(());
    }

    if let Some(port) = state_port {
        println!("Managed PostgreSQL fixture is not running; starting it now");
        return postgres_up_on_port(port);
    }

    println!("Managed PostgreSQL fixture is not provisioned; creating it now");
    postgres_up_on_port(postgres_test_port()?)
}

fn managed_postgres_is_running() -> Result<bool, String> {
    let mut command = docker_compose_command()?;
    let output = command
        .args(["ps", "--status", "running", "--services"])
        .output()
        .map_err(|error| format!("failed to inspect managed PostgreSQL fixture: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "cannot inspect managed PostgreSQL fixture; `docker compose` exited with {}",
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "docker compose returned non-UTF-8 service output".to_string())?;
    Ok(stdout.lines().any(|line| line.trim() == "postgres"))
}

fn postgres_live_external() -> Result<(), String> {
    let url = env::var_os("DOL_POSTGRES_TEST_URL")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "DOL_POSTGRES_TEST_URL must be set for `cargo xtask postgres-live-external`".to_string()
        })?;
    println!(
        "Running live PostgreSQL conformance against explicitly configured external TLS database"
    );

    let mut command = postgres_live_command();
    command.env("DOL_POSTGRES_TEST_URL", url);
    if let Some(ca) = env::var_os("DOL_POSTGRES_TEST_CA").filter(|value| !value.is_empty()) {
        command.env("DOL_POSTGRES_TEST_CA", ca);
    }
    run_prepared_command("cargo", &mut command)
}

fn postgres_live_command() -> Command {
    let mut command = Command::new("cargo");
    command.args([
        "test",
        "--package",
        "dol-postgres",
        "--test",
        "stage_h_live",
        "--",
        "--ignored",
        "--test-threads=1",
    ]);
    command
}

fn mongodb_live_external() -> Result<(), String> {
    let url = env::var_os("DOL_MONGODB_TEST_URL")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "DOL_MONGODB_TEST_URL must be set for `cargo xtask mongodb-live-external`".to_string()
        })?;
    println!("Running live MongoDB conformance against explicitly configured test database");
    let mut command = Command::new("cargo");
    command.env("DOL_MONGODB_TEST_URL", url).args([
        "test",
        "--package",
        "dol-mongodb",
        "--test",
        "stage_i_live",
        "--",
        "--ignored",
        "--test-threads=1",
    ]);
    run_prepared_command("cargo", &mut command)
}

fn print_postgres_test_logs() -> Result<(), String> {
    let mut logs = docker_compose_command()?;
    logs.args(["logs", "--no-color", "--tail", "200", "postgres"]);
    run_prepared_command("docker compose", &mut logs)
}

fn postgres_down() -> Result<(), String> {
    require_docker_compose()?;
    require_docker_engine()?;
    let mut down = docker_compose_command()?;
    down.args(["down", "--volumes", "--remove-orphans"]);
    run_prepared_command("docker compose", &mut down)?;

    let state_dir = postgres_test_state_dir()?;
    if state_dir.exists() {
        fs::remove_dir_all(&state_dir)
            .map_err(|error| format!("cannot remove PostgreSQL test state directory: {error}"))?;
    }
    println!("PostgreSQL test fixture stopped and its data/TLS volumes were removed");
    Ok(())
}

fn require_docker_engine() -> Result<(), String> {
    match Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(
            "the Docker engine is unavailable; start Docker Desktop (or another Docker engine) \
             and wait until its Linux engine is running, then rerun the command"
                .into(),
        ),
    }
}

fn require_docker_compose() -> Result<(), String> {
    match Command::new("docker")
        .args(["compose", "version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(
            "Docker with the Compose v2 plugin is required for the managed PostgreSQL live \
             fixture; install/start Docker, then rerun the command"
                .into(),
        ),
    }
}

fn docker_compose_command() -> Result<Command, String> {
    let compose = workspace_root()
        .map_err(|error| error.to_string())?
        .join("infra")
        .join("postgres-test")
        .join("compose.yaml");
    if !compose.is_file() {
        return Err(format!(
            "PostgreSQL test compose file is missing at {}",
            compose.display()
        ));
    }

    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("--project-name")
        .arg(POSTGRES_TEST_PROJECT)
        .arg("--file")
        .arg(compose);
    Ok(command)
}

fn wait_for_postgres() -> Result<(), String> {
    for _ in 0..60 {
        let mut probe = docker_compose_command()?;
        let status = probe
            .args([
                "exec",
                "-T",
                "postgres",
                "psql",
                "postgresql://dol_test:dol-test-only@localhost:5432/dol?sslmode=require",
                "-Atqc",
                "SELECT 1",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(value) if value.success()) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }

    Err(
        "managed PostgreSQL fixture did not become healthy; inspect it with \
         `docker compose -p dol-postgres-test -f infra/postgres-test/compose.yaml logs postgres`"
            .into(),
    )
}

fn verify_postgres_rejects_plaintext() -> Result<(), String> {
    let mut probe = docker_compose_command()?;
    let status = probe
        .args([
            "exec",
            "-T",
            "postgres",
            "psql",
            "postgresql://dol_test:dol-test-only@localhost:5432/dol?sslmode=disable",
            "-Atqc",
            "SELECT 1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("cannot probe PostgreSQL plaintext rejection: {error}"))?;

    if status.success() {
        return Err(
            "managed PostgreSQL fixture accepted a plaintext host connection; TLS-only HBA \
             enforcement is not active"
                .into(),
        );
    }
    Ok(())
}

fn postgres_test_port() -> Result<u16, String> {
    let Some(value) = env::var_os("DOL_POSTGRES_TEST_PORT") else {
        return Ok(POSTGRES_TEST_DEFAULT_PORT);
    };
    let value = value
        .into_string()
        .map_err(|_| "DOL_POSTGRES_TEST_PORT must be valid UTF-8".to_string())?;
    let port = value
        .parse::<u16>()
        .map_err(|_| "DOL_POSTGRES_TEST_PORT must be a valid TCP port".to_string())?;
    if port == 0 {
        return Err("DOL_POSTGRES_TEST_PORT must be greater than zero".into());
    }
    Ok(port)
}

fn read_managed_postgres_port() -> Result<u16, String> {
    let path = postgres_test_port_path()?;
    let value = fs::read_to_string(&path).map_err(|_| {
        "managed PostgreSQL fixture state is missing; run `cargo xtask postgres-up` first or \
         set `DOL_POSTGRES_TEST_URL` for an external TLS test database"
            .to_string()
    })?;
    value.trim().parse::<u16>().map_err(|_| {
        format!(
            "managed PostgreSQL fixture state at {} contains an invalid port",
            path.display()
        )
    })
}

fn postgres_test_state_dir() -> Result<PathBuf, String> {
    Ok(workspace_root()
        .map_err(|error| error.to_string())?
        .join(".dol")
        .join("postgres-test"))
}

fn postgres_test_ca_path() -> Result<PathBuf, String> {
    Ok(postgres_test_state_dir()?.join("ca.crt"))
}

fn postgres_test_port_path() -> Result<PathBuf, String> {
    Ok(postgres_test_state_dir()?.join("port"))
}

fn require_nightly() -> Result<(), String> {
    match Command::new("cargo")
        .args(["+nightly", "--version"])
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(
            "the nightly Rust toolchain is required for `cargo xtask fuzz`; install it \
             with `rustup toolchain install nightly --profile minimal`, then rerun the command"
                .into(),
        ),
    }
}

fn require_cargo_deny() -> Result<(), String> {
    match Command::new("cargo-deny").arg("--version").status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(
            "`cargo-deny` is required for `cargo xtask security` and `cargo xtask ci`; install it \
             with `cargo install cargo-deny --locked`, then rerun the command"
                .into(),
        ),
    }
}

fn require_cargo_fuzz() -> Result<(), String> {
    match Command::new("cargo-fuzz").arg("--version").status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => Err(
            "`cargo-fuzz` is required for `cargo xtask fuzz`; install it with \
             `cargo install cargo-fuzz --locked`, then rerun the command"
                .into(),
        ),
    }
}

fn cargo_nightly<const N: usize>(args: [&str; N]) -> Result<(), String> {
    let mut all = vec!["+nightly"];
    all.extend(args);
    run_command("cargo", all)
}

fn cargo<const N: usize>(args: [&str; N]) -> Result<(), String> {
    run_command("cargo", args)
}

fn canonical_bench(benchmark_args: &[String]) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args(["bench", "--package", "dol-bench", "--bench", "closure_v1"]);
    if !benchmark_args.is_empty() {
        command.arg("--").args(benchmark_args);
    }
    run_prepared_command("cargo", &mut command)
}

fn exploratory_bench(criterion_args: &[String]) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args(["bench", "--package", "dol-bench", "--bench", "explore"]);
    if !criterion_args.is_empty() {
        command.arg("--").args(criterion_args);
    }
    run_prepared_command("cargo", &mut command)
}

fn bench_check(arguments: &[String]) -> Result<(), String> {
    if !arguments.is_empty() {
        return Err("`cargo xtask bench-check` does not accept arguments".into());
    }
    cargo(["bench", "--workspace", "--no-run"])?;
    perf::overlay_check()
}

fn cargo_external<const N: usize>(subcommand: &str, args: [&str; N]) -> Result<(), String> {
    let mut all = vec![subcommand];
    all.extend(args);
    run_command("cargo", all)
}

fn run_command<I, S>(program: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args);
    run_prepared_command(program, &mut command)
}

fn run_prepared_command(label: &str, command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("failed to start `{label}`: {error}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("`{label}` exited with {status}"))
    }
}

fn metrics() -> io::Result<()> {
    let root = workspace_root()?;
    let mut files = Vec::new();
    collect_rust_files(&root, &mut files)?;

    files.sort();

    let mut total_lines = 0usize;
    let mut large_files = Vec::new();

    for path in &files {
        let content = fs::read_to_string(path)?;
        let lines = content.lines().count();
        total_lines += lines;
        if lines > 800 {
            large_files.push((lines, path.strip_prefix(&root).unwrap_or(path)));
        }
    }

    println!("Rust source files: {}", files.len());
    println!("Rust source lines: {total_lines}");

    if large_files.is_empty() {
        println!("Files above architecture-review threshold (800 LOC): 0");
    } else {
        println!("Files above architecture-review threshold (800 LOC):");
        for (lines, path) in large_files {
            println!("  {lines:>6} {}", path.display());
        }
    }

    Ok(())
}

fn workspace_root() -> io::Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("cannot resolve workspace root"))
}

fn collect_rust_files(path: &Path, output: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let name = entry.file_name();

        if name == OsStr::new("target") || name == OsStr::new(".git") {
            continue;
        }

        if child.is_dir() {
            collect_rust_files(&child, output)?;
        } else if child.extension() == Some(OsStr::new("rs")) {
            output.push(child);
        }
    }

    Ok(())
}

fn help() {
    println!(
        "DOL repository tasks:\n\
         \n  cargo xtask check        fmt + check + clippy + test + docs + metrics\
         \n  cargo xtask fmt          rustfmt verification\
         \n  cargo xtask lint         clippy with warnings denied\
         \n  cargo xtask test         workspace tests\
         \n  cargo xtask docs         workspace rustdoc\
         \n  cargo xtask api          facade feature-matrix API audit\
         \n  cargo xtask adoption     compile adoption examples and doctests\
         \n  cargo xtask security     cargo-deny checks\
         \n  cargo xtask metrics      source-size report\
         \n  cargo xtask conformance  conformance harness tests\
         \n  cargo xtask postgres-up  start the managed TLS PostgreSQL test fixture\
         \n  cargo xtask postgres-live ensure fixture + run managed PostgreSQL conformance\
         \n  cargo xtask postgres-live-external explicitly configured external PostgreSQL conformance\
         \n  cargo xtask postgres-down stop and remove the managed PostgreSQL fixture\
         \n  cargo xtask mongodb-live-external explicitly configured MongoDB conformance\
         \n  cargo xtask bench        run the canonical closure-v1 benchmark\
         \n  cargo xtask bench-explore run Criterion diagnostics; pass its args after `--`\
         \n  cargo xtask bench-check  compile canonical and exploratory benchmark targets\
         \n  cargo xtask perf-compare paired closure-v1 comparison on a qualified Linux runner\
         \n  cargo xtask perf-a-a-smoke cross-platform A/A benchmark-contract smoke\
         \n  cargo xtask perf-profile collect perf/flamegraph/assembly/DHAT/RSS evidence\
         \n  cargo xtask fuzz-check   format + strict-clippy all isolated fuzz targets\
         \n  cargo xtask fuzz         bounded cargo-fuzz smoke campaigns\
         \n  cargo xtask ci           check + security\n"
    );
}
