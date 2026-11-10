//! SQL scaffolding shared by the db submodules: the chunked `IN (...)` bind
//! contract and the fixed-width integer row mappers used by counter queries.
//! No filesystem access lives here.

use rusqlite::Connection;

use crate::error::Result;

/// Batch size for every `IN (...)` bind list. SQLite's variable limit is 999;
/// 500 keeps headroom for the other bound parameters in the same statement.
pub(crate) const SQL_CHUNK: usize = 500;

/// `?,?,...` placeholders for an `IN (...)` list of `count` bound values.
pub(crate) fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

/// Chunked IN-query scaffold: slices `values` into `SQL_CHUNK` batches
/// (SQLite's variable limit) and runs `body` per batch with the connection,
/// the batch's placeholder list, and the batch's value slice. `&Transaction`
/// call sites deref to `Connection`.
pub(crate) fn for_each_chunk<T: rusqlite::ToSql, F>(
    conn: &Connection,
    values: &[T],
    mut body: F,
) -> Result<()>
where
    F: FnMut(&Connection, &str, &[T]) -> Result<()>,
{
    for chunk in values.chunks(SQL_CHUNK) {
        body(conn, &sql_placeholders(chunk.len()), chunk)?;
    }
    Ok(())
}

/// Row mappers for the fixed-width integer tuples used by counter queries.
pub(crate) fn ints3(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
}

pub(crate) fn ints4(r: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, i64, i64, i64)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
}
