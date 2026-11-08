#!/usr/bin/env bash
# install.sh — Production build, symlink installation, and 5-agent configuration for agent-weave (aw)
# Supports: Antigravity, Grok Build, Zcode, OpenCode, Zed, Claude Desktop, Cursor.
set -euo pipefail

SOURCE="${BASH_SOURCE[0]}"
while [ -h "$SOURCE" ]; do
  DIR="$( cd -P "$( dirname "$SOURCE" )" >/dev/null 2>&1 && pwd )"
  SOURCE="$(readlink "$SOURCE")"
  [[ $SOURCE != /* ]] && SOURCE="$DIR/$SOURCE"
done
SCRIPT_DIR="$( cd -P "$( dirname "$SOURCE" )" >/dev/null 2>&1 && pwd )"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ANSI formatting
BOLD="\033[1m"
GREEN="\033[0;32m"
YELLOW="\033[0;33m"
CYAN="\033[0;36m"
RED="\033[0;31m"
RESET="\033[0m"

log_info() { echo -e "${CYAN}[aw-install]${RESET} $1"; }
log_ok() { echo -e "${GREEN}[aw-install]${RESET} $1"; }
log_warn() { echo -e "${YELLOW}[aw-install]${RESET} $1"; }
log_err() { echo -e "${RED}[aw-install]${RESET} $1" >&2; }

BIN_ONLY=0
SKIP_BUILD=0

usage() {
  cat <<EOF
Usage: ./scripts/install.sh [OPTIONS]

Options:
  --bin-only     Compile and install binary/scripts only; do not modify agent configurations.
  --skip-build   Skip cargo release compilation (use existing target/release/aw).
  -h, --help     Show this help message.
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-only)
      BIN_ONLY=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      log_err "Unknown argument: $1"
      usage
      ;;
  esac
done

cd "$REPO_ROOT"

# 1. Compile release binary
if [[ $SKIP_BUILD -eq 0 ]]; then
  log_info "Compiling release binary with cargo..."
  if ! command -v cargo >/dev/null 2>&1; then
    log_err "cargo not found in PATH. Please install Rust 1.90+."
    exit 1
  fi
  cargo build --release --locked
fi

RELEASE_BIN="$REPO_ROOT/target/release/aw"
if [[ ! -f "$RELEASE_BIN" ]]; then
  log_err "Release binary not found at $RELEASE_BIN"
  exit 1
fi

# 2. Install binary & helper scripts
log_info "Installing binaries and scripts into ~/.cargo/bin and ~/.local/bin..."
mkdir -p "$HOME/.cargo/bin" "$HOME/.local/bin"

# Atomic replacement of binary to avoid macOS in-place inode execution kill (SIGKILL 9)
rm -f "$HOME/.cargo/bin/aw"
cp "$RELEASE_BIN" "$HOME/.cargo/bin/aw.tmp"
chmod +x "$HOME/.cargo/bin/aw.tmp"
mv -f "$HOME/.cargo/bin/aw.tmp" "$HOME/.cargo/bin/aw"
if command -v codesign >/dev/null 2>&1; then
  codesign -s - -f "$HOME/.cargo/bin/aw" >/dev/null 2>&1 || true
fi
ln -sf "$HOME/.cargo/bin/aw" "$HOME/.local/bin/aw"

# Symlink monitor and dashboard utilities
MONITOR_SRC="$REPO_ROOT/scripts/aw-monitor"
DASHBOARD_SRC="$REPO_ROOT/scripts/aw-dashboard"

if [[ -f "$MONITOR_SRC" ]]; then
  chmod +x "$MONITOR_SRC"
  ln -sf "$MONITOR_SRC" "$HOME/.local/bin/aw-monitor"
fi

if [[ -f "$DASHBOARD_SRC" ]]; then
  chmod +x "$DASHBOARD_SRC"
  ln -sf "$DASHBOARD_SRC" "$HOME/.local/bin/aw-dashboard"
fi

# Ensure ~/.aw exists for global telemetry and memory
mkdir -p "$HOME/.aw"

log_ok "Binaries and scripts installed:"
echo "  ├─ ~/.cargo/bin/aw"
echo "  ├─ ~/.local/bin/aw -> ~/.cargo/bin/aw"
echo "  ├─ ~/.local/bin/aw-monitor -> $MONITOR_SRC"
echo "  └─ ~/.local/bin/aw-dashboard -> $DASHBOARD_SRC"

# Check PATH
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]] && [[ ":$PATH:" != *":$HOME/.cargo/bin:"* ]]; then
  log_warn "Neither ~/.local/bin nor ~/.cargo/bin is in your current PATH."
  log_warn "Add this to your shell config (~/.zshrc or ~/.bashrc):"
  log_warn "  export PATH=\"\$HOME/.local/bin:\$HOME/.cargo/bin:\$PATH\""
fi

# Verify version
AW_VERSION="$("$HOME/.cargo/bin/aw" --version 2>/dev/null || echo 'aw unknown')"
log_ok "Verified CLI: $AW_VERSION"

if [[ $BIN_ONLY -eq 1 ]]; then
  log_ok "Completed binary-only installation."
  exit 0
fi

# 3. Configure Active Coding Agents
log_info "Configuring active coding agents (MCP servers, skills, and routing rules)..."

python3 - <<'PYEOF'
import os
import json
import re

aw_bin = os.path.expanduser("~/.cargo/bin/aw")
repo_root = os.getcwd()
skill_source = os.path.join(repo_root, "skills", "aw-first", "SKILL.md")

rule_marker = "- **Code Navigation Invariant**:"
rule_text = (
    "- **Code Navigation Invariant**:\n"
    "  When locating callers, callees, the impact of a change, the call route between two symbols, "
    "a symbol's surroundings, a file's imports, or searching existing helpers, query `aw` (CLI or MCP `agent-weave`) "
    "first before reading entire source files or running broad greps. Verify pointed lines directly.\n"
    "  - **Pre-Task**: Run `aw mem query` to inspect prior architectural decisions, gotchas, and conventions.\n"
    "  - **Pre-Write / Post-Edit**: Run `aw search '<terms>'` before implementing helpers; run `aw reuse <paths>` to prevent duplicate functions.\n"
    "  - **Post-Fix**: Run `aw mem write <key> --kind gotcha|decision --body ... --refs <paths>` to persist durable learnings before task completion.\n"
)

def update_json_file(path, updater_fn):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    data = {}
    if os.path.exists(path) and os.path.getsize(path) > 0:
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except Exception as e:
            print(f"  [warn] Could not parse JSON at {path}: {e}")
            return False
    modified = updater_fn(data)
    if modified:
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
        return True
    return False

def deploy_skill(target_dir):
    if not os.path.exists(skill_source):
        return False
    os.makedirs(target_dir, exist_ok=True)
    target_file = os.path.join(target_dir, "SKILL.md")
    with open(skill_source, "r", encoding="utf-8") as sf:
        content = sf.read()
    with open(target_file, "w", encoding="utf-8") as tf:
        tf.write(content)
    return True

def inject_rule(rules_path):
    if not os.path.exists(rules_path):
        return False
    with open(rules_path, "r", encoding="utf-8") as f:
        content = f.read()
    if rule_marker in content:
        return True  # already present
    # Insert under Section 3 (Code Quality & Verification) if present
    target_section = "## 3. Code Quality & Verification\n"
    if target_section in content:
        parts = content.split(target_section, 1)
        new_content = parts[0] + target_section + rule_text + parts[1]
    else:
        new_content = content + "\n" + rule_text
    with open(rules_path, "w", encoding="utf-8") as f:
        f.write(new_content)
    return True

# --- 1. Antigravity ---
print("1. Antigravity:")
gemini_config = os.path.expanduser("~/.gemini/config/mcp_config.json")
def update_gemini(data):
    servers = data.setdefault("mcpServers", {})
    if "agent-weave" not in servers or servers["agent-weave"].get("args") != ["mcp", "--all-tools"]:
        servers["agent-weave"] = {"command": aw_bin, "args": ["mcp", "--all-tools"]}
        return True
    return False
if update_json_file(gemini_config, update_gemini):
    print("  ├─ Updated ~/.gemini/config/mcp_config.json")
else:
    print("  ├─ ~/.gemini/config/mcp_config.json already up-to-date")

# Mirror to antigravity appDataDir mcp_config.json if dir exists
anti_mcp = os.path.expanduser("~/.gemini/antigravity/mcp_config.json")
if os.path.exists(os.path.dirname(anti_mcp)):
    update_json_file(anti_mcp, update_gemini)
    print("  ├─ Synced ~/.gemini/antigravity/mcp_config.json")

# Generate Antigravity lazy-loaded MCP schemas in ~/.gemini/antigravity/mcp/agent-weave
anti_mcp_schemas = os.path.expanduser("~/.gemini/antigravity/mcp/agent-weave")
try:
    import subprocess
    proc = subprocess.Popen(
        [aw_bin, "mcp", "--all-tools"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    init_req = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "setup", "version": "1.0"}}}) + "\n"
    list_req = json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}) + "\n"
    proc.stdin.write(init_req)
    proc.stdin.write(list_req)
    proc.stdin.flush()
    proc.stdout.readline()
    list_line = proc.stdout.readline()
    proc.terminate()
    tools_data = json.loads(list_line).get("result", {}).get("tools", [])
    if tools_data:
        os.makedirs(anti_mcp_schemas, exist_ok=True)
        for t in tools_data:
            params = t.get("inputSchema", {})
            if "$schema" not in params:
                params["$schema"] = "https://json-schema.org/draft/2020-12/schema"
            with open(os.path.join(anti_mcp_schemas, f"{t['name']}.json"), "w", encoding="utf-8") as f:
                json.dump({"name": t["name"], "description": t.get("description", ""), "parameters": params}, f, indent=2)
        with open(os.path.join(anti_mcp_schemas, "instructions.md"), "w", encoding="utf-8") as f:
            f.write("# agent-weave (aw) Best Practices\n\nQuery aw for code graph navigation before reading files or grepping.\n")
        print(f"  ├─ Generated {len(tools_data)} MCP tool schemas in ~/.gemini/antigravity/mcp/agent-weave/")

        # Grant Antigravity MCP execution permissions in ~/.gemini/config/config.json
        anti_config_json = os.path.expanduser("~/.gemini/config/config.json")
        if os.path.exists(anti_config_json):
            def update_grants(data):
                grants = data.setdefault("userSettings", {}).setdefault("globalPermissionGrants", {}).setdefault("allow", [])
                needed = ["mcp(agent-weave/*)"] + [f"mcp(agent-weave/{t['name']})" for t in tools_data]
                mod = False
                for g in needed:
                    if g not in grants:
                        grants.append(g)
                        mod = True
                return mod
            if update_json_file(anti_config_json, update_grants):
                print("  ├─ Granted MCP execution permissions in ~/.gemini/config/config.json")
except Exception as e:
    print(f"  [warn] Could not generate Antigravity MCP schemas: {e}")

if deploy_skill(os.path.expanduser("~/.gemini/config/skills/aw-first")):
    print("  ├─ Deployed aw-first skill to ~/.gemini/config/skills/aw-first/")

if inject_rule(os.path.expanduser("~/.gemini/GEMINI.md")):
    print("  └─ Injected Code Navigation Invariant into ~/.gemini/GEMINI.md")

# --- 2. Grok Build ---
grok_dir = os.path.expanduser("~/.grok")
if os.path.exists(grok_dir):
    print("2. Grok Build:")
    grok_toml = os.path.join(grok_dir, "config.toml")
    if os.path.exists(grok_toml):
        toml_content = open(grok_toml, "r", encoding="utf-8").read()
        if "[mcp_servers.agent-weave]" not in toml_content:
            mcp_block = (
                "\n[mcp_servers.agent-weave]\n"
                f'command = "{aw_bin}"\n'
                'args = ["mcp", "--all-tools"]\n'
                "enabled = true\n"
            )
            with open(grok_toml, "a", encoding="utf-8") as f:
                f.write(mcp_block)
            print("  ├─ Added agent-weave to ~/.grok/config.toml")
        else:
            print("  ├─ ~/.grok/config.toml already configured")
    if os.path.exists(os.path.join(grok_dir, "skills")):
        deploy_skill(os.path.join(grok_dir, "skills", "aw-first"))
        print("  ├─ Deployed aw-first skill to ~/.grok/skills/aw-first/")
    if inject_rule(os.path.join(grok_dir, "AGENTS.md")):
        print("  └─ Injected Code Navigation Invariant into ~/.grok/AGENTS.md")

# --- 3. Zcode ---
zcode_dir = os.path.expanduser("~/.zcode")
if os.path.exists(zcode_dir):
    print("3. Zcode:")
    zcode_cfg = os.path.join(zcode_dir, "cli", "config.json")
    def update_zcode(data):
        mcp = data.setdefault("mcp", {})
        servers = mcp.setdefault("servers", {})
        if "agent-weave" not in servers or servers["agent-weave"].get("args") != ["mcp", "--all-tools"]:
            servers["agent-weave"] = {
                "type": "process",
                "command": aw_bin,
                "args": ["mcp", "--all-tools"]
            }
            return True
        return False
    if os.path.exists(os.path.dirname(zcode_cfg)):
        update_json_file(zcode_cfg, update_zcode)
        print("  ├─ Updated ~/.zcode/cli/config.json")
    if os.path.exists(os.path.join(zcode_dir, "skills")):
        deploy_skill(os.path.join(zcode_dir, "skills", "aw-first"))
        print("  ├─ Deployed aw-first skill to ~/.zcode/skills/aw-first/")
    if inject_rule(os.path.join(zcode_dir, "AGENTS.md")):
        print("  └─ Injected Code Navigation Invariant into ~/.zcode/AGENTS.md")

# --- 4. OpenCode ---
opencode_dir = os.path.expanduser("~/.config/opencode")
if os.path.exists(opencode_dir):
    print("4. OpenCode:")
    opencode_json = os.path.join(opencode_dir, "opencode.json")
    def update_opencode(data):
        mcp = data.setdefault("mcp", {})
        servers = mcp.setdefault("servers", {})
        if "agent-weave" not in servers or servers["agent-weave"].get("command") != [aw_bin, "mcp", "--all-tools"]:
            servers["agent-weave"] = {
                "type": "local",
                "command": [aw_bin, "mcp", "--all-tools"]
            }
            return True
        return False
    if update_json_file(opencode_json, update_opencode):
        print("  ├─ Updated ~/.config/opencode/opencode.json")
    else:
        print("  ├─ ~/.config/opencode/opencode.json already configured")
    if inject_rule(os.path.join(opencode_dir, "AGENTS.md")):
        print("  └─ Injected Code Navigation Invariant into ~/.config/opencode/AGENTS.md")

# --- 5. Zed ---
zed_dir = os.path.expanduser("~/.config/zed")
if os.path.exists(zed_dir):
    print("5. Zed:")
    zed_settings = os.path.join(zed_dir, "settings.json")
    if os.path.exists(zed_settings):
        content = open(zed_settings, "r", encoding="utf-8").read()
        if '"agent-weave"' not in content:
            if '"context_servers": {' in content:
                replacement = (
                    '"context_servers": {\n'
                    '    "agent-weave": {\n'
                    f'      "command": "{aw_bin}",\n'
                    '      "args": ["mcp", "--all-tools"],\n'
                    '    },'
                )
                content = content.replace('"context_servers": {', replacement, 1)
                with open(zed_settings, "w", encoding="utf-8") as f:
                    f.write(content)
                print("  └─ Added agent-weave context_server to ~/.config/zed/settings.json")
            else:
                print("  └─ Note: Add agent-weave to context_servers in ~/.config/zed/settings.json")
        else:
            print("  └─ ~/.config/zed/settings.json already configured")

# --- 6. Claude Code (CLI) & Claude Desktop ---
claude_json = os.path.expanduser("~/.claude.json")
if os.path.exists(claude_json):
    print("6. Claude Code (CLI):")
    def update_claude_cli(data):
        servers = data.setdefault("mcpServers", {})
        if "agent-weave" not in servers or servers["agent-weave"].get("args") != ["mcp", "--all-tools"]:
            servers["agent-weave"] = {"command": aw_bin, "args": ["mcp", "--all-tools"]}
            return True
        return False
    if update_json_file(claude_json, update_claude_cli):
        print("  ├─ Configured agent-weave in ~/.claude.json")
    else:
        print("  ├─ ~/.claude.json already up-to-date")
    claude_dir = os.path.expanduser("~/.claude")
    deploy_skill(os.path.join(claude_dir, "skills", "aw-first"))
    print("  ├─ Deployed aw-first skill to ~/.claude/skills/aw-first/")
    inject_rule(os.path.join(claude_dir, "CLAUDE.md"))
    print("  └─ Injected Code Navigation Invariant into ~/.claude/CLAUDE.md")

claude_desktop_dir = os.path.expanduser("~/Library/Application Support/Claude")
if os.path.exists(claude_desktop_dir):
    claude_cfg = os.path.join(claude_desktop_dir, "claude_desktop_config.json")
    def update_claude_desktop(data):
        servers = data.setdefault("mcpServers", {})
        if "agent-weave" not in servers or servers["agent-weave"].get("args") != ["mcp", "--all-tools"]:
            servers["agent-weave"] = {"command": aw_bin, "args": ["mcp", "--all-tools"]}
            return True
        return False
    update_json_file(claude_cfg, update_claude_desktop)
    print("  └─ Configured Claude Desktop in claude_desktop_config.json")

# --- 7. OpenAI Codex ---
codex_dir = os.path.expanduser("~/.codex")
if os.path.exists(codex_dir):
    print("7. OpenAI Codex:")
    codex_toml = os.path.join(codex_dir, "config.toml")
    if os.path.exists(codex_toml):
        content = open(codex_toml, "r", encoding="utf-8").read()
        if "[mcp_servers.agent-weave]" not in content:
            mcp_block = (
                "\n[mcp_servers.agent-weave]\n"
                f'command = "{aw_bin}"\n'
                'args = ["mcp", "--all-tools"]\n'
                "enabled = true\n"
            )
            with open(codex_toml, "a", encoding="utf-8") as f:
                f.write(mcp_block)
            print("  ├─ Added agent-weave to ~/.codex/config.toml")
        else:
            print("  ├─ ~/.codex/config.toml already configured")
    deploy_skill(os.path.join(codex_dir, "skills", "aw-first"))
    print("  ├─ Deployed aw-first skill to ~/.codex/skills/aw-first/")
    inject_rule(os.path.join(codex_dir, "AGENTS.md"))
    print("  └─ Injected Code Navigation Invariant into ~/.codex/AGENTS.md")
PYEOF

# 4. Local Workspace Index
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  log_info "Building local repository index (.aw/index.db)..."
  "$HOME/.cargo/bin/aw" build
fi

# 5. Global Telemetry Sanity Check
log_info "Verifying global telemetry (~/.aw/usage.log)..."
"$HOME/.cargo/bin/aw" usage --global

echo ""
log_ok "Installation and agent configuration completed successfully!"
echo -e "${BOLD}Operational Commands:${RESET}"
echo "  • aw-monitor           Launch interactive real-time TUI terminal monitor"
echo "  • aw-monitor --web     Launch live streaming Web SSE dashboard (http://localhost:7890)"
echo "  • aw-dashboard         Open multi-repo HTML dashboard in browser"
echo "  • aw usage --global    Print machine-wide telemetry & token savings summary"
