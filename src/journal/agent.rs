//! Caller attribution: which agent invoked `eg`.
//!
//! Identity comes from the MCP handshake when present, then from an explicit
//! override, then from the environment markers the agent CLIs export to the
//! shells they spawn. Editor markers are deliberately narrow: Zed sets
//! `TERM_PROGRAM`/`ZED_TERM` for every terminal it opens, so those would label
//! a human typing in Zed as an agent. Only `ZED_ENVIRONMENT=worktree-shell`,
//! which the agent worktree shell sets, is trusted.
//!
//! Only a marker's presence is read, never its value: several of these
//! variables ship secrets.

use std::cell::RefCell;

thread_local! {
    static CURRENT_AGENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Sanitized environment tag: first set key wins, trimmed and capped at 64.
pub(crate) fn env_tag(keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|v| v.trim().chars().take(64).collect())
        .unwrap_or_default()
}

/// Session attribution for memory rows and journal lines.
pub fn session_tag() -> String {
    env_tag(&["ENGRAM_SESSION"])
}

/// Set active agent context for subsequent journal invocations in this process
/// (e.g. from the MCP handshake `clientInfo.name`).
pub fn set_agent_context(name: &str) {
    if let Some(agent) = canonical_agent(name) {
        CURRENT_AGENT.with(|a| *a.borrow_mut() = Some(agent));
    }
}

/// Alias -> canonical slug. A client may identify itself with a product name
/// (`antigravity-client`), a binary name (`claude`), or a spelling variant;
/// without one table the dashboard splits one agent across several rows.
const AGENT_ALIASES: &[(&str, &str)] = &[
    ("antigravity", "antigravity"),
    ("antigravity-client", "antigravity"),
    ("claude", "claude-code"),
    ("claude-code", "claude-code"),
    ("claudecode", "claude-code"),
    ("claude_code", "claude-code"),
    ("codex", "codex"),
    ("opencode", "opencode"),
    ("zed", "zed"),
    ("zcode", "zcode"),
    ("user", "user"),
];

/// One canonical spelling for a caller-reported name. An unknown name is kept
/// as written (trimmed and capped): an operator's `ENGRAM_AGENT` tag must
/// survive even when it is not in the table.
fn canonical_agent(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some((_, canonical)) = AGENT_ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Some((*canonical).to_string());
    }
    Some(trimmed.chars().take(32).collect())
}

/// Env markers the agent CLIs export to the shells they spawn, innermost agent
/// first: Claude Code hosted inside Zed sets both `CLAUDECODE` and the Zed
/// marker, and the agent must win over the editor.
///
/// ZCode desktop exports `ZCODE_APP_VERSION` and `ZCODE_PROCESS_LABEL` into
/// every shell its tools spawn; without them an agent-driven call falls back
/// to `user` and the dashboard cannot tell the driver from a human terminal.
/// `ZCODE_MANAGED_PACKAGE` covers the packaged-agent variant.
const AGENT_MARKERS: &[(&str, &str)] = &[
    ("CLAUDECODE", "claude-code"),
    ("CODEX_TOOL_CALL_ID", "codex"),
    ("ZCODE_MANAGED_PACKAGE", "zcode"),
    ("ZCODE_APP_VERSION", "zcode"),
    ("ZCODE_PROCESS_LABEL", "zcode"),
    ("OPENCODE_CLIENT", "opencode"),
    ("ANTIGRAVITY_AGENT", "antigravity"),
];

/// The one Zed marker that identifies the agent worktree shell rather than
/// every terminal the editor opens.
const ZED_WORKTREE_SHELL: &str = "worktree-shell";

/// Detect the calling agent identity from explicit context, environment markers,
/// or fall back to `"user"`.
pub fn detect_agent() -> String {
    identify(CURRENT_AGENT.with(|a| a.borrow().clone()), |key| {
        std::env::var(key).ok()
    })
}

fn identify(current: Option<String>, lookup: impl Fn(&str) -> Option<String>) -> String {
    if let Some(agent) = current {
        return agent;
    }
    // An explicit operator tag beats detection: an agent that exports it must
    // not be renamed by whichever marker its launcher happens to leave behind.
    if let Some(raw) = lookup("ENGRAM_AGENT") {
        if let Some(agent) = canonical_agent(&raw) {
            return agent;
        }
    }
    for (key, name) in AGENT_MARKERS {
        if lookup(key).is_some() {
            return (*name).to_string();
        }
    }
    if lookup("ZED_ENVIRONMENT").as_deref() == Some(ZED_WORKTREE_SHELL) {
        return "zed".to_string();
    }
    "user".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn context_beats_every_marker() {
        let lookup = lookup_from(&[("CLAUDECODE", "1")]);
        assert_eq!(
            identify(Some("mcp-client".to_string()), lookup),
            "mcp-client"
        );
    }

    #[test]
    fn explicit_agent_env_wins_over_markers() {
        let lookup = lookup_from(&[("ENGRAM_AGENT", "my-agent"), ("CLAUDECODE", "1")]);
        assert_eq!(identify(None, lookup), "my-agent");
    }

    #[test]
    fn blank_explicit_agent_env_falls_through() {
        let lookup = lookup_from(&[("ENGRAM_AGENT", "   "), ("CLAUDECODE", "1")]);
        assert_eq!(identify(None, lookup), "claude-code");
    }

    #[test]
    fn agent_markers_are_recognized() {
        for (key, expected) in [
            ("CLAUDECODE", "claude-code"),
            ("CODEX_TOOL_CALL_ID", "codex"),
            ("ZCODE_MANAGED_PACKAGE", "zcode"),
            ("ZCODE_APP_VERSION", "zcode"),
            ("ZCODE_PROCESS_LABEL", "zcode"),
            ("OPENCODE_CLIENT", "opencode"),
            ("ANTIGRAVITY_AGENT", "antigravity"),
        ] {
            assert_eq!(identify(None, lookup_from(&[(key, "1")])), expected);
        }
    }

    #[test]
    fn agent_marker_beats_editor_marker() {
        let lookup = lookup_from(&[("CLAUDECODE", "1"), ("ZED_ENVIRONMENT", "worktree-shell")]);
        assert_eq!(identify(None, lookup), "claude-code");
    }

    #[test]
    fn zed_worktree_shell_is_detected() {
        let lookup = lookup_from(&[("ZED_ENVIRONMENT", "worktree-shell")]);
        assert_eq!(identify(None, lookup), "zed");
    }

    #[test]
    fn plain_zed_terminal_is_not_an_agent() {
        let lookup = lookup_from(&[
            ("TERM_PROGRAM", "zed"),
            ("ZED_TERM", "true"),
            ("ZED_ENVIRONMENT", "editor"),
        ]);
        assert_eq!(identify(None, lookup), "user");
    }

    #[test]
    fn aliases_collapse_to_one_slug() {
        for (raw, expected) in [
            ("antigravity-client", "antigravity"),
            ("Antigravity", "antigravity"),
            ("claude", "claude-code"),
            ("CLAUDE_CODE", "claude-code"),
            ("claudecode", "claude-code"),
            ("Zed", "zed"),
        ] {
            assert_eq!(canonical_agent(raw).as_deref(), Some(expected));
        }
    }

    #[test]
    fn unknown_names_are_kept_as_written() {
        assert_eq!(canonical_agent("  MyAgent ").as_deref(), Some("MyAgent"));
        assert_eq!(canonical_agent("   "), None);
    }

    #[test]
    fn mcp_context_is_canonicalized() {
        set_agent_context("antigravity-client");
        assert_eq!(detect_agent(), "antigravity");
        set_agent_context("custom-agent");
        assert_eq!(detect_agent(), "custom-agent");
    }

    #[test]
    fn explicit_env_is_canonicalized() {
        let lookup = lookup_from(&[("ENGRAM_AGENT", "claude")]);
        assert_eq!(identify(None, lookup), "claude-code");
    }

    #[test]
    fn no_markers_falls_back_to_user() {
        assert_eq!(identify(None, lookup_from(&[])), "user");
    }
}
