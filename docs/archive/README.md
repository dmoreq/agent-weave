# Archive

Historical design specifications and review reports, kept for provenance only.

Nothing here describes the shipping system. The current architecture lives in
[`docs/architecture/`](../architecture):

- [Code index](../architecture/code-index.md)
- [Memory store](../architecture/memory-store.md)

## Contents

| Directory | Contents |
|---|---|
| [`designs/`](designs) | Dated design specifications and amendment records written while the system was built. |
| [`reports/`](reports) | Dated codebase reviews, review findings, experiment plans, disclosure ledgers, and external research notes. |

## Note on removed raw data

Large raw evaluation dumps (multi-megabyte agent transcripts, per-run logs, and
prompt payloads) were removed from `docs/baselines/` during the documentation
restructure. The summaries, protocols, and frozen ground-truth fixtures that
were referenced by production checks remain in
[`docs/baselines/`](../baselines). The deleted raw dumps are recoverable from
Git history at the commit preceding the restructure.
