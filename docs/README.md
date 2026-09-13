# DOL Documentation

This index is the canonical entry point for repository documentation. Current
contracts are organized by subject; completed delivery records are kept under
`history/` and must not be read as current repository status.

## Design contracts

- [Architecture](design/architecture.md) explains crate ownership and the
  separation between semantic meaning and physical execution.
- [Data model](design/data-model.md) covers types, models, values, bindings,
  identity, and fingerprints.
- [Normative semantics](design/semantics.md) is the source of truth for portable
  behavior.
- [Expressions](design/expressions.md), [pipelines](design/pipelines.md), and
  [writes and data sets](design/writes-and-datasets.md) describe the public
  language layers.
- [Engine SPI](design/engine-spi.md) defines capabilities, placement, streams,
  limits, caching, and transactions.
- [Migration](design/migration.md), [wire](design/wire.md), and
  [AI/ML](design/ml.md) document the optional systems.
- [Security](design/security.md) collects the repository's trust-boundary and
  resource-safety rules.

## Engine guides

- [Memory](engines/memory.md) is the exact executable reference engine.
- [JSONL](engines/jsonl.md) is the bounded incremental file adapter.
- [PostgreSQL](engines/postgresql.md) and [MongoDB](engines/mongodb.md) expose
  capability-bounded remote read adapters.

## Performance

- [Benchmark contract](performance/benchmarks.md) describes canonical,
  exploratory, and diagnostic benchmark use.
- [Reproducible closure v1](performance/closure-v1.md) defines the frozen
  evidence and verdict policy.
- The machine-readable contract, fixtures, runner manifest, and report template
  live under [`perf/`](../perf/README.md).

## Decisions and history

- [Architecture decision records](adr/README.md) preserve accepted design
  decisions and are not rewritten as general guides evolve.
- [Delivery history](history/delivery-roadmap-a-m.md) records the completed A-M
  roadmap.
- The [2026-08-31 implementation review](history/implementation-review-2026-08-31.md)
  and [2026-09-01 performance milestone](history/performance-milestone-2026-09-01.md)
  are point-in-time evidence, not current status pages.

## Documentation ownership

Keep each normative rule in one current subject guide and link to it elsewhere.
ADRs explain why a durable decision was made. Historical reports preserve what
was observed at a named revision and date; update their links and add context
banners, but do not silently revise their recorded results. Repository workflow
and contribution commands belong in [CONTRIBUTING.md](../CONTRIBUTING.md).
