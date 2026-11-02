# agy / zcode agent-utility battery (2026-11-02)

| host | stage | arm | n | correct | tools | kg calls | feature calls | input tok | cache-read tok | out tok | seconds | violations |
|---|---|---|---|---|---|---|---|---|---|---|---|
| agy | path | kgc | 6 | 6/6 | 32.2 | 21.3 | 0.0 | 104002 | 875165 | 12210 | 215 | 5 |
| agy | path | kgp | 6 | 6/6 | 9.3 | 4.0 | 1.2 | 38897 | 177427 | 3406 | 65 | 0 |
| agy | search | kgc | 2 | 2/2 | 6.5 | 1.0 | 0.0 | 26964 | 135462 | 3360 | 60 | 0 |
| agy | search | kgs | 2 | 2/2 | 4.0 | 1.5 | 1.5 | 14338 | 103350 | 3208 | 54 | 0 |
| agy | clones | kgc | 2 | 2/2 | 11.5 | 0.5 | 0.0 | 46836 | 267634 | 7866 | 99 | 0 |
| agy | clones | kgcl | 2 | 2/2 | 9.0 | 1.0 | 1.0 | 38305 | 155716 | 4060 | 68 | 0 |
| agy | context | kgc | 3 | 3/3 | 16.7 | 7.0 | 0.0 | 68006 | 373083 | 7121 | 109 | 0 |
| agy | context | kgx | 3 | 3/3 | 12.0 | 3.0 | 1.3 | 43809 | 256057 | 5033 | 99 | 0 |
| zcode | path | kgc | 6 | 6/6 | 17.0 | 17.3 | 0.0 | 22934 | 217472 | 5548 | 59 | 0 |
| zcode | path | kgp | 6 | 6/6 | 6.3 | 6.2 | 1.7 | 14676 | 109611 | 2349 | 32 | 0 |
| zcode | search | kgc | 2 | 2/2 | 7.0 | 1.0 | 0.0 | 15307 | 85376 | 992 | 20 | 0 |
| zcode | search | kgs | 2 | 2/2 | 5.0 | 2.0 | 2.0 | 16843 | 85248 | 1180 | 13 | 0 |
| zcode | clones | kgc | 2 | 2/2 | 8.0 | 0.0 | 0.0 | 19455 | 108544 | 4322 | 35 | 0 |
| zcode | clones | kgcl | 2 | 2/2 | 7.5 | 4.5 | 4.5 | 19062 | 87552 | 2592 | 27 | 0 |
| zcode | context | kgc | 3 | 3/3 | 9.0 | 3.0 | 0.0 | 17984 | 110165 | 2350 | 27 | 0 |
| zcode | context | kgx | 3 | 3/3 | 4.7 | 1.0 | 1.0 | 13446 | 89088 | 1535 | 22 | 0 |

## Control vs treatment deltas

| host | stage | metric | control | treatment | delta |
|---|---|---|---|---|---|
| agy | path | tools | 32.2 | 9.3 | -71% |
| agy | path | kg_calls | 21.3 | 4.0 | -81% |
| agy | path | tok_in | 104002.5 | 38897.2 | -63% |
| agy | path | cache_read | 875164.8 | 177426.8 | -80% |
| agy | path | tok_out | 12210.0 | 3405.8 | -72% |
| agy | path | duration_s | 215.5 | 65.4 | -70% |
| agy | search | tools | 6.5 | 4.0 | -38% |
| agy | search | kg_calls | 1.0 | 1.5 | +50% |
| agy | search | tok_in | 26964.0 | 14338.0 | -47% |
| agy | search | cache_read | 135462.5 | 103350.5 | -24% |
| agy | search | tok_out | 3359.5 | 3208.0 | -5% |
| agy | search | duration_s | 60.0 | 53.9 | -10% |
| agy | clones | tools | 11.5 | 9.0 | -22% |
| agy | clones | kg_calls | 0.5 | 1.0 | +100% |
| agy | clones | tok_in | 46836.5 | 38305.0 | -18% |
| agy | clones | cache_read | 267634.0 | 155716.0 | -42% |
| agy | clones | tok_out | 7866.5 | 4060.0 | -48% |
| agy | clones | duration_s | 98.6 | 68.3 | -31% |
| agy | context | tools | 16.7 | 12.0 | -28% |
| agy | context | kg_calls | 7.0 | 3.0 | -57% |
| agy | context | tok_in | 68006.3 | 43808.7 | -36% |
| agy | context | cache_read | 373083.3 | 256057.0 | -31% |
| agy | context | tok_out | 7121.0 | 5032.7 | -29% |
| agy | context | duration_s | 109.1 | 98.8 | -9% |
| zcode | path | tools | 17.0 | 6.3 | -63% |
| zcode | path | kg_calls | 17.3 | 6.2 | -64% |
| zcode | path | tok_in | 22934.3 | 14675.5 | -36% |
| zcode | path | cache_read | 217472.0 | 109610.7 | -50% |
| zcode | path | tok_out | 5547.8 | 2348.7 | -58% |
| zcode | path | duration_s | 59.1 | 31.7 | -46% |
| zcode | search | tools | 7.0 | 5.0 | -29% |
| zcode | search | kg_calls | 1.0 | 2.0 | +100% |
| zcode | search | tok_in | 15307.0 | 16843.0 | +10% |
| zcode | search | cache_read | 85376.0 | 85248.0 | -0% |
| zcode | search | tok_out | 991.5 | 1180.5 | +19% |
| zcode | search | duration_s | 19.6 | 13.4 | -32% |
| zcode | clones | tools | 8.0 | 7.5 | -6% |
| zcode | clones | kg_calls | 0.0 | 4.5 | n/a |
| zcode | clones | tok_in | 19455.0 | 19061.5 | -2% |
| zcode | clones | cache_read | 108544.0 | 87552.0 | -19% |
| zcode | clones | tok_out | 4321.5 | 2591.5 | -40% |
| zcode | clones | duration_s | 34.6 | 26.9 | -22% |
| zcode | context | tools | 9.0 | 4.7 | -48% |
| zcode | context | kg_calls | 3.0 | 1.0 | -67% |
| zcode | context | tok_in | 17984.0 | 13446.3 | -25% |
| zcode | context | cache_read | 110165.3 | 89088.0 | -19% |
| zcode | context | tok_out | 2350.3 | 1535.3 | -35% |
| zcode | context | duration_s | 27.1 | 22.0 | -19% |

## Violations

- `agy-kgc-r1-P1`: tool budget exceeded (27 > 25)
- `agy-kgc-r1-P2`: tool budget exceeded (31 > 25)
- `agy-kgc-r2-P2`: tool budget exceeded (28 > 25)
- `agy-kgc-r2-P3`: tool budget exceeded (46 > 25)
- `agy-kgc-r1-P3`: tool budget exceeded (39 > 25)

## Correctness failures

