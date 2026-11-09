# Public navigation benchmark

This directory holds a small, self-contained regression corpus for `eg`'s
static navigation output. It is designed to run without a model, network
access, or private repository.

## Run

```sh
cargo build --release
python3 -I bench/run.py target/release/eg
python3 -I bench/test_run.py
```

`run.py` copies each corpus root to a temporary directory, builds the index,
runs every query, and checks expected and forbidden pointers. It exits `0`
only when all queries pass.

## Layout

```text
bench/
├── corpus/
│   ├── python/      package, inheritance, typed receiver, test path
│   ├── typescript/  class method, this receiver, typed parameter
│   └── rust/        crate root, module import, impl method
├── queries.jsonl    expected pointers per command
├── run.py           scorer
└── test_run.py      parser/scorer unit tests
```

Each corpus is a separate `eg` root. The query record names its corpus:

```json
{"id":"py-callers-order-total","corpus":"python","command":"callers",
 "args":["shop.models.Order.total"],
 "expect":[{"path":"shop/models.py","detail_contains":"shop.models.DiscountedOrder.discount"}],
 "exit":0}
```

- `expect` entries match the pointer path and, optionally, a substring of the
  detail after the pointer. Line numbers are not compared, so adding a blank
  line does not break the benchmark.
- `forbid` entries fail the query when a matching pointer appears.
- `expect_chain` compares the qualname chain from `eg: chain ...`.
- `exit` is the required exit code. Negative queries use `exit: 1`.

## What this benchmark checks

- The indexed call, import, inheritance, and test relations for a known corpus.
- Pointer rendering for `callers`, `callees`, `context`, `path`, `impact`,
  `imports`, `implementers`, `tests`, and `search`.
- Empty-result exit behavior for queries with no caller.

## What it does not check

- Agent utility, token savings, or tool-call reduction. Those measurements live
  in [the evaluation summary](../docs/evaluation.md) and have separate limits.
- Dynamic dispatch, reflection, or external-library resolution.
- Behavior on large or private repositories.

This is a regression gate for the public corpus, not a replacement for the
private historical evidence described in `docs/evaluation.md`.
