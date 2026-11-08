#!/usr/bin/env bash
# Agent-facing review state. AXI: typed contract, compact output, minimal
# schema, pre-computed aggregates, definitive empty state, structured errors.
set -uo pipefail

VERSION="0.2.0"
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$SELF_DIR/$(basename "${BASH_SOURCE[0]}")"
case "$BIN" in "$HOME"/*) BIN="~${BIN#"$HOME"}" ;; esac

usage() {
  cat <<EOF
bin: $BIN
description: Compact PR review state for agents

usage:
  scripts/review-state.sh [<pr>] [--findings] [--full]
  scripts/review-state.sh --help
  scripts/review-state.sh --version

flags:
  --findings  list findings as a TOON table
  --full      include the complete review comment (collapsed to one line)
  --help      this reference
  --version   print the version only

examples:
  scripts/review-state.sh
  scripts/review-state.sh 20
  scripts/review-state.sh 20 --findings
  scripts/review-state.sh 20 --full
EOF
}

err() {
  printf 'error: %s\nhelp: %s\n' "$1" "$2"
  exit "${3:-1}"
}

pr=""
full=0
show_findings=0

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    --version|-v|-V) printf '%s\n' "$VERSION"; exit 0 ;;
    --full) full=1 ;;
    --findings) show_findings=1 ;;
    -*) err "unknown flag $arg" "valid flags: --findings, --full, --help, --version" 2 ;;
    *)
      [[ -z "$pr" ]] || err "unexpected argument $arg" "pass one PR number" 2
      pr="$arg"
      ;;
  esac
done

command -v gh >/dev/null 2>&1 || err "gh is not installed" "install the GitHub CLI"
command -v jq >/dev/null 2>&1 || err "jq is not installed" "install jq"

GERR="$(mktemp)"
trap 'rm -f "$GERR"' EXIT

if [[ -z "$pr" ]]; then
  pr="$(gh pr view --json number --jq .number 2>"$GERR")"
  if [[ -z "$pr" ]]; then
    if grep -qi "no pull requests found" "$GERR"; then
      branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
      printf 'pr: 0 open pull requests for branch %s\n' "$branch"
      printf 'help[1]: push the branch and open a PR to get a review\n'
      exit 0
    fi
    err "gh pr view failed: $(head -1 "$GERR")" "run \`gh auth status\`"
  fi
else
  [[ "$pr" =~ ^[0-9]+$ ]] || err "PR must be a number, got $pr" "run \`gh pr list\` to see numbers" 2
fi

if ! meta="$(gh pr view "$pr" --json number,state,title,headRefOid,url 2>"$GERR")"; then
  err "PR #$pr not found: $(head -1 "$GERR")" "check the number or run \`gh pr list\`"
fi

number="$(jq -r '.number' <<<"$meta")"
state="$(jq -r '.state' <<<"$meta")"
title="$(jq -r '.title' <<<"$meta")"
head="$(jq -r '.headRefOid[0:7]' <<<"$meta")"
head_full="$(jq -r '.headRefOid' <<<"$meta")"
url="$(jq -r '.url' <<<"$meta")"

checks="$(gh pr checks "$pr" --json name,state 2>"$GERR")"
if [[ -z "$checks" ]]; then
  if grep -qi "no checks reported" "$GERR"; then
    checks='[]'
  else
    err "gh pr checks failed: $(head -1 "$GERR")" "run \`gh auth status\`"
  fi
fi
jq -e . >/dev/null 2>&1 <<<"$checks" || err "gh pr checks returned invalid JSON" "rerun the command"

total="$(jq 'length' <<<"$checks")"
passed="$(jq '[.[] | select(.state == "SUCCESS")] | length' <<<"$checks")"
failed="$(jq '[.[] | select(.state == "FAILURE" or .state == "CANCELLED" or .state == "TIMED_OUT" or .state == "ERROR" or .state == "ACTION_REQUIRED" or .state == "STARTUP_FAILURE" or .state == "STALE")] | length' <<<"$checks")"
pending="$(jq '[.[] | select(.state == "PENDING" or .state == "IN_PROGRESS" or .state == "QUEUED" or .state == "WAITING" or .state == "REQUESTED" or .state == "EXPECTED")] | length' <<<"$checks")"
skipped="$(jq '[.[] | select(.state == "SKIPPED" or .state == "NEUTRAL")] | length' <<<"$checks")"

if ! review="$(gh pr view "$pr" --json comments \
  --jq '[.comments[] | select(.body | test("pr-agent-review-state|Risk level|Merge recommendation|PR Reviewer Guide"))] | last // {}' \
  2>"$GERR")"; then
  err "gh pr view comments failed: $(head -1 "$GERR")" "run \`gh auth status\`"
fi
[[ -n "$review" ]] || review='{}'

review_body="$(jq -r '.body // ""' <<<"$review" 2>/dev/null || echo "")"
review_url="$(jq -r '.url // ""' <<<"$review" 2>/dev/null || echo "")"

# Typed state block pr-agent embeds in the review comment (multi-line HTML comment).
state_json="$(printf '%s\n' "$review_body" | awk '/pr-agent-review-state:v1/{f=1;next} f&&/^-->/{exit} f')"
if [[ -n "$state_json" ]] && jq -e . >/dev/null 2>&1 <<<"$state_json"; then
  complete="$(jq -r '.last_run.complete // false' <<<"$state_json")"
  excluded="$(jq -r '(.last_run.excluded_files // []) | length' <<<"$state_json")"
  findings_total="$(jq -r '(.findings // []) | length' <<<"$state_json")"
  findings_active="$(jq -r '[.findings[]? | select(.state == "ACTIVE")] | length' <<<"$state_json")"
  review_head="$(jq -r '.last_run.head_sha // ""' <<<"$state_json")"
  if [[ "$complete" == "true" && "$excluded" -eq 0 ]]; then
    coverage="full"
  else
    coverage="partial"
  fi
else
  state_json=""
  findings_total="0"
  findings_active="0"
  review_head=""
  if [[ -z "$review_body" ]]; then
    coverage="unknown"
  elif grep -q "Review coverage" <<<"$review_body"; then
    coverage="partial"
  else
    coverage="full"
  fi
fi

if [[ -n "$review_head" ]]; then
  [[ "$review_head" == "$head_full" ]] && stale="false" || stale="true"
else
  stale="unknown"
fi

risk="$(sed -n 's/.*<strong>Risk level<\/strong>:[[:space:]]*\([^<]*\).*/\1/p' <<<"$review_body" | head -1 | tr '[:upper:]' '[:lower:]' | xargs)"
verdict="$(sed -n 's/.*<strong>Merge recommendation<\/strong>:[[:space:]]*\([^<]*\).*/\1/p' <<<"$review_body" | head -1 | tr '[:upper:]' '[:lower:]' | tr ' ' '_' | xargs)"

[[ -n "$risk" ]] || risk="unknown"
[[ -n "$verdict" ]] || verdict="unknown"
[[ -n "$review_body" ]] && present="true" || present="false"

printf 'bin: %s\n' "$BIN"
printf 'description: Compact PR review state for agents\n'
printf 'pr:\n'
printf '  number: %s\n' "$number"
printf '  state: %s\n' "$state"
printf '  title: %s\n' "$title"
printf '  head: %s\n' "$head"
printf '  url: %s\n' "$url"
printf 'checks:\n'
printf '  total: %s\n' "$total"
printf '  passed: %s\n' "$passed"
printf '  failed: %s\n' "$failed"
printf '  pending: %s\n' "$pending"
printf '  skipped: %s\n' "$skipped"
printf 'review:\n'
printf '  present: %s\n' "$present"
printf '  stale: %s\n' "$stale"
printf '  risk: %s\n' "$risk"
printf '  verdict: %s\n' "$verdict"
printf '  coverage: %s\n' "$coverage"
printf '  findings: %s\n' "$findings_total"
printf '  active: %s\n' "$findings_active"
if [[ -n "$review_url" ]]; then
  printf '  url: %s\n' "$review_url"
else
  printf '  url: null\n'
fi

if [[ "$show_findings" -eq 1 ]]; then
  if [[ -z "$state_json" ]]; then
    printf 'findings: unknown; the review has no typed state block\n'
  elif [[ "$findings_total" -eq 0 ]]; then
    printf 'findings: 0 findings in this review\n'
  else
    printf 'findings[%s]{id,path,line,state}:\n' "$findings_total"
    jq -r '.findings[]? | [.finding_id, .path, (.line_start|tostring), .state] | join(",")' <<<"$state_json"
  fi
fi

if [[ "$full" -eq 1 && -n "$review_body" ]]; then
  chars="${#review_body}"
  limit=20000
  if [[ "$chars" -gt "$limit" ]]; then
    body="$(tr '\n' ' ' <<<"${review_body:0:$limit}") ... (truncated, $chars chars total)"
  else
    body="$(tr '\n' ' ' <<<"$review_body")"
  fi
  printf 'comment:\n'
  printf '  chars: %s\n' "$chars"
  printf '  body: %s\n' "$body"
fi

if [[ "$present" != "true" ]]; then
  printf 'help[2]:\n'
  printf '  No review comment yet; it is posted when the PR opens\n'
  printf '  Run `gh pr view %s --web` to open the PR\n' "$number"
elif [[ "$stale" == "true" ]]; then
  printf 'help[2]:\n'
  printf '  Review is stale for head %s; run `gh pr comment %s --body "/review"` to refresh\n' "$head" "$number"
  printf '  Run `scripts/review-state.sh %s --findings` to list findings\n' "$number"
elif [[ "$coverage" == "partial" ]]; then
  printf 'help[2]:\n'
  printf '  Coverage is partial; open the PR and read the coverage footer\n'
  printf '  Run `scripts/review-state.sh %s --findings` to list findings\n' "$number"
else
  printf 'help[2]:\n'
  printf '  Run `scripts/review-state.sh %s --findings` to list findings\n' "$number"
  printf '  Run `gh pr view %s --web` to open the PR\n' "$number"
fi
