# PR Agent decision record

Scope: the review bot added in `.github/workflows/pr-agent.yml` and
`.pr_agent.toml`. Mode: **Full** (network, credentials, multiple components).

Everything below is tagged:

- `[VERIFIED: source]` — read from an official doc, shipped config, or source.
- `[MEASURED: how]` — observed in a run on 2026-11-07.
- `[UNVERIFIABLE]` — no source; treat as an open risk.

## Components

GitHub Actions workflow, pr-agent 0.49.0 (Docker action), LiteLLM (inside
pr-agent), Command Code Provider API, model `deepseek/deepseek-v4.1-flash`,
repository secret, `.pr_agent.toml` policy.

## Done ledger

| # | Knob | Decision | Why | Source |
|---|---|---|---|---|
| D1 | Deployment | GitHub Action on GitHub's runner | Zero infra; the repo is public, so minutes are free; no server to expose | `[VERIFIED: pr-agent README v0.49.0, "GitHub Action (Recommended)"]` |
| D2 | API surface | `https://api.commandcode.ai/provider/v1` | Official OpenAI-compatible surface; replaces the earlier translation bridge | `[VERIFIED: https://commandcode.ai/docs/provider, Endpoints table]` |
| D3 | Auth | `Authorization: Bearer`, key in `COMMAND_CODE_API_KEY` | The documented scheme; API access needs a plan other than Go | `[VERIFIED: Provider API Quickstart and Errors table]` |
| D4 | Model | `openai/deepseek/deepseek-v4.1-flash` | User directive; served on `/chat/completions` | `[VERIFIED: GET /provider/v1/models, supported_endpoints=["/chat/completions","/responses"], context_length=1000000]` |
| D5 | Reasoning | `reasoning_effort = "max"` | User requires the highest effort | `[VERIFIED: CLI model registry, reasoningEfforts=["off","low","high","max"]]`; `[MEASURED: max 498-509s / 70,483 reasoning tokens; low 118-417s; off 22s]` |
| D6 | Reasoning passthrough | `additional_reasoning_effort_models = ["openai/deepseek/deepseek-v4.1-flash"]` | LiteLLM does not know the id; pr-agent only forwards the effort when listed | `[VERIFIED: configuration.toml v0.49.0 line 125; litellm_ai_handler.py reasoning gate]` |
| D7 | Context window | `custom_model_max_tokens = 1000000` | pr-agent refuses an unknown id without a declared window | `[MEASURED: "Model ... is not defined in MAX_TOKENS ... set config.custom_model_max_tokens"]`; `[VERIFIED: live models list context_length=1000000]` |
| D8 | Input cap | `max_model_tokens = 256000` | Input and output share this budget; 256k leaves 128k of input after the 128k output cap. The measured prompt is ~14k | `[VERIFIED: configuration.toml line 50 meaning]`; `[MEASURED: "leaves no input capacity" at 64k/64k]` |
| D9 | Output cap | `max_output_tokens = 128000` | Max reasoning consumed 70,483 tokens; a 64k cap returned empty content with `finish_reason=length`, 128k finished | `[VERIFIED: configuration.toml line 60]`; `[MEASURED: 64k empty, 128k success]` |
| D10 | Request timeout | `ai_timeout = 900`, `retry_same_model_on_timeout = false` | Measured max review: 400-509s. Worst case is primary + one fallback entry (same model) = 2 x 900s = 30 min, under the 45-minute job timeout. Same-model replay on timeout would make it 4 attempts = 60 min | `[VERIFIED: configuration.toml line 27 default 120; _should_retry_same_model; retry_with_fallback_models]`; `[MEASURED: 400s on PR #19, 498-509s locally]` |
| D11 | Fallback | Same model only | Failures retry the chosen model; no silent switch to another vendor | `[VERIFIED: configuration.toml fallback_models]` |
| D12 | Auto tools | `/describe` and `/review` on open; `/improve` manual | Chosen by the user; lowest quota, no committable suggestions | `[VERIFIED: github_action_config.auto_* defaults in github_action_runner.py]` |
| D13 | Failure semantics | A recorded tool failure fails the check | No green check without a review | `[VERIFIED: github_action_runner.py _fail_on_recorded_tool_error, default true]` |
| D14 | Repo rules | Read `AGENTS.md`, `NEXT.md`, `context.md` from `main` | Reviewers must judge by the repo's safety and honesty rules, and a PR cannot rewrite them | `[VERIFIED: configuration.toml repo_context_files and repo_context_from_default_branch]` |
| D15 | Triggers | `pull_request` opened/reopened/ready_for_review, `issue_comment` created | Standard review-bot surface | `[VERIFIED: pr-agent GitHub Action template and runner event handling]` |
| D16 | Fork PRs | Skipped | Secrets are not passed to fork PR workflows | `[VERIFIED: GitHub Docs, "Using secrets in GitHub Actions"]` |
| D17 | Comment auth | `author_association` in OWNER/MEMBER/COLLABORATOR and body starts with `/` | Prevents anyone spending credits with a comment | `[VERIFIED: upstream pr-agent-review.yaml `if` condition]` |
| D18 | Permissions | `contents: read`, `pull-requests: write`, `issues: write` | Enough to read the PR and post comments; no write to code | `[VERIFIED: pr-agent action usage and the tools enabled]` |
| D19 | Checkout | None | The runner reads PR data and settings through the API; upstream's own workflow uses the action without checkout | `[VERIFIED: upstream pr-agent-review.yaml steps; github_provider.get_repo_settings uses the API]` |
| D20 | Image pin | `pragent/pr-agent@sha256:b1ed6e8d…` | Immutable; matches tag `0.49.0-github_action` | `[VERIFIED: Docker Hub tag API, manifest digest, 2026-10-25]` |
| D21 | Concurrency | Per PR/issue, no cancel | A skipped command must not cancel a queued review | `[VERIFIED: upstream pr-agent-review.yaml comment]` |
| D22 | Policy file | `.pr_agent.toml` with repo-specific blocking rules | Carries the pacman invariants from AGENTS.md and context.md into every review | `[VERIFIED: repo files and configuration.toml key names]` |
| D23 | Finding shape | `inline_key_issues = true` | Findings as file:line comments an agent can act on, not prose buried in a summary | `[VERIFIED: configuration.toml line 200]` |
| D24 | Verdict fields | `require_risk_assessment = true`, `require_merge_recommendation = true` | Typed risk level and merge verdict, so an agent gets a decision without parsing prose | `[VERIFIED: configuration.toml lines 183-184]` |
| D25 | Large PRs | `enable_large_pr_chunking = true`, `max_number_of_calls = 3` | Chunk instead of skip or truncate. Chunks beyond the cap land in `remaining_files_list`, and the coverage footer lists them; the footer stays enabled so a partial review is never silent | `[VERIFIED: configuration.toml lines 216-217; pr_processing.py _pack_pr_multi_diffs; pr_reviewer.py coverage footer]` |
| D26 | AX contract | `AGENTS.md` section "Agent review surface" | An agent entering the repo learns triggers, commands, output fields, authority, and failure semantics without reading the workflow | `[VERIFIED: AGENTS.md]` |
| D27 | Output trim | `require_estimate_effort_to_review = false`, `require_ticket_analysis_review = false`, `enable_intro_text = false` | Drop rows that only help human triage; keep findings, tests, security, risk, and verdict | `[VERIFIED: configuration.toml lines 176, 181, 211]` |
| D28 | Agent state CLI | `scripts/review-state.sh` | Reads the `pr-agent-review-state:v1` typed block: head staleness, coverage, finding counts, `path:line`; TOON-ish output, exit codes, `--help`, `--version` | `[VERIFIED: review comment state block; script tests]` |

## Not-done ledger

| # | Item | Why not | Source |
|---|---|---|---|
| N1 | Docker webhook / GitHub App deployment | Needs an always-on public endpoint; conflicts with the no-exposure constraint | `[VERIFIED: pr-agent Docker deployment docs]` |
| N2 | Local proxy or tunnel | Prohibited by the user; also unnecessary once the official Provider API exists | `[VERIFIED: user instruction; Provider API docs]` |
| N3 | Fork PR review | `pull_request` gives no secrets; `pull_request_target` with PR code is a pwn-request risk | `[VERIFIED: GitHub Docs secrets note; repo ci.yml comment F15]` |
| N4 | `/update_changelog`, `/add_docs`, committable suggestions | Would need `contents: write`; out of the requested scope | `[VERIFIED: permission surface above]` |
| N5 | Branch protection requiring `Review` | Repo setting, not a workflow change; requires a maintainer decision | `[VERIFIED: GitHub branch protection docs]` |
| N6 | Ollama or another local model | Needs an always-on host; user chose Command Code | `[VERIFIED: user directive]` |
| N7 | Auto-merge or auto-approve | Review is advisory; approval stays human | `[VERIFIED: enable_auto_approval default false]` |
| N8 | `workflow_run` CI-conclusion trigger | Doubles runs; the chosen trigger set matches the user's "describe + review on open" choice | `[VERIFIED: github_action_runner.py workflow_run branch]` |
| N9 | Review on every push (`synchronize`) | User: per-PR review is enough | `[VERIFIED: user instruction]` |
| N10 | Path filters for `target/**`, `Cargo.lock` | User: those paths are reviewed too | `[VERIFIED: user instruction]` |
| N11 | Per-directory `.pr_agent.toml` rules | Not requested; one global policy is enough | `[VERIFIED: user instruction]` |
| N12 | TOON for the bot's own PR comment | The comment format is pr-agent's; the state command emits the compact form instead | `[VERIFIED: pr_reviewer prompts and renderer]` |

## Coverage

| Component | Knobs covered | Knobs deliberately defaulted |
|---|---|---|
| Workflow | triggers, permissions, concurrency, guards, timeout, image pin, env | runner choice, retry of the job itself |
| pr-agent | model, fallback, custom_model_max_tokens, max_model_tokens, max_output_tokens, reasoning_effort, additional_reasoning_effort_models, ai_timeout, repo context, auto tools, response language | temperatures, seeds, callbacks, labels, score/effort sections |
| Provider API | base URL, endpoint, auth, model, reasoning effort | ZDR header, Responses API, Anthropic Messages |
| Secrets | name, scope, fork policy | environment-scoped secrets |
| Policy | reviewer/description/suggestion instructions, ignore rules via repo context | per-directory settings, global org settings |
| Everything else in `configuration.toml` | `[N/A]` — webhooks, callbacks/telemetry, vector DB, chat integrations, publishing cosmetics, OpenRouter controls, per-directory/global settings | Not reachable from this deployment |

Coverage script: `coverage_diff.py` against `configuration.toml` reported many
`GAP` rows for the `[N/A]` group above and `SINGLE-SOURCE` because only one
machine-readable source was supplied. Both are recorded here, not hidden. The
Provider API facts were checked against the live endpoint instead of a file.

## Agent experience (AX)

The surface is built agent-first: a coding agent reads the PR comment and acts;
a human reads it second. The contract lives in `AGENTS.md` under "Agent review
surface".

| Role | How it is served |
|---|---|
| Reader | `.github/PR-AGENT.md` is the reference; policy loads from `main`; repo context bounded at 900 lines |
| Developer | Inline file:line findings, typed risk and merge verdict, one persistent comment, deterministic `/review` and `/describe` commands |
| End user (agent) | Machine-readable state over prose; merge stays a human action; CI and Sim remain the gates |

Invariants applied:

- Typed contracts over prose: `require_risk_assessment`,
  `require_merge_recommendation`, `inline_key_issues`, and the
  `pr-agent-review-state:v1` block the bot embeds in its comment.
- State command: `scripts/review-state.sh` turns that block into compact state:
  head staleness, coverage, finding counts, and `path:line` rows.
- Recovery-shaped failures: a failed tool fails the check and posts a failure
  comment; it never turns green.
- Machine-readable state: one persistent comment with stable keys, not a thread
  an agent has to reconstruct.
- Trust boundary closed by default: rules and config from `main`, fork PRs
  skipped, commands limited to OWNER, MEMBER, COLLABORATOR.
- Prose guides, gates guarantee: the bot advises; CI and Sim enforce.

## Verification

| Check | Result |
|---|---|
| YAML parse, TOML parse | pass |
| Live models list | `deepseek/deepseek-v4.1-flash`, 1,000,000 context, `/chat/completions` |
| Direct API, small prompt, reasoning high/low/off | content returned, `finish_reason=stop` |
| Exact review prompt, reasoning low, 64k cap | 118s, 14,540 completion tokens, valid YAML |
| Exact review prompt, reasoning max, 64k cap | failed: 64,000 reasoning tokens, empty content, `finish_reason=length` |
| Exact review prompt, reasoning max, 128k cap | 498s, 70,483 reasoning tokens, valid YAML |
| pr-agent review, reasoning low, 32k cap | failed: empty content, `finish_reason=length` |
| pr-agent review, reasoning low, 64k cap, 64k model budget | failed: no input capacity |
| pr-agent review, reasoning low, 64k cap, 128k model budget | 238s, review generated, not published |
| pr-agent `doctor` | 3/3 (config, key, completion) |
| pr-agent `describe` on PR #12 | generated, not published |
| pr-agent `review` on PR #12, reasoning off | completed in 22s |
| Env spelling `CONFIG__MODEL`, `CONFIG__REASONING_EFFORT`, `CONFIG__ADDITIONAL_REASONING_EFFORT_MODELS`, `GITHUB_ACTION_CONFIG__AUTO_IMPROVE`, `OPENAI__API_BASE` | all read by `get_settings()`; same spelling as upstream's own workflow |
| Full review with the exact workflow env, dunder names, reasoning max | 509s, review generated, not published |
| New AX fields (`inline_key_issues`, risk, merge verdict, chunking) | keys verified in configuration.toml; first live run is the next PR after merge |
| Workflow on PR #19 | success, 400s, all 6 checks passed, review comment posted |
| `scripts/review-state.sh` on PR #20 | 1 finding, stale true, coverage full, `path:line` table |
| State command empty / usage / error paths | 0-PR message exit 0; unknown flag exit 2; missing PR exit 1 |

## Open items

1. Reasoning latency: max measured 400s on PR #19 and 498-509s locally on the
   same diff. The retry budget is 2 x 900s = 30 min, under the 45-minute job
   timeout. A larger diff can still time out; the check fails loudly. Watch for
   `finish_reason=length` in the Action log. `[UNVERIFIABLE]` without a larger PR.
2. Docker Hub anonymous pull limits on GitHub runners. `[UNVERIFIABLE]`; a limit
   fails the job loudly.
3. Provider API access is plan-gated (every plan except Go). The key used for
   local tests and PR #19 worked; the CI key must come from a plan with API
   access. `[VERIFIED: Provider API docs]` + `[MEASURED: live calls succeeded]`.

## Rollback

Delete `.github/workflows/pr-agent.yml`, `.pr_agent.toml`, and
`.github/PR-AGENT.md`; remove the `COMMAND_CODE_API_KEY` secret. No runtime
state, no deployed service, no data to clean up.
