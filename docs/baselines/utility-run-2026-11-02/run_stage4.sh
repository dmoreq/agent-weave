#!/usr/bin/env bash
# Stage 2 runner — search utility A/B. 24 cells: S1-S3 x 2 arms x 2 hosts x 2 runs.
set -uo pipefail
KG=/Users/quy.doan/Workspace/kg/target/release/kg
GROK=/Users/quy.doan/.grok/bin/grok
OC=/Users/quy.doan/.opencode/bin/opencode
ROOT=/Users/quy.doan/Workspace/kg/docs/baselines/utility-run-2026-11-02
STATUS="$ROOT/status4.txt"; MANIFEST="$ROOT/manifest4.txt"
MAX="${1:-99}"; KILL_SECS=480

repo_for() { case "$1" in D1|D2|D3) echo /tmp/utility-corpus-2026-11-02/scale;; *) echo "";; esac; }

run_with_timeout() { local secs=$1; shift; "$@" & local pid=$!; ( sleep "$secs"; kill -9 "$pid" 2>/dev/null ) & local wpid=$!; wait "$pid"; local rc=$?; kill "$wpid" 2>/dev/null; return "$rc"; }

run_cell() {
  local host=$1 arm=$2 task=$3 run=$4
  local hosttag="$host"; [ "$host" = "opencode" ] && hosttag="oc"
  local runid="$hosttag-$arm-r$run-$task"
  local repo; repo=$(repo_for "$task")
  grep -q "^\[done\] $runid " "$STATUS" 2>/dev/null && { echo "[skip] $runid"; return 0; }
  env -u KG_ARM -u KG_SESSION "$KG" build --root "$repo" >/dev/null 2>&1
  rm -f "$repo/.kg/usage.log"* "$repo/.kg/usage-results.jsonl"*
  local prompt; prompt=$(sed -e "s|{ANSWER_PATH}|/tmp/utility-answers-2026-11-02/$runid.json|g" "$ROOT/prompts/$arm-$task.txt")
  echo "[start] $runid host=$host repo=$repo $(date +%H:%M:%S)" | tee -a "$STATUS"
  local sid rc
  if [ "$host" = grok ]; then
    sid=$(uuidgen | tr 'A-Z' 'a-z'); echo "$runid|$host|$arm|$task|$sid" >> "$MANIFEST"
    ( cd "$repo" && export KG_SESSION="$runid" KG_ARM="$runid" \
      && run_with_timeout "$KILL_SECS" "$GROK" --always-approve --max-turns 40 \
        --session-id "$sid" --prompt-file <(printf '%s\n' "$prompt") ) > "$ROOT/logs/$runid.out" 2>&1
    rc=$?
  else
    sid="ses_util_${runid//-/_}"; echo "$runid|$host|$arm|$task|$sid" >> "$MANIFEST"
    ( cd "$repo" && export KG_SESSION="$runid" KG_ARM="$runid" \
      && run_with_timeout "$KILL_SECS" "$OC" run --auto \
        -m openai/gpt-6-sol --format json --session "$sid" "$prompt" ) > "$ROOT/logs/$runid.out" 2>&1
    rc=$?
  fi
  [ -f "$repo/.kg/usage.log" ] && mv "$repo/.kg/usage.log" "$ROOT/logs/$runid.usage.log"
  [ -f "$repo/.kg/usage-results.jsonl" ] && mv "$repo/.kg/usage-results.jsonl" "$ROOT/logs/$runid.usage-results.jsonl"
  [ -f "/tmp/utility-answers-2026-11-02/$runid.json" ] && cp "/tmp/utility-answers-2026-11-02/$runid.json" "$ROOT/answers/"
  echo "[done] $runid exit=$rc $(date +%H:%M:%S)" | tee -a "$STATUS"
}

if [ "${1:-}" = "--cell" ]; then run_cell "$2" "$3" "$4" "$5"; exit $?; fi

run_order() { local host=$1; for task in D2 D1 D3; do echo "$host kgc $task 1"; echo "$host kgp $task 1"; echo "$host kgp $task 2"; echo "$host kgc $task 2"; done; }
COUNT=0
while read -r host arm task run; do
  [ -z "$host" ] && continue
  [ "$COUNT" -ge "$MAX" ] && break
  runid="$arm-r$run-$task"; [ "$host" = "opencode" ] && runid="oc-$runid"; [ "$host" = "grok" ] && runid="grok-$runid"
  if ! grep -q "^\[done\] $runid " "$STATUS" 2>/dev/null; then COUNT=$((COUNT+1)); echo "=== cell $COUNT/$MAX: $host $runid ==="; run_cell "$host" "$arm" "$task" "$run"; fi
done < <(run_order grok; run_order opencode)
echo "=== stage2 runner done: $COUNT new cells ==="
