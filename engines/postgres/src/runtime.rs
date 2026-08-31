//! TLS-required PostgreSQL runtime and bounded synchronous pull stream.

use std::collections::VecDeque;
use std::error::Error as StdError;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};

use dol_core::diagnostic::{Diagnostic, Result};
use dol_core::plan::LogicalPlan;
use dol_engine::{
    Capabilities, DataStream, ExecutionBatch, ExecutionLimits, ExecutionOptions, ExecutionRequest,
    ExecutionRow, PipelineCapabilities, PlanExecutor, Support, TransactionCapabilities,
    WriteCapabilities, analyze_engine_placement,
};
use native_tls::{Certificate, TlsConnector};
use postgres::config::{SslMode, SslNegotiation};
use postgres::error::SqlState;
use postgres_native_tls::{MakeTlsConnector, set_postgresql_alpn};

use crate::PostgresEngine;
use crate::codec::{EncodedBinds, RowDecoder, encode_binds};
use crate::inspect::validate_live_contract;
use crate::sql::CompiledQuery;

/// TLS-required PostgreSQL connection configuration.
#[derive(Clone)]
pub struct PostgresRuntimeConfig {
    config: postgres::Config,
    trusted_ca: Option<Arc<[Certificate]>>,
}

impl PostgresRuntimeConfig {
    /// Parses a libpq-style or PostgreSQL URL connection string.
    ///
    /// Runtime execution rejects `sslmode=disable` and `sslmode=prefer`; callers
    /// must explicitly require authenticated TLS.
    pub fn parse(value: &str) -> Result<Self> {
        let config = value.parse::<postgres::Config>().map_err(|error| {
            Diagnostic::error(
                "POSTGRES-CONNECT-001",
                format!("invalid PostgreSQL connection configuration: {error}"),
            )
        })?;
        if config.get_ssl_mode() != SslMode::Require {
            return Err(Diagnostic::error(
                "POSTGRES-CONNECT-002",
                "PostgreSQL runtime requires sslmode=require; plaintext fallback is not permitted",
            ));
        }
        Ok(Self {
            config,
            trusted_ca: None,
        })
    }

    /// Adds one or more PEM-encoded CA certificates to this runtime TLS verifier.
    ///
    /// This is intended for private/internal PostgreSQL deployments and the repository's
    /// generated Docker test CA. Normal platform roots remain enabled.
    pub fn with_trusted_ca_pem(mut self, pem: impl AsRef<[u8]>) -> Result<Self> {
        let pem = pem.as_ref();
        let certificates = Certificate::stack_from_pem(pem).map_err(|error| {
            Diagnostic::error(
                "POSTGRES-CONNECT-005",
                format!("invalid PostgreSQL trusted CA certificate bundle: {error}"),
            )
        })?;
        if certificates.is_empty() {
            return Err(Diagnostic::error(
                "POSTGRES-CONNECT-005",
                "PostgreSQL trusted CA certificate bundle is empty",
            ));
        }
        self.trusted_ca = Some(Arc::from(certificates));
        Ok(self)
    }

    /// Loads a PEM-encoded CA certificate bundle from a file and trusts it for this runtime.
    pub fn with_trusted_ca_file(self, path: impl AsRef<Path>) -> Result<Self> {
        let pem = std::fs::read(path.as_ref()).map_err(|error| {
            Diagnostic::error(
                "POSTGRES-CONNECT-006",
                format!("cannot read PostgreSQL trusted CA certificate: {error}"),
            )
        })?;
        self.with_trusted_ca_pem(pem)
    }

    pub(crate) fn connect(&self, execution_timeout: Option<Duration>) -> Result<postgres::Client> {
        let tls = tls_connector(
            self.config.get_ssl_negotiation(),
            self.trusted_ca.as_deref(),
        )?;
        let mut config = self.config.clone();
        let connect_timeout =
            effective_connect_timeout(config.get_connect_timeout().copied(), execution_timeout);
        config.connect_timeout(connect_timeout);
        config.connect(tls).map_err(|error| {
            Diagnostic::error(
                "POSTGRES-CONNECT-003",
                format!("PostgreSQL TLS connection failed: {error}"),
            )
        })
    }
}

impl fmt::Debug for PostgresRuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresRuntimeConfig")
            .field("user", &self.config.get_user())
            .field("database", &self.config.get_dbname())
            .field("ssl_mode", &self.config.get_ssl_mode())
            .field("ssl_negotiation", &self.config.get_ssl_negotiation())
            .field("custom_trusted_ca", &self.trusted_ca.is_some())
            .finish_non_exhaustive()
    }
}

/// Pull stream backed by one dedicated PostgreSQL connection worker.
pub struct PostgresStream {
    commands: SyncSender<WorkerCommand>,
    responses: Receiver<Result<Option<ExecutionBatch>>>,
    done: bool,
    cancelled: bool,
}

impl PostgresStream {
    pub(crate) fn open(
        engine: &PostgresEngine,
        runtime: &PostgresRuntimeConfig,
        plan: &LogicalPlan,
        compiled: CompiledQuery,
        options: &ExecutionOptions,
    ) -> Result<Self> {
        options.limits.validate_plan(plan)?;
        if options.batch_rows == 0 {
            return Err(Diagnostic::error(
                "POSTGRES-EXEC-001",
                "PostgreSQL execution batch size must be greater than zero",
            ));
        }

        let (command_tx, command_rx) = sync_channel(1);
        let (response_tx, response_rx) = sync_channel(1);
        let (init_tx, init_rx) = sync_channel(1);
        let runtime = runtime.clone();
        let catalog = engine.catalog().clone();
        let plan = plan.clone();
        let options = options.clone();

        let worker = Worker {
            runtime,
            catalog,
            plan,
            compiled,
            options,
            commands: command_rx,
            responses: response_tx,
            init: init_tx,
        };

        std::thread::Builder::new()
            .name("dol-postgres-stream".into())
            .spawn(move || worker.run())
            .map_err(|error| {
                Diagnostic::error(
                    "POSTGRES-EXEC-002",
                    format!("cannot start PostgreSQL execution worker: {error}"),
                )
            })?;

        init_rx.recv().map_err(|_| worker_stopped())??;
        Ok(Self {
            commands: command_tx,
            responses: response_rx,
            done: false,
            cancelled: false,
        })
    }
}

impl fmt::Debug for PostgresStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresStream")
            .field("done", &self.done)
            .field("cancelled", &self.cancelled)
            .finish_non_exhaustive()
    }
}

impl DataStream for PostgresStream {
    fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
        if self.done || self.cancelled {
            return Ok(None);
        }
        self.commands
            .send(WorkerCommand::Pull)
            .map_err(|_| worker_stopped())?;
        let response = self.responses.recv().map_err(|_| worker_stopped())?;
        match response {
            Ok(batch) => {
                if batch.is_none() {
                    self.done = true;
                }
                Ok(batch)
            }
            Err(error) => {
                self.done = true;
                Err(error)
            }
        }
    }

    fn cancel(&mut self) {
        if self.done || self.cancelled {
            return;
        }
        self.cancelled = true;
        let _ = self.commands.try_send(WorkerCommand::Cancel);
    }
}

impl Drop for PostgresStream {
    fn drop(&mut self) {
        if !self.done && !self.cancelled {
            let _ = self.commands.try_send(WorkerCommand::Cancel);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WorkerCommand {
    Pull,
    Cancel,
}

struct PullState {
    decoder: RowDecoder,
    started: Instant,
    buffered_rows: VecDeque<postgres::Row>,
    pending: Option<ExecutionRow>,
}

impl PullState {
    fn new(compiled: &CompiledQuery, started: Instant) -> Self {
        Self {
            decoder: RowDecoder::new(compiled),
            started,
            buffered_rows: VecDeque::new(),
            pending: None,
        }
    }

    fn prove_empty_or_reject(
        &mut self,
        transaction: &mut postgres::Transaction<'_>,
        portal: &postgres::Portal,
        compiled: &CompiledQuery,
    ) -> Result<Option<ExecutionBatch>> {
        if self.pending.is_some() {
            return Err(zero_row_limit());
        }

        let row = if let Some(row) = self.buffered_rows.pop_front() {
            Some(row)
        } else {
            transaction
                .query_portal(portal, 1)
                .map_err(|error| execution_driver_error_at("portal emptiness probe", error))?
                .into_iter()
                .next()
        };
        let Some(row) = row else {
            return Ok(None);
        };
        let _ = self.decoder.decode(compiled, &row)?;
        Err(zero_row_limit())
    }
}

struct Worker {
    runtime: PostgresRuntimeConfig,
    catalog: crate::PostgresCatalog,
    plan: LogicalPlan,
    compiled: CompiledQuery,
    options: ExecutionOptions,
    commands: Receiver<WorkerCommand>,
    responses: SyncSender<Result<Option<ExecutionBatch>>>,
    init: SyncSender<Result<()>>,
}

impl Worker {
    fn run(self) {
        let started = Instant::now();
        let initialization = self.initialize(started);
        let (mut client, binds) = match initialization {
            Ok(value) => value,
            Err(error) => {
                let _ = self.init.send(Err(error));
                return;
            }
        };

        let mut transaction = match client.build_transaction().read_only(true).start() {
            Ok(transaction) => transaction,
            Err(error) => {
                let _ = self.init.send(Err(execution_driver_error_at(
                    "read-only transaction start",
                    error,
                )));
                return;
            }
        };
        let bind_types = binds.postgres_types();
        let statement = match transaction.prepare_typed(self.compiled.sql(), &bind_types) {
            Ok(statement) => statement,
            Err(error) => {
                let _ = self.init.send(Err(execution_driver_error_at(
                    "typed statement preparation",
                    error,
                )));
                return;
            }
        };
        if statement.params().len() != binds.bind_count() {
            let _ = self.init.send(Err(bind_count_mismatch(
                statement.params().len(),
                binds.bind_count(),
            )));
            return;
        }
        let bind_values = binds.values();
        // Keep only the PostgreSQL portal across DOL pull commands. Each page fetch is
        // completed synchronously so no driver RowIter remains suspended while the worker
        // waits for backpressure from the consumer.
        let portal = match transaction.bind(&statement, &bind_values) {
            Ok(portal) => portal,
            Err(error) => {
                let _ = self.init.send(Err(execution_driver_error_at(
                    "parameterized portal bind",
                    error,
                )));
                return;
            }
        };
        if self.init.send(Ok(())).is_err() {
            return;
        }

        let mut pull_state = PullState::new(&self.compiled, started);
        while let Ok(command) = self.commands.recv() {
            match command {
                WorkerCommand::Cancel => return,
                WorkerCommand::Pull => {
                    let result = self.pull_batch(&mut transaction, &portal, &mut pull_state);
                    let terminal = !matches!(&result, Ok(Some(_)));
                    if self.responses.send(result).is_err() || terminal {
                        return;
                    }
                }
            }
        }
    }

    fn initialize(&self, started: Instant) -> Result<(postgres::Client, EncodedBinds)> {
        let mut client = self.runtime.connect(self.options.limits.timeout)?;
        configure_timeout(&mut client, self.options.limits.timeout)?;
        check_timeout(&self.options.limits, started)?;
        validate_live_contract(&mut client, &self.catalog, &self.plan)?;
        check_timeout(&self.options.limits, started)?;
        let binds = encode_binds(self.compiled.binds())?;
        Ok((client, binds))
    }

    fn pull_batch(
        &self,
        transaction: &mut postgres::Transaction<'_>,
        portal: &postgres::Portal,
        state: &mut PullState,
    ) -> Result<Option<ExecutionBatch>> {
        check_timeout(&self.options.limits, state.started)?;
        let target_rows = materialized_row_target(&self.options);
        if target_rows == 0 {
            return state.prove_empty_or_reject(transaction, portal, &self.compiled);
        }

        let mut batch = Vec::with_capacity(target_rows.min(1_024));
        let mut batch_bytes = 0_u64;
        while batch.len() < target_rows {
            check_timeout(&self.options.limits, state.started)?;
            let decoded = if let Some(row) = state.pending.take() {
                row
            } else {
                if state.buffered_rows.is_empty() {
                    let remaining = target_rows.saturating_sub(batch.len());
                    let page = transaction
                        .query_portal(
                            portal,
                            portal_fetch_rows(
                                remaining,
                                self.options.limits.max_materialized_bytes.is_some(),
                            ),
                        )
                        .map_err(|error| execution_driver_error_at("portal page fetch", error))?;
                    if page.is_empty() {
                        break;
                    }
                    state.buffered_rows.extend(page);
                }
                let Some(row) = state.buffered_rows.pop_front() else {
                    break;
                };
                state.decoder.decode(&self.compiled, &row)?
            };
            let row_bytes = decoded.logical_bytes();
            enforce_single_row_byte_limit(&self.options.limits, row_bytes)?;
            if !batch.is_empty()
                && self
                    .options
                    .limits
                    .max_materialized_bytes
                    .is_some_and(|limit| batch_bytes.saturating_add(row_bytes) > limit)
            {
                state.pending = Some(decoded);
                break;
            }
            batch_bytes = batch_bytes.saturating_add(row_bytes);
            batch.push(decoded);
        }

        if batch.is_empty() {
            return Ok(None);
        }
        Ok(Some(ExecutionBatch::new(
            self.compiled.output().clone(),
            batch.into_boxed_slice(),
        )))
    }
}

fn effective_connect_timeout(
    configured: Option<Duration>,
    execution: Option<Duration>,
) -> Duration {
    const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    match (configured, execution) {
        (Some(configured), Some(execution)) => configured.min(execution),
        (Some(configured), None) => configured,
        (None, Some(execution)) => execution.min(DEFAULT_CONNECT_TIMEOUT),
        (None, None) => DEFAULT_CONNECT_TIMEOUT,
    }
}

fn configure_timeout(client: &mut postgres::Client, timeout: Option<Duration>) -> Result<()> {
    let Some(timeout) = timeout else {
        return Ok(());
    };
    let requested = duration_millis(timeout)?;
    let current = client
        .query_one(
            "SELECT setting::bigint FROM pg_catalog.pg_settings WHERE name = 'statement_timeout'",
            &[],
        )
        .map_err(execution_driver_error)?
        .try_get::<_, i64>(0)
        .map_err(execution_driver_error)?;
    let current = u64::try_from(current).map_err(|_| {
        Diagnostic::error(
            "POSTGRES-EXEC-003",
            "PostgreSQL statement_timeout is outside the supported millisecond range",
        )
    })?;
    let effective = if current == 0 {
        requested
    } else {
        requested.min(current)
    };
    let value = format!("{effective}ms");
    client
        .query_one(
            "SELECT pg_catalog.set_config('statement_timeout', $1, false)",
            &[&value],
        )
        .map_err(execution_driver_error)?;
    Ok(())
}

fn duration_millis(timeout: Duration) -> Result<u64> {
    let millis = timeout.as_millis().max(1);
    u64::try_from(millis).map_err(|_| {
        Diagnostic::error(
            "POSTGRES-EXEC-003",
            "PostgreSQL execution timeout exceeds the supported millisecond range",
        )
    })
}

fn materialized_row_target(options: &ExecutionOptions) -> usize {
    let limit = options
        .limits
        .max_materialized_rows
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(usize::MAX);
    options.batch_rows.min(limit)
}

fn zero_row_limit() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-LIMIT-001",
        "PostgreSQL execution would materialize a row with a zero materialized-row limit",
    )
}

const MAX_PORTAL_FETCH_ROWS: usize = 1_024;

fn portal_fetch_rows(rows: usize, byte_bounded: bool) -> i32 {
    if byte_bounded {
        // The synchronous driver materializes a portal page before DOL can decode
        // and account logical bytes. Fetching one row is the only hard bound when
        // no trustworthy backend row-size estimate exists.
        return 1;
    }
    let rows = rows.clamp(1, MAX_PORTAL_FETCH_ROWS);
    i32::try_from(rows).unwrap_or(1)
}

fn enforce_single_row_byte_limit(limits: &ExecutionLimits, bytes: u64) -> Result<()> {
    if limits
        .max_materialized_bytes
        .is_some_and(|limit| bytes > limit)
    {
        return Err(Diagnostic::error(
            "POSTGRES-LIMIT-002",
            "one PostgreSQL result row exceeds the configured materialized-byte limit",
        ));
    }
    Ok(())
}

fn check_timeout(limits: &ExecutionLimits, started: Instant) -> Result<()> {
    if limits
        .timeout
        .is_some_and(|timeout| started.elapsed() > timeout)
    {
        return Err(Diagnostic::error(
            "POSTGRES-LIMIT-003",
            "PostgreSQL execution exceeded the configured timeout",
        ));
    }
    Ok(())
}

pub(crate) fn runtime_capabilities() -> Capabilities {
    let deferred = |reason: &'static str| Support::unsupported(reason);
    let writes =
        || Support::unsupported("PostgreSQL Stage H read runtime does not advertise writes yet");
    Capabilities {
        pipeline: PipelineCapabilities {
            source: Support::ExactNative,
            filter: Support::ExactEmulated,
            project: Support::ExactEmulated,
            aggregate: deferred(
                "PostgreSQL aggregate lowering is deferred beyond the current Stage H read runtime",
            ),
            unnest: deferred(
                "PostgreSQL unnest lowering is deferred beyond the current Stage H read runtime",
            ),
            window: deferred(
                "PostgreSQL window lowering is deferred beyond the current Stage H read runtime",
            ),
            sort: deferred(
                "PostgreSQL sort lowering is deferred beyond the current Stage H read runtime",
            ),
            distinct: deferred(
                "PostgreSQL distinct lowering is deferred beyond the current Stage H read runtime",
            ),
            slice: Support::ExactNative,
            join: deferred(
                "PostgreSQL join lowering is deferred beyond the current Stage H read runtime",
            ),
            set: deferred(
                "PostgreSQL set lowering is deferred beyond the current Stage H read runtime",
            ),
            exists: deferred(
                "PostgreSQL correlation lowering is deferred beyond the current Stage H read runtime",
            ),
        },
        writes: WriteCapabilities {
            insert: writes(),
            insert_many: writes(),
            update: writes(),
            delete: writes(),
        },
        transaction: TransactionCapabilities {
            transactions: writes(),
            atomic_writes: writes(),
            referential_integrity: writes(),
        },
    }
}

impl PlanExecutor for PostgresEngine {
    type Stream = PostgresStream;

    fn execute_plan(&self, request: ExecutionRequest<'_>) -> Result<Self::Stream> {
        let runtime = self.runtime_config().ok_or_else(|| {
            Diagnostic::error(
                "POSTGRES-EXEC-004",
                "offline PostgreSQL engine has no runtime connection configuration",
            )
        })?;
        request.options.limits.validate_plan(request.plan)?;
        let placement = analyze_engine_placement(request.plan, self, request.options.placement)?;
        if !placement.is_fully_engine() {
            return Err(Diagnostic::error(
                "POSTGRES-PLACEMENT-001",
                "PostgreSQL runtime does not execute hybrid local-residual plans",
            ));
        }
        let compiled = self.compile(request.plan, request.parameters)?;
        PostgresStream::open(self, runtime, request.plan, compiled, request.options)
    }
}

fn tls_connector(
    negotiation: SslNegotiation,
    trusted_ca: Option<&[Certificate]>,
) -> Result<MakeTlsConnector> {
    let mut builder = TlsConnector::builder();
    if let Some(certificates) = trusted_ca {
        for certificate in certificates {
            builder.add_root_certificate(certificate.clone());
        }
    }
    if negotiation == SslNegotiation::Direct {
        set_postgresql_alpn(&mut builder);
    }
    let connector = builder.build().map_err(|error| {
        Diagnostic::error(
            "POSTGRES-CONNECT-004",
            format!("cannot construct the platform TLS verifier: {error}"),
        )
    })?;
    Ok(MakeTlsConnector::new(connector))
}

fn execution_driver_error(error: postgres::Error) -> Diagnostic {
    execution_driver_error_at("execution", error)
}

fn execution_driver_error_at(stage: &str, error: postgres::Error) -> Diagnostic {
    let detail = postgres_error_detail(&error);
    if error.code() == Some(&SqlState::QUERY_CANCELED) {
        return Diagnostic::error(
            "POSTGRES-LIMIT-003",
            format!(
                "PostgreSQL {stage} was canceled by the server timeout/cancellation policy: {detail}"
            ),
        );
    }
    Diagnostic::error(
        "POSTGRES-EXEC-005",
        format!("PostgreSQL {stage} failed: {detail}"),
    )
}

fn postgres_error_detail(error: &postgres::Error) -> String {
    let mut detail = error.to_string();
    let mut source = StdError::source(error);
    for _ in 0..8 {
        let Some(cause) = source else {
            break;
        };
        let message = cause.to_string();
        if !message.is_empty() && !detail.ends_with(&message) {
            detail.push_str(": ");
            detail.push_str(&message);
        }
        source = cause.source();
    }
    detail
}

fn bind_count_mismatch(expected: usize, actual: usize) -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-BIND-003",
        format!(
            "compiled PostgreSQL bind count differs from the prepared statement contract: expected {expected}, got {actual}",
        ),
    )
}

fn worker_stopped() -> Diagnostic {
    Diagnostic::error(
        "POSTGRES-EXEC-006",
        "PostgreSQL execution worker stopped before completing the pull request",
    )
}

#[cfg(test)]
mod tests {
    use dol_engine::{ExecutionLimits, ExecutionOptions};

    use super::{
        MAX_PORTAL_FETCH_ROWS, enforce_single_row_byte_limit, materialized_row_target,
        portal_fetch_rows,
    };

    #[test]
    fn portal_fetch_rows_never_requests_unbounded_execution() {
        assert_eq!(portal_fetch_rows(0, false), 1);
        assert_eq!(portal_fetch_rows(7, false), 7);
        assert_eq!(portal_fetch_rows(MAX_PORTAL_FETCH_ROWS, false), 1_024);
        assert_eq!(portal_fetch_rows(MAX_PORTAL_FETCH_ROWS + 1, false), 1_024);
        assert_eq!(portal_fetch_rows(usize::MAX, false), 1_024);
        assert_eq!(portal_fetch_rows(usize::MAX, true), 1);
    }

    #[test]
    fn materialized_row_target_respects_batch_and_stage_limits() {
        let unbounded = ExecutionOptions {
            limits: ExecutionLimits {
                max_materialized_rows: None,
                ..ExecutionLimits::default()
            },
            batch_rows: 7,
            ..ExecutionOptions::default()
        };
        assert_eq!(materialized_row_target(&unbounded), 7);

        let bounded = ExecutionOptions {
            limits: ExecutionLimits {
                max_materialized_rows: Some(2),
                ..ExecutionLimits::default()
            },
            batch_rows: 7,
            ..ExecutionOptions::default()
        };
        assert_eq!(materialized_row_target(&bounded), 2);

        let zero = ExecutionOptions {
            limits: ExecutionLimits {
                max_materialized_rows: Some(0),
                ..ExecutionLimits::default()
            },
            ..ExecutionOptions::default()
        };
        assert_eq!(materialized_row_target(&zero), 0);
    }

    #[test]
    fn single_row_byte_limit_accepts_boundary_and_rejects_overflow() {
        let limits = ExecutionLimits {
            max_materialized_bytes: Some(8),
            ..ExecutionLimits::default()
        };
        assert!(enforce_single_row_byte_limit(&limits, 8).is_ok());

        let error = enforce_single_row_byte_limit(&limits, 9).unwrap_err();
        assert_eq!(error.code(), "POSTGRES-LIMIT-002");
    }
}
