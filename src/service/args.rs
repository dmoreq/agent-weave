//! MCP argument parsing: typed `ArgReader` bounds checks and per-family
//! conversion from tool JSON args into `CommandRequest`s.

use super::*;

/// The `ParamSpec` declaring argument `name` of tool `tool`. Every Int/Float
/// read goes through its spec so the declared bounds execute at the MCP
/// boundary instead of living only in the JSON Schema renderer; a missing
/// entry is an internal error (an argument wired into `from_tool_args`
/// without a TOOL_SPECS declaration).
fn param_spec(tool: &str, name: &str) -> Result<&'static ParamSpec> {
    TOOL_SPECS
        .iter()
        .find(|t| t.name == tool)
        .and_then(|t| t.params.iter().find(|p| p.name == name))
        .ok_or_else(|| {
            EgError::Internal(format!(
                "argument '{name}' of tool '{tool}' has no ParamSpec declaration"
            ))
        })
}

/// Build a `CommandRequest` from MCP tool arguments (untrusted JSON: every
/// field is type-checked here; anything else is a usage error).
/// Typed reader over untrusted MCP tool arguments.
struct ArgReader<'a>(&'a serde_json::Value);

impl ArgReader<'_> {
    fn s(&self, key: &str) -> Option<String> {
        self.0.get(key).and_then(|v| v.as_str()).map(str::to_string)
    }

    fn b(&self, key: &str) -> bool {
        self.0.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    /// Strict String reader for new params: absent/null is `None`, a string is
    /// accepted, and any other JSON type is a usage error instead of a silent
    /// filter drop.
    fn s_opt_spec(&self, tool: &str, name: &str) -> Result<Option<String>> {
        if !matches!(param_spec(tool, name)?.kind, ParamKind::Str) {
            return Err(EgError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Str"
            )));
        }
        match self.0.get(name) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(EgError::Usage(format!("{tool}: '{name}' must be a string"))),
        }
    }

    /// Strict Bool reader for new params: absent/null is `false`, a bool is
    /// accepted, and any other JSON type is a usage error.
    fn b_spec(&self, tool: &str, name: &str) -> Result<bool> {
        if !matches!(param_spec(tool, name)?.kind, ParamKind::Bool) {
            return Err(EgError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Bool"
            )));
        }
        match self.0.get(name) {
            None | Some(serde_json::Value::Null) => Ok(false),
            Some(serde_json::Value::Bool(b)) => Ok(*b),
            Some(_) => Err(EgError::Usage(format!(
                "{tool}: '{name}' must be a boolean"
            ))),
        }
    }

    fn req(&self, key: &str) -> Result<String> {
        self.s(key)
            .ok_or_else(|| EgError::Usage(format!("missing required argument '{key}'")))
    }

    /// Read an Int argument per its TOOL_SPECS declaration (absent or null
    /// falls back to `default`). A non-integer is a usage error (the module's
    /// untrusted-JSON type contract), and a value above the declared max is
    /// clamped to it — the same clamp the design's cap contract assigns to
    /// every declared max ("N > 10,000 clamped to 10,000"; depth "clamp
    /// 1-10"). Values below min pass through untouched: the documented
    /// below-min policies differ per parameter (`limit` rejects < 1 with
    /// exit 2, depth clamps), so the min stays with the consumer that owns
    /// that contract. A param added to TOOL_SPECS cannot ship without its
    /// max executing here.
    fn n_spec(&self, tool: &str, name: &str, default: usize) -> Result<usize> {
        Ok(self.n_opt_spec(tool, name)?.unwrap_or(default))
    }

    /// The Option form for Int arguments that are genuinely optional inputs
    /// (`ttl`, `mem_export` limit): `None` only when the key is absent/null.
    fn n_opt_spec(&self, tool: &str, name: &str) -> Result<Option<usize>> {
        let ParamKind::Int { max, .. } = param_spec(tool, name)?.kind else {
            return Err(EgError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Int"
            )));
        };
        let Some(v) = self.0.get(name).filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let mut n = v.as_u64().ok_or_else(|| {
            EgError::Usage(format!(
                "argument '{name}' must be a non-negative integer, got {v}"
            ))
        })? as usize;
        if let Some(max) = max {
            n = n.min(max.max(0) as usize);
        }
        Ok(Some(n))
    }

    /// Read a Float argument per its TOOL_SPECS declaration (absent or null
    /// falls back to `default`). A non-number is a usage error; out-of-range
    /// or non-finite values are rejected, matching the one declared Float's
    /// consumer contract (`build.rs` rejects outside [0, 1]).
    fn f_spec(&self, tool: &str, name: &str, default: f64) -> Result<f64> {
        let ParamKind::Float { min, max } = param_spec(tool, name)?.kind else {
            return Err(EgError::Internal(format!(
                "argument '{name}' of tool '{tool}' is not declared as Float"
            )));
        };
        let Some(v) = self.0.get(name).filter(|v| !v.is_null()) else {
            return Ok(default);
        };
        let x = v
            .as_f64()
            .filter(|x| x.is_finite() && *x >= min && *x <= max)
            .ok_or_else(|| {
                EgError::Usage(format!(
                    "argument '{name}' must be a finite number in [{min}, {max}], got {v}"
                ))
            })?;
        Ok(x)
    }

    /// Read a StrArray argument, executing the declared `maxItems` bound
    /// (MAX_STR_ARRAY_ITEMS, the same constant the JSON Schema renderer
    /// emits) here at the MCP boundary like every other declared bound:
    /// arrays above the cap are truncated to it, the clamp policy `n_opt_spec`
    /// applies to declared Int maxima. Every element is type-checked before
    /// the cap truncates.
    fn str_array(&self, name: &str) -> Result<Vec<String>> {
        let Some(v) = self.0.get(name).filter(|v| !v.is_null()) else {
            return Ok(Vec::new());
        };
        let arr = v.as_array().ok_or_else(|| {
            EgError::Usage(format!("argument '{name}' must be an array of strings"))
        })?;
        let mut out = Vec::with_capacity(arr.len().min(MAX_STR_ARRAY_ITEMS));
        for item in arr {
            let s = item.as_str().ok_or_else(|| {
                EgError::Usage(format!("argument '{name}' elements must be strings"))
            })?;
            out.push(s.to_string());
        }
        out.truncate(MAX_STR_ARRAY_ITEMS);
        Ok(out)
    }
}

/// Usage error for a tool name the `from_tool_args*` family does not route;
/// shared so the message cannot drift between the families.
fn unknown_tool(tool: &str) -> EgError {
    EgError::Usage(format!("unknown tool '{tool}'"))
}

pub fn from_tool_args(tool: &str, args: &serde_json::Value) -> Result<CommandRequest> {
    let a = ArgReader(args);
    match tool {
        "build" | "usage" => from_tool_args_diag(&a, tool),
        "callers" | "callees" | "context" | "imports" | "path" | "impact" => {
            from_tool_args_graph(&a, tool)
        }
        "symbols" | "reuse" | "search" => from_tool_args_analysis(&a, tool),
        "mem_write" | "mem_touch" | "mem_query" | "mem_get" | "mem_delete" | "mem_gc"
        | "mem_export" | "mem_import" | "mem_reindex" => from_tool_args_memory(&a, tool),
        _ => Err(unknown_tool(tool)),
    }
}

fn from_tool_args_diag(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "build" => CommandRequest::Build {
            force: a.b("force"),
            max_parse_error_rate: a.f_spec(
                tool,
                "max_parse_error_rate",
                crate::query::DEFAULT_MAX_PARSE_ERROR_RATE,
            )?,
        },
        "usage" => CommandRequest::Usage {
            quality: a
                .0
                .get("quality")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            global: a.0.get("global").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_graph(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "callers" | "callees" => {
            let name = a.req("name")?;
            let limit = a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?;
            let depth = a.n_spec(tool, "depth", crate::query::DEFAULT_DEPTH)?;
            if tool == "callers" {
                CommandRequest::Callers { name, limit, depth }
            } else {
                CommandRequest::Callees { name, limit, depth }
            }
        }
        "context" => CommandRequest::Context {
            name: a.req("name")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_CONTEXT_LIMIT)?,
        },
        "imports" => CommandRequest::Imports {
            file: a.req("file")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "path" => CommandRequest::Path {
            from: a.req("from")?,
            to: a.req("to")?,
            depth: a.n_spec(tool, "depth", crate::query::DEFAULT_PATH_DEPTH)?,
        },
        "impact" => CommandRequest::Impact {
            file: a.req("file")?,
            sites: a.b("sites"),
            depth: a.n_spec(tool, "depth", crate::query::DEFAULT_DEPTH)?,
            edges: crate::query::parse_edge_kinds(
                &a.s("edges")
                    .unwrap_or_else(|| crate::query::DEFAULT_IMPACT_EDGES.to_string()),
            )?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_analysis(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    if tool == "reuse" {
        if let Some(limit) = a.0.get("limit").and_then(serde_json::Value::as_i64) {
            if !(1..=100).contains(&limit) {
                return Err(EgError::Usage("limit must be 1..=100".to_string()));
            }
        }
    }
    Ok(match tool {
        "symbols" => CommandRequest::Symbols {
            file: a.req("file")?,
            outline: a.b("outline"),
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        "reuse" => CommandRequest::Reuse {
            paths: a.str_array("paths")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_REUSE_LIMIT)?,
            deep_callee: a.b_spec(tool, "deep_callee")?,
            min_confidence: a.s_opt_spec(tool, "min_confidence")?.unwrap_or_default(),
            propose: a.s_opt_spec(tool, "propose")?,
            callees: a.str_array("callees")?,
            explain: a.b_spec(tool, "explain")?,
            show_suppressed: a.b_spec(tool, "show_suppressed")?,
            coverage: a.str_array("coverage")?,
            judge: a.b_spec(tool, "judge")?,
        },
        "search" => CommandRequest::Search {
            terms: a.req("terms")?,
            limit: a.n_spec(tool, "limit", crate::query::DEFAULT_LIMIT)?,
        },
        _ => return Err(unknown_tool(tool)),
    })
}
fn from_tool_args_memory(a: &ArgReader<'_>, tool: &str) -> Result<CommandRequest> {
    Ok(match tool {
        "mem_write" => CommandRequest::MemWrite {
            key: a.req("key")?,
            body: a.req("body")?,
            kind: a.req("kind")?,
            tags: a.s("tags"),
            refs: a.s("refs"),
            scope: a.s("scope").unwrap_or_else(|| "durable".to_string()),
            ttl: a.n_opt_spec(tool, "ttl")?.map(|v| v as u64),
            supersedes: a.s("supersedes"),
            global: a.b("global"),
            touch: a.b("touch"),
            confirm: a.b("confirm"),
        },
        "mem_touch" => CommandRequest::MemTouch {
            key: a.req("key")?,
            global: a.b("global"),
        },
        "mem_query" => {
            let local = a.b_spec(tool, "local")?;
            let global = a.b("global");
            super::scope::StoreScope::from_flags(local, global)?;
            let window = crate::memory::TimeWindow {
                since: a
                    .s_opt_spec(tool, "since")?
                    .as_deref()
                    .map(|r| crate::memory::parse_time_bound(r, "mem query", "--since"))
                    .transpose()?,
                until: a
                    .s_opt_spec(tool, "until")?
                    .as_deref()
                    .map(|r| crate::memory::parse_time_bound(r, "mem query", "--until"))
                    .transpose()?,
            };
            crate::memory::validate_window(&window, "mem query")?;
            CommandRequest::MemQuery {
                terms: a
                    .s("terms")
                    .map(|t| t.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                all: a.b("all"),
                kind: crate::memory::parse_kind_filter(tool, a.s("kind").as_deref())?,
                scope: crate::memory::parse_scope_filter(tool, a.s("scope").as_deref())?,
                limit: a.n_spec(tool, "limit", crate::memory::DEFAULT_MEM_LIMIT)?,
                include_superseded: a.b("include_superseded"),
                global,
                reference: a.s("ref").or_else(|| a.s("reference")),
                since: window.since,
                until: window.until,
                local,
                explain: a.b_spec(tool, "explain")?,
                legacy: a.b_spec(tool, "legacy")?,
            }
        }
        "mem_get" => CommandRequest::MemGet {
            key: a.req("key")?,
            global: a.b("global"),
        },
        "mem_delete" => CommandRequest::MemDelete {
            key: a.req("key")?,
            global: a.b("global"),
        },
        "mem_gc" => CommandRequest::MemGc {
            global: a.b("global"),
        },
        "mem_export" => {
            let window = crate::memory::TimeWindow {
                since: a
                    .s_opt_spec(tool, "since")?
                    .as_deref()
                    .map(|r| crate::memory::parse_time_bound(r, "mem export", "--since"))
                    .transpose()?,
                until: a
                    .s_opt_spec(tool, "until")?
                    .as_deref()
                    .map(|r| crate::memory::parse_time_bound(r, "mem export", "--until"))
                    .transpose()?,
            };
            crate::memory::validate_window(&window, "mem export")?;
            CommandRequest::MemExport {
                format: a.s("format"),
                limit: a.n_opt_spec(tool, "limit")?,
                global: a.b("global"),
                since: window.since,
                until: window.until,
                scan: false,
            }
        }
        "mem_import" => CommandRequest::MemImport {
            path: a.req("path")?,
            format: a.s("format"),
            force: a.b("force"),
            // MCP writes are confined to the project store: untrusted repos
            // cannot pollute the user-global memory over the tool boundary.
            global: false,
        },
        "mem_reindex" => CommandRequest::MemReindex {
            global: a.b("global"),
        },
        _ => return Err(unknown_tool(tool)),
    })
}
