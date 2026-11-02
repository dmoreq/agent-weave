#!/usr/bin/env bash
# Stage 1 runner — agent-utility A/B for `kg path` (Amendment 13).
# 24 cells: 3 tasks x 2 arms x 2 hosts x 2 runs, sequential.
# Idempotent: a cell whose runid already appears in status.txt is skipped.
# Optional arg: max number of NEW cells to run in this invocation.
set -uo pipefail

KG=/Users/quy.doan/Workspace/kg/target/release/kg
GROK=/Users/quy.doan/.grok/bin/grok
OC=/Users/quy.doan/.opencode/bin/opencode
ROOT=/Users/quy.doan/Workspace/kg/docs/baselines/utility-run-2026-11-02
MAX="${1:-99}"
export HOME="${HOME:-/Users/quy.doan}"

repo_for() { # task -> sanitized corpus path (no eval artifacts; bash 3.2)
  case "$1" in
    P1|P2) echo /tmp/utility-corpus-2026-11-02/kg ;;
    P3) echo /tmp/utility-corpus-2026-11-02/vihap ;;
    *) echo "" ;;
  esac
}
KILL_SECS=480

run_with_timeout() { # secs cmd...
  local secs=$1; shift
  "$@" & local pid=$!
  ( sleep "$secs"; kill -9 "$pid" 2>/dev/null ) & local wpid=$!
  wait "$pid"; local rc=$?
  kill "$wpid" 2>/dev/null
  return "$rc"
}

run_cell() { # host arm task run
  local host=$1 arm=$2 task=$3 run=$4
  local hosttag="$host"
  [ "$host" = "opencode" ] && hosttag="oc"
  local runid="$hosttag-$arm-r$run-$task"
  local repo
  repo=$(repo_for "$task")
  grep -q "^\[done\] $runid " "$ROOT/status.txt" 2>/dev/null && { echo "[skip] $runid"; return 0; }
  env -u KG_ARM -u KG_SESSION "$KG" build --root "$repo" >/dev/null 2>&1
  rm -f "$repo/.kg/usage.log"* "$repo/.kg/usage-results.jsonl"*
  local prompt
  prompt=$(sed -e "s|{ANSWER_PATH}|/tmp/utility-answers-2026-11-02/$runid.json|g" "$ROOT/prompts/$arm-$task.txt")
  echo "[start] $runid host=$host repo=$repo $(date +%H:%M:%S)" | tee -a "$ROOT/status.txt"
  local sid rc
  if [ "$host" = grok ]; then
    sid=$(uuidgen | tr 'A-Z' 'a-z')
    echo "$runid|$host|$arm|$task|$sid" >> "$ROOT/manifest.txt"
    ( cd "$repo" && export KG_SESSION="$runid" KG_ARM="$runid" \
      && run_with_timeout "$KILL_SECS" "$GROK" --always-approve --max-turns 40 \
        --session-id "$sid" --prompt-file <(printf '%s\n' "$prompt") ) \
      > "$ROOT/logs/$runid.out" 2>&1
    rc=$?
  else
    sid="ses_util_${runid//-/_}"
    echo "$runid|$host|$arm|$task|$sid" >> "$ROOT/manifest.txt"
    ( cd "$repo" && export KG_SESSION="$runid" KG_ARM="$runid" \
      && run_with_timeout "$KILL_SECS" "$OC" run --auto \
        -m openai/gpt-6-sol --format json --session "$sid" "$prompt" ) \
      > "$ROOT/logs/$runid.out" 2>&1
    rc=$?
  fi
  [ -f "$repo/.kg/usage.log" ] && mv "$repo/.kg/usage.log" "$ROOT/logs/$runid.usage.log"
  [ -f "$repo/.kg/usage-results.jsonl" ] && mv "$repo/.kg/usage-results.jsonl" "$ROOT/logs/$runid.usage-results.jsonl"
  [ -f "/tmp/utility-answers-2026-11-02/$runid.json" ] && cp "/tmp/utility-answers-2026-11-02/$runid.json" "$ROOT/answers/"
  echo "[done] $runid exit=$rc $(date +%H:%M:%S)" | tee -a "$ROOT/status.txt"
}

run_order() {
  # ABBA per task: r1 kgc->kgp, r2 kgp->kgc. P2 first (smoke), then P1, P3.
  local host=$1
  for task in P2 P1 P3; do
    echo "$host kgc $task 1"
    echo "$host kgp $task 1"
    echo "$host kgp $task 2"
    echo "$host kgc $task 2"
  done
}

# Single-cell mode for smoke tests: run_stage1.sh --cell <host> <arm> <task> <run>
if [ "${1:-}" = "--cell" ]; then
  run_cell "$2" "$3" "$4" "$5"
  exit $?
fi

COUNT=0
while read -r host arm task run; do
  [ -z "$host" ] && continue
  [ "$COUNT" -ge "$MAX" ] && break
  runid="$arm-r$run-$task"
  if ! grep -q "^\[done\] $runid " "$ROOT/status.txt" 2>/dev/null; then
    COUNT=$((COUNT + 1))
    echo "=== cell $COUNT/$MAX: $host $runid ==="
    run_cell "$host" "$arm" "$task" "$run"
  fi
done < <(run_order grok; run_order opencode)

echo "=== runner done: $COUNT new cells ==="
