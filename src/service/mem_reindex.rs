//! `eg mem reindex`: force a full rebuild of the derived Unicode memory index.
//! Notes are primary data and are never touched; only the v2 FTS table and the
//! per-row `fts_version` markers are rewritten.

use super::*;

pub(super) fn run_mem_reindex(req: &CommandRequest, root_arg: Option<&Path>) -> Result<ArmOut> {
    let CommandRequest::MemReindex { global } = req else {
        unreachable!("run_mem_reindex called with a non-reindex request")
    };
    let conn = open_mem_conn(root_arg, *global, MemoryMode::Mutation)?;
    let rows = crate::db::ensure_memory_fts_v2(&conn, true)?;
    crate::journal::record_result(crate::journal::OutcomeCounts::uniform(1), false, 0);
    Ok(ArmOut::plain(
        format!("reindexed {rows} rows\n"),
        String::new(),
    ))
}
