# Baseline manifest — aw 0.4.0 experiments

- Date (UTC): 2026-11-05
- Commit: 365863a130798621db39c146593f1d77d67783e5
- Branch: feat/graft-learning
- aw: aw 0.4.0
- Release binary sha256: f84ff77fbb18c7b5dd14a7b7b82244d6498a19316404b974b56f44c9852ac507
- Post-fix binary sha256: b8c661329c90b704bb4745bfa1c067db064989c8406a84fddb28d6f336e79e84 (E1.2 rerun after the `callees` near-miss fix; E1.1 used the baseline binary)
- rustc: rustc 1.94.1 (e408947bf 2026-03-25)
- Host: Darwin arm64
- Schema: pub const SCHEMA_VERSION: i32 = 8;

## Index at measurement
```
aw: built index: files=85 symbols=1010 edges=3323 parse_errors=0 unresolved=6474 carried=65 wall=0.27s (walk=0.04s parse=0.10s extract=0.07s db=0.13s)
aw stats: files=85 generated=0 symbols=1010 edges=3323 (calls=2294 imports=78 inherits=21) parse_errors=0 unresolved=6474 db_bytes=2801664 aw=0.4.0 grammar=py-0.25.0+ts-0.23.2+js-0.25.0+rs-0.23.3
```

## New public benchmark in this commit
- `bench/run.py` + `bench/queries.jsonl` (navigation regression, no agent utility).
