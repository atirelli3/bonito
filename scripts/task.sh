#!/usr/bin/env bash
# task.sh — the tuna board: `task.sh` lists open items, `task.sh <issue#> <status>` adds/moves one issue.
set -euo pipefail
owner=atirelli3
statuses='Todo|In Progress|In Review|Done'

num=$(gh project list --owner "$owner" --format json --jq '.projects[] | select(.title == "tuna") | .number')
[[ -n $num ]] || { echo "task.sh: no project 'tuna' for $owner" >&2; exit 1; }

if (( $# == 0 )); then
  gh project item-list "$num" --owner "$owner" --limit 500 --format json \
    --jq '.items[] | select(.status != "Done") | "\(.status // "-")\t#\(.content.number)\t\(.title)"' | sort
  exit 0
fi
(( $# == 2 )) && [[ $2 =~ ^($statuses)$ ]] || { echo "usage: task.sh [<issue#> <$statuses>]" >&2; exit 2; }

url=$(gh issue view "$1" --json url --jq .url)
item=$(gh project item-add "$num" --owner "$owner" --url "$url" --format json --jq .id)
project=$(gh project view "$num" --owner "$owner" --format json --jq .id)
field=$(gh project field-list "$num" --owner "$owner" --format json --jq '.fields[] | select(.name == "Status")')
option=$(jq -r --arg s "$2" '.options[] | select(.name == $s) | .id' <<<"$field")
[[ -n $option ]] || { echo "task.sh: board has no status '$2'" >&2; exit 1; }
gh project item-edit --id "$item" --project-id "$project" --field-id "$(jq -r .id <<<"$field")" \
  --single-select-option-id "$option" >/dev/null
echo "#$1 -> $2"
