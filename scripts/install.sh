#!/usr/bin/env bash
# install.sh — Production build, symlink installation, and 5-agent configuration for engram (eg)
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

log_info() { echo -e "${CYAN}[eg-install]${RESET} $1"; }
log_ok() { echo -e "${GREEN}[eg-install]${RESET} $1"; }
log_warn() { echo -e "${YELLOW}[eg-install]${RESET} $1"; }
log_err() { echo -e "${RED}[eg-install]${RESET} $1" >&2; }

BIN_ONLY=0
SKIP_BUILD=0

usage() {
  cat <<EOF
Usage: ./scripts/install.sh [OPTIONS]

Options:
  --bin-only     Compile and install binary/scripts only; do not modify agent configurations.
  --skip-build   Skip cargo release compilation (use existing target/release/eg).
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

# Honour CARGO_TARGET_DIR: a custom target directory would otherwise leave a
# stale previous build behind at $REPO_ROOT/target/release/eg.
RELEASE_BIN="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/release/eg"
if [[ ! -f "$RELEASE_BIN" ]]; then
  log_err "Release binary not found at $RELEASE_BIN"
  exit 1
fi

# 2. Install binary & helper scripts
log_info "Installing binaries and scripts into ~/.cargo/bin and ~/.local/bin..."
mkdir -p "$HOME/.cargo/bin" "$HOME/.local/bin"

# Atomic replacement of binary to avoid macOS in-place inode execution kill (SIGKILL 9)
rm -f "$HOME/.cargo/bin/eg"
cp "$RELEASE_BIN" "$HOME/.cargo/bin/eg.tmp"
chmod +x "$HOME/.cargo/bin/eg.tmp"
mv -f "$HOME/.cargo/bin/eg.tmp" "$HOME/.cargo/bin/eg"
if command -v codesign >/dev/null 2>&1; then
  codesign -s - -f "$HOME/.cargo/bin/eg" >/dev/null 2>&1 || true
fi
ln -sf "$HOME/.cargo/bin/eg" "$HOME/.local/bin/eg"

# Clean up legacy transitional symlinks
rm -f "$HOME/.cargo/bin/aw" "$HOME/.local/bin/aw" "$HOME/.local/bin/aw-dashboard" "$HOME/.local/bin/eg-monitor"

# Symlink dashboard utility (native Rust launcher)
DASHBOARD_SRC="$REPO_ROOT/scripts/eg-dashboard"

if [[ -f "$DASHBOARD_SRC" ]]; then
  chmod +x "$DASHBOARD_SRC"
  ln -sf "$DASHBOARD_SRC" "$HOME/.local/bin/eg-dashboard"
fi

# Ensure ~/.engram exists for global telemetry and memory.
mkdir -p "$HOME/.engram"
chmod 700 "$HOME/.engram" 2>/dev/null || true

log_ok "Binaries and scripts installed:"
echo "  ├─ ~/.cargo/bin/eg"
echo "  ├─ ~/.local/bin/eg -> ~/.cargo/bin/eg"
echo "  └─ ~/.local/bin/eg-dashboard -> $DASHBOARD_SRC"

# Check PATH
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]] && [[ ":$PATH:" != *":$HOME/.cargo/bin:"* ]]; then
  log_warn "Neither ~/.local/bin nor ~/.cargo/bin is in your current PATH."
  log_warn "Add this to your shell config (~/.zshrc or ~/.bashrc):"
  log_warn "  export PATH=\"\$HOME/.local/bin:\$HOME/.cargo/bin:\$PATH\""
fi

# Verify version
ENGRAM_VERSION="$("$HOME/.cargo/bin/eg" --version 2>/dev/null || echo 'eg unknown')"
log_ok "Verified CLI: $ENGRAM_VERSION"

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

eg_bin = os.path.expanduser("~/.cargo/bin/eg")
repo_root = os.getcwd()
skill_source = os.path.join(repo_root, "skills", "engram-first", "SKILL.md")

rule_marker = "- **Code Navigation Invariant**:"
rule_text = (
    "- **Code Navigation Invariant**:\n"
    "  When locating callers, callees, the impact of a change, the call route between two symbols, "
    "a symbol's surroundings, a file's imports, or searching existing helpers, query `eg` (CLI or MCP `engram`) "
    "first before reading entire source files or running broad greps. Verify pointed lines directly.\n"
    "  - **Pre-Task**: Run `eg mem query --limit 3` as the first command of a session; cite any matching decision or gotcha, and verify a `[stale-ref]` note before relying on it.\n"
    "  - **Pre-Write / Post-Edit**: Run `eg search '<terms>'` before implementing helpers; run `eg reuse <paths>` to prevent duplicate functions.\n"
    "  - **Post-Fix**: Run `eg mem write <key> --kind gotcha --refs \"<path> <path>\" --body ...` to persist a durable learning before the task ends; reuse the same key to supersede.\n"
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
        # Replace the block already present. Skipping it as "already present"
        # meant an updated rule_text never reached a configured agent.
        lines = content.splitlines(keepends=True)
        out = []
        index = 0
        while index < len(lines):
            if lines[index].startswith(rule_marker):
                out.append(rule_text)
                index += 1
                # The block owns its indented sub-bullets; drop them with it.
                while index < len(lines) and lines[index].startswith("  "):
                    index += 1
                continue
            out.append(lines[index])
            index += 1
        new_content = "".join(out)
        if new_content == content:
            return True
    else:
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
    return True

# --- 1. Antigravity ---
print("1. Antigravity:")
gemini_config = os.path.expanduser("~/.gemini/config/mcp_config.json")
def update_gemini(data):
    servers = data.setdefault("mcpServers", {})
    if "engram" not in servers or servers["engram"].get("args") != ["mcp", "--all-tools"]:
        servers["engram"] = {"command": eg_bin, "args": ["mcp", "--all-tools"]}
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

# Generate Antigravity lazy-loaded MCP schemas in ~/.gemini/antigravity/mcp/engram
anti_mcp_schemas = os.path.expanduser("~/.gemini/antigravity/mcp/engram")
try:
    import subprocess
    proc = subprocess.Popen(
        [eg_bin, "mcp", "--all-tools"],
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
            f.write("# engram (eg) Best Practices\n\nQuery eg for code graph navigation before reading files or grepping.\n")
        print(f"  ├─ Generated {len(tools_data)} MCP tool schemas in ~/.gemini/antigravity/mcp/engram/")

        # Grant Antigravity MCP execution permissions in ~/.gemini/config/config.json
        anti_config_json = os.path.expanduser("~/.gemini/config/config.json")
        if os.path.exists(anti_config_json):
            def update_grants(data):
                grants = data.setdefault("userSettings", {}).setdefault("globalPermissionGrants", {}).setdefault("allow", [])
                needed = ["mcp(engram/*)"] + [f"mcp(engram/{t['name']})" for t in tools_data]
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

if deploy_skill(os.path.expanduser("~/.gemini/config/skills/engram-first")):
    print("  ├─ Deployed engram-first skill to ~/.gemini/config/skills/engram-first/")

if inject_rule(os.path.expanduser("~/.gemini/GEMINI.md")):
    print("  └─ Injected Code Navigation Invariant into ~/.gemini/GEMINI.md")

# --- 2. Grok Build ---
grok_dir = os.path.expanduser("~/.grok")
if os.path.exists(grok_dir):
    print("2. Grok Build:")
    grok_toml = os.path.join(grok_dir, "config.toml")
    if os.path.exists(grok_toml):
        toml_content = open(grok_toml, "r", encoding="utf-8").read()
        if "[mcp_servers.engram]" not in toml_content:
            mcp_block = (
                "\n[mcp_servers.engram]\n"
                f'command = "{eg_bin}"\n'
                'args = ["mcp", "--all-tools"]\n'
                "enabled = true\n"
            )
            with open(grok_toml, "a", encoding="utf-8") as f:
                f.write(mcp_block)
            print("  ├─ Added engram to ~/.grok/config.toml")
        else:
            print("  ├─ ~/.grok/config.toml already configured")
    if os.path.exists(os.path.join(grok_dir, "skills")):
        deploy_skill(os.path.join(grok_dir, "skills", "engram-first"))
        print("  ├─ Deployed engram-first skill to ~/.grok/skills/engram-first/")
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
        if "engram" not in servers or servers["engram"].get("args") != ["mcp", "--all-tools"]:
            servers["engram"] = {
                "type": "process",
                "command": eg_bin,
                "args": ["mcp", "--all-tools"]
            }
            return True
        return False
    if os.path.exists(os.path.dirname(zcode_cfg)):
        update_json_file(zcode_cfg, update_zcode)
        print("  ├─ Updated ~/.zcode/cli/config.json")
    if os.path.exists(os.path.join(zcode_dir, "skills")):
        deploy_skill(os.path.join(zcode_dir, "skills", "engram-first"))
        print("  ├─ Deployed engram-first skill to ~/.zcode/skills/engram-first/")
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
        if "engram" not in servers or servers["engram"].get("command") != [eg_bin, "mcp", "--all-tools"]:
            servers["engram"] = {
                "type": "local",
                "command": [eg_bin, "mcp", "--all-tools"]
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
        if '"engram"' not in content:
            if '"context_servers": {' in content:
                replacement = (
                    '"context_servers": {\n'
                    '    "engram": {\n'
                    f'      "command": "{eg_bin}",\n'
                    '      "args": ["mcp", "--all-tools"],\n'
                    '    },'
                )
                content = content.replace('"context_servers": {', replacement, 1)
                with open(zed_settings, "w", encoding="utf-8") as f:
                    f.write(content)
                print("  └─ Added engram context_server to ~/.config/zed/settings.json")
            else:
                print("  └─ Note: Add engram to context_servers in ~/.config/zed/settings.json")
        else:
            print("  └─ ~/.config/zed/settings.json already configured")

# --- 6. Claude Code (CLI) & Claude Desktop ---
claude_json = os.path.expanduser("~/.claude.json")
if os.path.exists(claude_json):
    print("6. Claude Code (CLI):")
    def update_claude_cli(data):
        servers = data.setdefault("mcpServers", {})
        if "engram" not in servers or servers["engram"].get("args") != ["mcp", "--all-tools"]:
            servers["engram"] = {"command": eg_bin, "args": ["mcp", "--all-tools"]}
            return True
        return False
    if update_json_file(claude_json, update_claude_cli):
        print("  ├─ Configured engram in ~/.claude.json")
    else:
        print("  ├─ ~/.claude.json already up-to-date")
    claude_dir = os.path.expanduser("~/.claude")
    deploy_skill(os.path.join(claude_dir, "skills", "engram-first"))
    print("  ├─ Deployed engram-first skill to ~/.claude/skills/engram-first/")
    inject_rule(os.path.join(claude_dir, "CLAUDE.md"))
    print("  └─ Injected Code Navigation Invariant into ~/.claude/CLAUDE.md")

claude_desktop_dir = os.path.expanduser("~/Library/Application Support/Claude")
if os.path.exists(claude_desktop_dir):
    claude_cfg = os.path.join(claude_desktop_dir, "claude_desktop_config.json")
    def update_claude_desktop(data):
        servers = data.setdefault("mcpServers", {})
        if "engram" not in servers or servers["engram"].get("args") != ["mcp", "--all-tools"]:
            servers["engram"] = {"command": eg_bin, "args": ["mcp", "--all-tools"]}
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
        if "[mcp_servers.engram]" not in content:
            mcp_block = (
                "\n[mcp_servers.engram]\n"
                f'command = "{eg_bin}"\n'
                'args = ["mcp", "--all-tools"]\n'
                "enabled = true\n"
            )
            with open(codex_toml, "a", encoding="utf-8") as f:
                f.write(mcp_block)
            print("  ├─ Added engram to ~/.codex/config.toml")
        else:
            print("  ├─ ~/.codex/config.toml already configured")
    deploy_skill(os.path.join(codex_dir, "skills", "engram-first"))
    print("  ├─ Deployed engram-first skill to ~/.codex/skills/engram-first/")
    inject_rule(os.path.join(codex_dir, "AGENTS.md"))
    print("  └─ Injected Code Navigation Invariant into ~/.codex/AGENTS.md")

# --- Purge legacy MCP entries ---
LEGACY_MCP_NAME = "agent-weave"

def _servers_node(data, path_keys):
    node = data
    for key in path_keys:
        if not isinstance(node, dict):
            return None
        node = node.get(key)
    return node if isinstance(node, dict) else None

def purge_legacy_json(path):
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return False
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception:
        return False
    changed = False
    for path_keys in (("mcpServers",), ("mcp", "servers"), ("context_servers",)):
        servers = _servers_node(data, path_keys)
        if servers is None:
            continue
        if LEGACY_MCP_NAME in servers:
            del servers[LEGACY_MCP_NAME]
            changed = True
    if changed:
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
    return changed

for cfg in (
    gemini_config,
    anti_mcp,
    os.path.expanduser("~/.zcode/cli/config.json"),
    os.path.expanduser("~/.config/opencode/opencode.json"),
    os.path.expanduser("~/.config/zed/settings.json"),
    claude_json,
):
    if purge_legacy_json(cfg):
        print(f"  ├─ Purged legacy {LEGACY_MCP_NAME} entry from {cfg}")

def purge_legacy_toml(path):
    if not os.path.exists(path):
        return False
    try:
        with open(path, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except Exception:
        return False
    new_lines = []
    skip = False
    changed = False
    for line in lines:
        if line.strip() == f"[mcp_servers.{LEGACY_MCP_NAME}]":
            skip = True
            changed = True
            continue
        if skip:
            if line.startswith("["):
                skip = False
                new_lines.append(line)
            continue
        new_lines.append(line)
    if changed:
        with open(path, "w", encoding="utf-8") as f:
            f.writelines(new_lines)
    return changed

for cfg in (os.path.expanduser("~/.grok/config.toml"), os.path.expanduser("~/.codex/config.toml")):
    if purge_legacy_toml(cfg):
        print(f"  ├─ Purged legacy {LEGACY_MCP_NAME} entry from {cfg}")
PYEOF

# 4. Local Workspace Index
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  log_info "Building local repository index (.engram/index.db)..."
  "$HOME/.cargo/bin/eg" build
fi

# 5. Semantic Judge Wiring (optional)
if command -v eg-judge >/dev/null 2>&1; then
  JUDGE_BIN="$(command -v eg-judge)"
  JUDGE_MODEL="${EG_JUDGE_MODEL:-$HOME/.engram/judge/model}"
  log_info "Detected eg-judge at $JUDGE_BIN"
  echo "  Add these to your shell config to enable 'eg reuse --judge':"
  echo "    export EG_JUDGE_BIN=\"$JUDGE_BIN\""
  echo "    export EG_JUDGE_MODEL=\"$JUDGE_MODEL\""
  if [ ! -f "$JUDGE_MODEL/lazy.json" ]; then
    log_warn "No checkpoint at $JUDGE_MODEL; fetch one before using --judge."
  fi
  for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
    [ -f "$rc" ] || continue
    if grep -q "EG_JUDGE_BIN" "$rc"; then
      continue
    fi
    printf '\n# eg-judge (engram reuse --judge)\nexport EG_JUDGE_BIN="%s"\nexport EG_JUDGE_MODEL="%s"\n' \
      "$JUDGE_BIN" "$JUDGE_MODEL" >> "$rc"
    echo "  ├─ Added EG_JUDGE_* to $rc"
  done
else
  log_info "eg-judge not on PATH; 'eg reuse --judge' stays disabled (static reuse is unchanged)."
fi

# 6. Global Telemetry Sanity Check
log_info "Verifying global telemetry (~/.engram/usage.log)..."
"$HOME/.cargo/bin/eg" usage --global

echo ""
log_ok "Installation and agent configuration completed successfully!"
echo -e "${BOLD}Operational Commands:${RESET}"
echo "  • eg dashboard         Launch native live streaming Web SSE dashboard (http://localhost:7890)"
echo "  • eg monitor           Stream live query events in terminal (ANSI stream)"
echo "  • eg-dashboard         Shortcut to launch native web dashboard"
echo "  • eg usage --global    Print machine-wide telemetry & token savings summary"
