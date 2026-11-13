//! Reverse-dependency impact queries and their traversal bookkeeping.

mod deep;
mod direct;

use super::*;
use deep::impact_deep;
use direct::{impact_direct_grouped, impact_direct_sites};

/// `--edges` accepted kinds plus `all`. `type_ref` rows are storage-only
/// (no surface reads them), so the CLI keeps rejecting that spelling.
const EDGE_CLI_KINDS: [EdgeKind; 3] = [EdgeKind::Calls, EdgeKind::Imports, EdgeKind::Inherits];

/// Validate a `--edges` spec into its kinds (sorted, deduplicated). Called at
/// the service boundary so `CommandRequest::Impact` carries validated kinds.
pub(crate) fn parse_edge_kinds(spec: &str) -> Result<Vec<EdgeKind>> {
    let mut out: Vec<EdgeKind> = Vec::new();
    let mut saw_all = false;
    for raw in spec.split(',') {
        let part = raw.trim();
        if part == "all" {
            saw_all = true;
            out.extend(EDGE_CLI_KINDS);
            continue;
        }
        match EdgeKind::parse(part) {
            Some(kind) if EDGE_CLI_KINDS.contains(&kind) => out.push(kind),
            _ => {
                let mut names: Vec<&str> =
                    EDGE_CLI_KINDS.iter().map(|kind| kind.as_str()).collect();
                names.push("all");
                let accepted = names.join(",");
                return Err(EgError::Usage(match part {
                    "" => format!("--edges requires {accepted}"),
                    other => format!("unknown --edges value '{other}' ({accepted})"),
                }));
            }
        }
    }
    if saw_all && spec.split(',').any(|p| p.trim() != "all") {
        return Err(EgError::Usage(
            "--edges all cannot be combined with explicit kinds".to_string(),
        ));
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Source-symbol join `edges e -> symbols s`, for statements that read the
/// source symbol without its file. Same alias set as
/// [`crate::db::EDGE_SRC_FILE_JOIN`], the one home for the source-side join
/// vocabulary. Lives here because impact is its only consumer.
pub(crate) const EDGE_SRC_JOIN: &str = "JOIN symbols s ON s.id = e.src_id";

/// Destination-side joins shared by every impact SQL statement that filters
/// on the destination file: edge -> destination symbol -> its file.
const IMPACT_DST_JOINS: &str = "JOIN symbols ds ON ds.id = e.dst_id
         JOIN files df ON df.id = ds.file_id";

fn impact_empty_detail(kinds: &[EdgeKind]) -> String {
    let has_imports = kinds.contains(&EdgeKind::Imports);
    let has_inherits = kinds.contains(&EdgeKind::Inherits);
    match (has_imports, has_inherits) {
        (true, true) => "0 impacting files found (unresolved imports/bases are omitted)",
        (true, false) => "0 impacting files found (unresolved imports are omitted)",
        (false, true) => "0 impacting files found (unresolved bases are omitted)",
        (false, false) => "0 impacting files found",
    }
    .to_string()
}

fn impact_no_results(ctx: &ImpactCtx<'_>) -> EgError {
    EgError::NoResultsCaveated {
        caveat: ctx.caveat,
        detail: impact_empty_detail(ctx.kinds),
    }
}

/// Impact query options: call-site vs grouped, traversal depth, and the validated
/// edge kinds.
pub struct ImpactOpts {
    pub sites: bool,
    pub depth: usize,
    pub kinds: Vec<EdgeKind>,
}

pub fn impact(
    qctx: &QueryCtx<'_>,
    file: &str,
    limit: usize,
    opts: ImpactOpts,
) -> Result<QueryOutput> {
    let ImpactOpts {
        sites,
        depth,
        kinds,
    } = opts;
    let limit = clamp_limit(limit)?;
    let depth = depth.clamp(1, crate::query::MAX_DEPTH);
    let kind_sql = db::edge_kind_sql(&kinds);
    let rel = indexed_file(qctx.conn, file)?;
    let extended = depth > 1 || kinds.iter().any(|k| *k != EdgeKind::Calls);
    let caveat = if extended {
        if kinds.iter().any(|k| *k != EdgeKind::Calls) {
            IMPACT_EDGES_CAVEAT
        } else {
            IMPACT_CALLS_CAVEAT
        }
    } else {
        ""
    };
    if depth > 1 && sites {
        return Err(EgError::Usage(
            "--sites is only valid with --depth 1".to_string(),
        ));
    }

    let ctx = ImpactCtx {
        rel: &rel,
        limit,
        depth,
        kind_sql: &kind_sql,
        caveat,
        kinds: &kinds,
        extended,
    };
    let mut out = if depth == 1 {
        if !sites {
            impact_direct_grouped(qctx, &ctx)?
        } else {
            impact_direct_sites(qctx, &ctx)?
        }
    } else {
        impact_deep(qctx, &ctx)?
    };

    let guards = crate::memory::find_guarding_notes(qctx.root, &[&rel], &[]);
    for g in guards.iter().filter(|g| g.kind == "decision").take(1) {
        prepend_header(
            &mut out,
            &format!(
                "warning: guarded by decision '{}': {}",
                sanitize(&g.key),
                sanitize(&g.summary)
            ),
        );
    }
    Ok(out)
}

/// Shared, immutable arguments for the three impact phases. The phases stay
/// contiguous; this is not a generic query framework.
struct ImpactCtx<'a> {
    rel: &'a str,
    limit: usize,
    depth: usize,
    kind_sql: &'a str,
    caveat: &'static str,
    kinds: &'a [EdgeKind],
    extended: bool,
}

fn impact_with_caveat(mut out: QueryOutput, caveat: &str, extended: bool) -> QueryOutput {
    if extended {
        prepend_header(&mut out, caveat);
    }
    out
}
