//! Synchronous, pull-based MongoDB aggregation runtime.

use std::fmt;
use std::time::{Duration, Instant};

use dol_core::diagnostic::{Diagnostic, Result};
use dol_engine::capability::{
    Capabilities, PipelineCapabilities, Support, TransactionCapabilities, WriteCapabilities,
};
use dol_engine::{
    DataStream, ExecutionBatch, ExecutionLimits, ExecutionOptions, ExecutionRequest, ExecutionRow,
    PlanExecutor, analyze_engine_placement,
};
use mongodb::bson::Document;
use mongodb::options::ClientOptions;
use mongodb::sync::{Client, Cursor};

use crate::MongodbEngine;
use crate::codec::RowDecoder;
use crate::compiled::CompiledAggregation;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_DRIVER_BATCH_ROWS: usize = 1_024;

/// Connection policy for an explicitly enabled MongoDB runtime.
#[derive(Clone)]
pub struct MongodbRuntimeConfig {
    uri: String,
    application_name: Option<String>,
    connect_timeout: Option<Duration>,
    server_selection_timeout: Option<Duration>,
}

impl MongodbRuntimeConfig {
    /// Creates a runtime configuration. No connection is opened here.
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            application_name: Some("dol-mongodb".to_owned()),
            connect_timeout: None,
            server_selection_timeout: None,
        }
    }

    /// Overrides the driver application name.
    #[must_use]
    pub fn application_name(mut self, value: impl Into<String>) -> Self {
        self.application_name = Some(value.into());
        self
    }

    /// Caps connection establishment time.
    #[must_use]
    pub const fn connect_timeout(mut self, value: Duration) -> Self {
        self.connect_timeout = Some(value);
        self
    }

    /// Caps server selection time.
    #[must_use]
    pub const fn server_selection_timeout(mut self, value: Duration) -> Self {
        self.server_selection_timeout = Some(value);
        self
    }

    pub(crate) fn uri(&self) -> &str {
        &self.uri
    }
}

impl fmt::Debug for MongodbRuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MongodbRuntimeConfig")
            .field("uri", &"<redacted>")
            .field("application_name", &self.application_name)
            .field("connect_timeout", &self.connect_timeout)
            .field("server_selection_timeout", &self.server_selection_timeout)
            .finish()
    }
}

/// Pull stream backed by one MongoDB aggregation cursor.
pub struct MongodbStream {
    client: Option<Client>,
    cursor: Option<Cursor<Document>>,
    compiled: CompiledAggregation,
    decoder: RowDecoder,
    options: ExecutionOptions,
    started: Instant,
    pending: Option<ExecutionRow>,
    cancelled: bool,
}

impl fmt::Debug for MongodbStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MongodbStream")
            .field("compiled", &self.compiled)
            .field("options", &self.options)
            .field("cancelled", &self.cancelled)
            .finish_non_exhaustive()
    }
}

impl MongodbStream {
    pub(crate) fn open(
        runtime: &MongodbRuntimeConfig,
        compiled: CompiledAggregation,
        options: &ExecutionOptions,
    ) -> Result<Self> {
        let started = Instant::now();
        check_timeout(&options.limits, started)?;
        let mut client_options = ClientOptions::parse(runtime.uri())
            .run()
            .map_err(|error| driver_error("connection-string parsing", error))?;
        client_options
            .app_name
            .clone_from(&runtime.application_name);
        let effective_connect = effective_timeout(
            runtime.connect_timeout,
            options.limits.timeout,
            DEFAULT_CONNECT_TIMEOUT,
        );
        client_options.connect_timeout = Some(effective_connect);
        client_options.server_selection_timeout = Some(effective_timeout(
            runtime.server_selection_timeout,
            options.limits.timeout,
            DEFAULT_CONNECT_TIMEOUT,
        ));
        let client = Client::with_options(client_options)
            .map_err(|error| driver_error("client construction", error))?;
        let collection = client
            .database(compiled.database())
            .collection::<Document>(compiled.collection());

        // Validate the entire source contract before any filter/slice can hide a
        // malformed physical document. The server returns at most one offender.
        let mut validation = collection
            .aggregate(compiled.validation_pipeline().iter().cloned())
            .batch_size(1)
            .allow_disk_use(false);
        if let Some(timeout) = options.limits.timeout {
            validation = validation.max_time(timeout);
        }
        let mut validation_cursor = validation
            .run()
            .map_err(|error| driver_error("collection contract preflight", error))?;
        if let Some(result) = validation_cursor.next() {
            result.map_err(|error| driver_error("collection contract preflight", error))?;
            return Err(Diagnostic::error(
                "MONGODB-DECODE-006",
                "MongoDB collection contains a document that violates its mapped DOL type/presence contract",
            ));
        }
        drop(validation_cursor);
        check_timeout(&options.limits, started)?;

        let driver_batch = driver_batch_rows(options)?;
        let mut aggregate = collection
            .aggregate(compiled.pipeline().iter().cloned())
            .batch_size(driver_batch)
            .allow_disk_use(false);
        if let Some(timeout) = options.limits.timeout {
            aggregate = aggregate.max_time(timeout);
        }
        let cursor = aggregate
            .run()
            .map_err(|error| driver_error("aggregation execution", error))?;
        let decoder = RowDecoder::new(&compiled);
        Ok(Self {
            client: Some(client),
            cursor: Some(cursor),
            compiled,
            decoder,
            options: options.clone(),
            started,
            pending: None,
            cancelled: false,
        })
    }
}

impl DataStream for MongodbStream {
    fn next_batch(&mut self) -> Result<Option<ExecutionBatch>> {
        if self.cancelled {
            return Ok(None);
        }
        check_timeout(&self.options.limits, self.started)?;
        let target = materialized_row_target(&self.options)?;
        let mut rows = Vec::with_capacity(target.min(MAX_DRIVER_BATCH_ROWS));
        let mut bytes = 0_u64;

        if let Some(row) = self.pending.take() {
            bytes = row.logical_bytes();
            enforce_single_row_byte_limit(&self.options.limits, bytes)?;
            rows.push(row);
        }

        while rows.len() < target {
            check_timeout(&self.options.limits, self.started)?;
            let Some(cursor) = self.cursor.as_mut() else {
                break;
            };
            let Some(document) = cursor.next() else {
                self.cursor = None;
                self.client = None;
                break;
            };
            let document =
                document.map_err(|error| driver_error("aggregation cursor pull", error))?;
            let row = self.decoder.decode(&self.compiled, &document)?;
            let row_bytes = row.logical_bytes();
            enforce_single_row_byte_limit(&self.options.limits, row_bytes)?;
            if !rows.is_empty()
                && self
                    .options
                    .limits
                    .max_materialized_bytes
                    .is_some_and(|limit| bytes.saturating_add(row_bytes) > limit)
            {
                self.pending = Some(row);
                break;
            }
            bytes = bytes.saturating_add(row_bytes);
            rows.push(row);
        }

        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ExecutionBatch::new(
                self.compiled.output().clone(),
                rows.into_boxed_slice(),
            )))
        }
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.pending = None;
        self.cursor = None;
        self.client = None;
    }
}

pub(crate) fn runtime_capabilities() -> Capabilities {
    let deferred = |reason: &'static str| Support::unsupported(reason);
    let write = || Support::unsupported("MongoDB Stage I is a bounded read runtime");
    Capabilities {
        pipeline: PipelineCapabilities {
            source: Support::ExactEmulated,
            filter: Support::ExactEmulated,
            project: Support::ExactEmulated,
            slice: Support::ExactNative,
            aggregate: deferred("MongoDB exact aggregation lowering is not implemented"),
            unnest: deferred("MongoDB exact unnest lowering is not implemented"),
            window: deferred("MongoDB exact window lowering is not implemented"),
            sort: deferred("MongoDB exact ordering lowering is not implemented"),
            distinct: deferred("MongoDB exact distinct lowering is not implemented"),
            join: deferred("MongoDB exact join lowering is not implemented"),
            set: deferred("MongoDB exact set lowering is not implemented"),
            exists: deferred("MongoDB exact correlated lowering is not implemented"),
        },
        writes: WriteCapabilities {
            insert: write(),
            insert_many: write(),
            update: write(),
            delete: write(),
        },
        transaction: TransactionCapabilities {
            transactions: write(),
            atomic_writes: write(),
            referential_integrity: write(),
        },
    }
}

impl PlanExecutor for MongodbEngine {
    type Stream = MongodbStream;

    fn execute_plan(&self, request: ExecutionRequest<'_>) -> Result<Self::Stream> {
        let runtime = self.runtime_config().ok_or_else(|| {
            Diagnostic::error(
                "MONGODB-EXEC-001",
                "offline MongoDB engine has no runtime connection configuration",
            )
        })?;
        request.options.limits.validate_plan(request.plan)?;
        let placement = analyze_engine_placement(request.plan, self, request.options.placement)?;
        if !placement.is_fully_engine() {
            return Err(Diagnostic::error(
                "MONGODB-PLACEMENT-001",
                "MongoDB runtime does not execute local residual plan fragments",
            ));
        }
        let compiled = self.compile(request.plan, request.parameters)?;
        Self::Stream::open(runtime, compiled, request.options)
    }
}

fn effective_timeout(
    configured: Option<Duration>,
    execution: Option<Duration>,
    fallback: Duration,
) -> Duration {
    match (configured, execution) {
        (Some(configured), Some(execution)) => configured.min(execution),
        (Some(configured), None) => configured,
        (None, Some(execution)) => execution.min(fallback),
        (None, None) => fallback,
    }
}

fn driver_batch_rows(options: &ExecutionOptions) -> Result<u32> {
    let rows = materialized_row_target(options)?.clamp(1, MAX_DRIVER_BATCH_ROWS);
    u32::try_from(rows).map_err(|_| {
        Diagnostic::error(
            "MONGODB-LIMIT-001",
            "MongoDB driver batch size exceeds its u32 representation",
        )
    })
}

fn materialized_row_target(options: &ExecutionOptions) -> Result<usize> {
    let limit = options
        .limits
        .max_materialized_rows
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX);
    let target = options.batch_rows.min(limit);
    if target == 0 {
        return Err(Diagnostic::error(
            "MONGODB-LIMIT-001",
            "MongoDB execution requires a non-zero batch/materialized-row limit",
        ));
    }
    Ok(target)
}

fn enforce_single_row_byte_limit(limits: &ExecutionLimits, bytes: u64) -> Result<()> {
    if limits
        .max_materialized_bytes
        .is_some_and(|limit| bytes > limit)
    {
        return Err(Diagnostic::error(
            "MONGODB-LIMIT-002",
            "one MongoDB result row exceeds the configured materialized-byte limit",
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
            "MONGODB-LIMIT-003",
            "MongoDB execution exceeded the configured timeout",
        ));
    }
    Ok(())
}

fn driver_error(stage: &str, error: mongodb::error::Error) -> Diagnostic {
    Diagnostic::error(
        "MONGODB-EXEC-002",
        format!("MongoDB {stage} failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use dol_engine::{ExecutionLimits, ExecutionOptions};

    use super::{driver_batch_rows, materialized_row_target};

    #[test]
    fn driver_batch_is_bounded_and_zero_limits_are_rejected() {
        let options = ExecutionOptions {
            batch_rows: usize::MAX,
            ..ExecutionOptions::default()
        };
        assert_eq!(driver_batch_rows(&options).unwrap(), 1_024);

        let zero = ExecutionOptions {
            limits: ExecutionLimits {
                max_materialized_rows: Some(0),
                ..ExecutionLimits::default()
            },
            ..ExecutionOptions::default()
        };
        assert!(materialized_row_target(&zero).is_err());
    }
}
