#!/usr/bin/env bash
# Tests for scripts/action-outputs.sh. Run: bash scripts/action-outputs.test.sh
set -euo pipefail

SCRIPT="$(cd "$(dirname "$0")" && pwd)/action-outputs.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
FAILS=0

# run MODE STDOUT_CONTENT -> sets $OUTPUTS (GITHUB_OUTPUT file) and $LOG
run() {
  printf '%s' "$2" >"$WORK/out"
  : >"$WORK/gh_output"
  if ! GITHUB_OUTPUT="$WORK/gh_output" bash "$SCRIPT" "$1" "$WORK/out" >"$WORK/log" 2>&1; then
    echo "FAIL: script exited nonzero"
    cat "$WORK/log"
    FAILS=$((FAILS + 1))
  fi
  OUTPUTS=$(cat "$WORK/gh_output")
  LOG=$(cat "$WORK/log")
}

expect_line() {
  if ! grep -qxF -- "$2" <<<"$OUTPUTS"; then
    echo "FAIL [$1]: expected output line: $2"
    echo "$OUTPUTS" | sed 's/^/    /'
    FAILS=$((FAILS + 1))
  fi
}

expect_log() {
  if ! grep -qF -- "$2" <<<"$LOG"; then
    echo "FAIL [$1]: expected log to contain: $2"
    echo "$LOG" | sed 's/^/    /'
    FAILS=$((FAILS + 1))
  fi
}

expect_no_log() {
  if grep -qF -- "$2" <<<"$LOG"; then
    echo "FAIL [$1]: log should not contain: $2"
    FAILS=$((FAILS + 1))
  fi
}

RELEASE_JSON='{"version":"1.2.3","previous_version":"1.2.2","tag":"v1.2.3","bump":"patch","floating_tag":"v1","commit_count":2}'

# 1. Clean JSON on stdout.
run release "$RELEASE_JSON"$'\n'
expect_line clean "version=1.2.3"
expect_line clean "tag=v1.2.3"
expect_line clean "previous_version=1.2.2"
expect_line clean "floating_tag=v1"
expect_line clean "commit_count=2"
expect_line clean "released=true"
expect_line clean "$RELEASE_JSON"
expect_no_log clean "::warning::"

# 2. npm publish noise ahead of the JSON (the reported failure).
run release "+ @scope/pkg@1.2.3"$'\n'"npm notice Tarball Contents"$'\n'"npm notice 1.2kB package.json"$'\n'"$RELEASE_JSON"$'\n'
expect_line noise "version=1.2.3"
expect_line noise "tag=v1.2.3"
expect_line noise "released=true"
expect_log noise "::warning::sr stdout had extra text"

# 3. Noise containing a JSON-looking line, then pretty-printed plan JSON.
PLAN_JSON=$'{\n  "branch": "main",\n  "version": "2.0.0",\n  "tag": "v2.0.0",\n  "bump": "major"\n}'
run plan $'{"not":"the result"}\nsome text\n'"$PLAN_JSON"$'\n'
expect_line pretty "version=2.0.0"
expect_line pretty "bump=major"
expect_line pretty "released=false"

# 4. Pretty-printed JSON alone parses strictly.
run plan "$PLAN_JSON"$'\n'
expect_line plan "tag=v2.0.0"
expect_no_log plan "::warning::"

# 5. Nothing parseable: warn with raw output, still succeed.
run release $'+ @scope/pkg@1.2.3\nnot json at all\n'
expect_line garbage "released=true"
expect_line garbage "version="
expect_log garbage "::warning::Could not parse JSON from sr stdout"
expect_log garbage "%0Anot json at all"

# 6. Empty stdout in prepare mode.
run prepare ""
expect_line empty "released=false"
expect_log empty "::warning::Could not parse JSON"

# 7. Two concatenated values: strict fails, last object wins.
run release '{"version":"0.0.1"}'$'\n'"$RELEASE_JSON"$'\n'
expect_line multi "version=1.2.3"

if [ "$FAILS" -ne 0 ]; then
  echo "$FAILS check(s) failed"
  exit 1
fi
echo "action-outputs: all checks passed"
