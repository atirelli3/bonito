#!/usr/bin/env bash
# tests-only.sh — PreToolUse guard of the tests subagent: only the approved test commands run.
cmd=$(jq -r '.tool_input.command')
re='^(bash scripts/gate\.sh|cargo test --locked( [A-Za-z0-9_:]+)?)$'
[[ $cmd =~ $re ]] && exit 0
echo "tests: not an approved test command: $cmd" >&2
exit 2
