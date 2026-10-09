#!/usr/bin/env bash
# Turn sr's stdout into GitHub Action step outputs.
#
# Usage: action-outputs.sh MODE OUT_FILE
#   MODE      plan | prepare | release
#   OUT_FILE  file holding sr's stdout from a run that exited 0
# Writes key=value lines to "$GITHUB_OUTPUT".
#
# sr prints exactly one JSON value on stdout. This script still tolerates
# other text ahead of it (for example output from an older sr, or from a
# tool that writes to /dev/tty): it never fails a run that sr reported as
# successful.
set -uo pipefail

MODE="$1"
OUT_FILE="$2"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT must be set}"

# Escape text for a workflow command message (%, CR, LF).
escape_cmd() {
  local s="$1"
  s="${s//'%'/'%25'}"
  s="${s//$'\r'/'%0D'}"
  s="${s//$'\n'/'%0A'}"
  printf '%s' "$s"
}

# Print the compact JSON object in OUT_FILE, or fail.
parse_strict() {
  local json
  json=$(jq -c . <"$OUT_FILE" 2>/dev/null) || return 1
  # Exactly one value, and it must be an object.
  [ -n "$json" ] && [ "$(printf '%s\n' "$json" | wc -l)" -eq 1 ] || return 1
  printf '%s' "$json" | jq -e 'type == "object"' >/dev/null 2>&1 || return 1
  printf '%s' "$json"
}

# Print the last JSON object in OUT_FILE that runs to end of file. The
# object starts on a line beginning with `{` (true for both compact and
# pretty-printed JSON). Try such lines from the bottom up.
parse_last_object() {
  local starts n json
  starts=$(grep -n '^{' "$OUT_FILE" | cut -d: -f1 | sort -rn) || return 1
  for n in $starts; do
    if json=$(tail -n "+$n" "$OUT_FILE" | jq -c . 2>/dev/null) &&
      [ "$(printf '%s\n' "$json" | wc -l)" -eq 1 ] &&
      printf '%s' "$json" | jq -e 'type == "object"' >/dev/null 2>&1; then
      printf '%s' "$json"
      return 0
    fi
  done
  return 1
}

released=false
[ "$MODE" = "release" ] && released=true

if JSON_OUTPUT=$(parse_strict); then
  :
elif JSON_OUTPUT=$(parse_last_object); then
  echo "::warning::sr stdout had extra text before its JSON result; it was ignored."
else
  echo "::warning::Could not parse JSON from sr stdout. sr exited 0, so outputs are set from the mode only. Raw output:%0A$(escape_cmd "$(cat "$OUT_FILE")")"
  {
    echo "json="
    echo "version="
    echo "previous_version="
    echo "tag="
    echo "bump="
    echo "floating_tag="
    echo "commit_count=0"
    echo "released=$released"
  } >>"$GITHUB_OUTPUT"
  exit 0
fi

field() { printf '%s' "$JSON_OUTPUT" | jq -r "$1"; }

{
  echo "json<<__SR_EOF__"
  echo "$JSON_OUTPUT"
  echo "__SR_EOF__"
  echo "version=$(field '.version // ""')"
  echo "previous_version=$(field '.previous_version // ""')"
  echo "tag=$(field '.tag // .tag_name // ""')"
  echo "bump=$(field '.bump // ""')"
  echo "floating_tag=$(field '.floating_tag // ""')"
  echo "commit_count=$(field '.commit_count // 0')"
  echo "released=$released"
} >>"$GITHUB_OUTPUT"
