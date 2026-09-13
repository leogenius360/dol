# closure-v1 footprint contract

Rust 1.98.0 on x86-64 reports these reviewed layouts for candidate `cfbc97f`:

| Type | Visibility | Size | Alignment | Stability meaning |
| --- | --- | ---: | ---: | --- |
| `Expr<Truth>` | public handle | 24 bytes | platform-defined | Hard closure-v1 contract |
| `Pipeline<T>` | public handle | 8 bytes | platform-defined | Hard closure-v1 contract |
| `ExprNode` | private representation | 240 bytes | 16 bytes | Review gate, not public API |
| `PipelineNode` | private representation | 280 bytes | 8 bytes | Review gate, not public API |

The private pins deliberately force a reviewed update rather than declaring the
representation immutable. Any change must update this file with compiler output
and dedicated benchmark evidence explaining its heap and throughput impact.

No relative heap cap is imposed against `772f73a`: the candidate intentionally
adds bounded caches. Closure instead requires measurements for heap before
construction, after construction, after cache fill, and after owner drop, plus
peak RSS, retained heap, allocation blocks/bytes, bytes per node/stage, and
normalized large-scale growth. Large workloads may be at most 25% above the
small-workload normalized cost, and owner drop must release owned cache state.

These compiler-measured numbers are layout observations only. They are not heap
measurements and do not satisfy the memory closure criteria by themselves.
