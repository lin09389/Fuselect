# Fuselect CLI Implementation Plan

**Goal:** Build an Apache-2.0 Rust CLI that provides Codex CLI with a local, authenticated endpoint implementing the core observable behavior of OpenRouter Fusion: an outer model can decide to invoke a bounded multi-model deliberation tool, receive structured Judge analysis, and then produce the sole final response or tool call.

**Architecture:** Fuselect runs a loopback-only HTTP gateway and a CLI control plane. Codex calls `fuselect/auto` or `fuselect/fusion` through the OpenAI Responses API. Fuselect chooses an outer Worker for `auto`, then exposes an internal `fuselect:fusion` server tool to that Worker. When invoked, Fusion runs a configurable panel of read-only advisors in parallel, gives their in-memory results to a Judge, and returns structured analysis to the same outer Worker. The outer Worker—not the gateway, advisor, or Judge—continues the original turn and emits the only client-visible response and Codex tool calls.

**Tech Stack:** Rust stable, Tokio, Axum, Reqwest, Clap, Serde, SQLx + SQLite, OS keyring (`keyring` crate), `toml_edit`, `tracing`, Apache-2.0, GitHub Actions.

---

## Confirmed product constraints

- Target platforms: Windows x64 and Linux ARM64; ship a single CLI binary for each.
- No native graphical UI in v1. The CLI starts the gateway in the foreground for MVP; the optional TUI and user-scoped background service are follow-on tasks in this plan.
- External clients: Codex CLI first; expose both `/v1/responses` and `/v1/chat/completions` for future clients.
- Upstream Workers: at most 10, manually configured in the CLI; every v1 Worker must support OpenAI Chat Completions, SSE, `tools`, and streamed tool calls.
- API secrets never leave the device. SQLite stores only non-secret metadata; Worker keys live in the operating-system credential store.
- Gateway binds to `127.0.0.1` only and requires a generated Gateway Key. `fuselect codex setup` creates a separate Codex profile and backs up `~/.codex/config.toml`; it must not overwrite the user's default provider.
- Public Fusion entry points are `fuselect/auto` (outer Worker may invoke Fusion when useful), `fuselect/fusion` (Fusion required for the turn), and the `fuselect:fusion` server tool (explicit integration surface for future non-Codex clients).
- Policy: quality first, then minimize cost. Both per-task and daily hard budgets are enforced by the gateway.
- Fusion depth is exactly one. The caller may select a named preset or explicitly choose 1–8 advisors and a Judge; otherwise Fuselect uses a local cost/quality policy. Advisors and Judge receive no Codex tools. The outer Worker alone retains original client tools and can make the final response. Nested Fusion is rejected.
- Fusion supports request-scoped `enabled`, `force`, `preset`, `analysis_models`, `judge_model`, `max_completion_tokens`, `reasoning`, and `temperature` controls. Presets start with `coding-high` and `coding-budget`.
- Persist only metadata: route decisions, participating Worker IDs, aggregate token/cost data, latency, outcome, and error category. Raw prompts, responses, code, tool arguments, and tool outputs stay in memory and are discarded after the request.
- The public conceptual server tool is `fuselect:fusion`; its upstream Chat Completions wire name is the collision-resistant function `fuselect__fusion`. It is injected only by Fuselect and is never forwarded to Codex.
- A turn may invoke Fusion once. A model response that combines `fuselect__fusion` with caller-owned tool calls is rejected as an invalid mixed tool turn; the model must first receive the Fusion result, then decide whether to call a Codex tool.
- The first outer-model response is held in an in-memory protocol state machine until it has terminally completed or requested Fusion. This prevents leaking internal tool-call events to Codex. Only the final direct/continuation response is serialized as public SSE.
- Upstream retries are allowed only before a response attempt has been committed to the public client. No retry may duplicate a caller tool call or replay an emitted public stream.
- License: Apache-2.0.

## Protocol boundary

Codex custom Providers should be configured with `wire_api = "responses"`. Fuselect must implement the portion of OpenAI Responses needed by Codex, then translate to a provider-neutral internal request and to upstream `/v1/chat/completions` requests. Never rely on an omitted `wire_api` default.

```text
Codex -- Responses/SSE --> Fuselect loopback gateway
                              |
                              +-- choose outer Worker / inject fuselect:fusion tool
                              |
                              +-- outer Worker Chat Completions/SSE
                                      |                         |
                                      | direct                  '-- buffered until public-safe, then sole client-visible response/tool calls
                                      |
                                      '-- fuselect:fusion call -> advisors (parallel, no Codex tools)
                                                             -> Judge (structured JSON, no Codex tools)
                                                             -> tool result returned to outer Worker
```

## CLI interaction contract

Fuselect is a terminal-first control plane. Every mutating command supports both explicit flags (automation-friendly) and an interactive wizard when required values are absent and stdin is a TTY. It must never prompt in a non-interactive shell; instead it exits with a precise validation error and an example command.

### Command surface

```text
fuselect init                         # create local state, Keyring Gateway Key, and defaults
fuselect worker add|list|show|remove|test
fuselect fusion preset add|list|show|remove
fuselect gateway start [--port 8787] [--verbose]
fuselect tui                          # optional full-screen local control console
fuselect gateway rotate-key
fuselect gateway-token                # prints only the Gateway Key for Codex auth.command
fuselect codex setup|status|rollback
fuselect doctor
fuselect privacy                         # show local data and configured upstream data egress
fuselect config validate
fuselect backup create|list|restore
fuselect status [--json]
fuselect logs list [--since ...] [--json]
```

`worker add` wizard prompts in this order: display name, Base URL, model ID, input/output/cache token prices, capability tags, then API Key. It validates the URL and prices before storing anything. The API Key is accepted through hidden input, written only to the OS Keyring, and never echoed, printed, or placed in shell history. `worker test` must be an explicit network action; it reports a compact capability matrix for chat, streaming, ordinary function tools, and streamed tool arguments.

`fusion preset add` wizard selects an outer model policy, one to eight advisor Workers, a Judge, maximum output tokens, and task/daily budget limits. It shows a worst-case cost estimate and requires confirmation before saving a preset that can exceed the current task budget.

`gateway start` prints one stable, non-secret status line (`listening`, URL, active profile name, and PID), then metadata-only request events in `--verbose` mode. It must not print the Gateway Key, prompt text, upstream payloads, or Worker API Keys. `gateway-token` prints exactly one token plus a newline and writes no diagnostics to stdout; errors go to stderr.

`codex setup` presents the planned `config.toml` diff, creates a timestamped backup, asks for confirmation unless `--yes` is supplied, writes the provider and profile using `toml_edit`, and verifies the token command plus `GET /health`. It never overwrites the default Codex provider. `codex rollback` lists backup IDs and requires explicit confirmation.

### Output, errors, and exit codes

- Human output is concise, Chinese-first, and carries a next command where useful; `--json` emits stable machine-readable objects without ANSI color.
- Secrets are redacted as `***`; request IDs and Worker IDs are safe to display.
- Exit `0` means completed; `2` means user/configuration validation error; `3` means Keyring/authentication failure; `4` means gateway/provider health failure; `5` means budget denial; `6` means an upstream/Fusion execution failure.
- Every destructive or connectivity-affecting mutation (`worker remove`, `preset remove`, `codex rollback`, network probe) requires confirmation in TTY mode and `--yes` in non-interactive automation.

### First-run path

```text
fuselect init
  → fuselect worker add
  → fuselect fusion preset add
  → fuselect gateway start
  → fuselect codex setup
  → codex -p fuselect
```

The first release has no chat interface. `fuselect tui` is the optional full-screen control console introduced after the core gateway compatibility suite; Codex remains the only coding-task interaction surface. The TUI is a convenient view and wizard layer over the same command/domain services: it must not create a second configuration format, a second routing engine, or a separate secret store.

## Delivery milestones

The numbered tasks are deliberately granular. These milestones make the dependency order visible and prevent an early demo build from being mistaken for a stable release.

| Milestone | Includes | Evidence of completion | Release label |
| --- | --- | --- | --- |
| M0 — foundation | Tasks 1–6 | A local authenticated gateway accepts validated requests, with no secrets in SQLite. | internal prototype |
| M1 — correct Coding/Fusion path | Tasks 7–14 and 22 | Codex-style multi-turn and compaction fixtures pass for direct and Fusion tool calls. | private alpha |
| M2 — daily local operation | Tasks 16, 17, and 21 | TUI/CLI workflows, lifecycle limits, recovery, and per-user service behavior pass on both target platforms. | beta |
| M3 — trustworthy public project | Tasks 18–20 and 23 | Security, evaluation, compatibility, performance, documentation, and reproducible-release gates have current evidence. | stable |

Tasks in a later milestone do not block local learning or M0/M1 testing; they do block a “stable” public release claim.

## Task 1: Establish the repository and supply-chain baseline

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `LICENSE`
- Create: `README.md`
- Create: `.gitignore`
- Create: `rustfmt.toml`
- Create: `.github/workflows/ci.yml`

**Step 1: Create the failing smoke test.**

Create `tests/cli_smoke.rs`:

```rust
#[test]
fn help_lists_gateway_subcommand() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fuselect"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("gateway"));
}
```

**Step 2: Run it to verify it fails.**

Run: `cargo test --test cli_smoke`

Expected: FAIL because the package and binary do not exist.

**Step 3: Add the minimal crate.**

Use package name `fuselect`, edition `2024`, license `Apache-2.0`, and a `clap` command with `init`, `gateway`, `worker`, `fusion`, `codex`, `status`, and `logs` subcommands. Add Apache-2.0 text unchanged to `LICENSE`.

**Step 4: Run formatting and tests.**

Run: `cargo fmt --check; cargo test --test cli_smoke`

Expected: PASS.

**Step 5: Add CI.**

CI must run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all-targets` on Windows and Ubuntu. Use only released Rust stable.

**Step 6: Commit.**

```bash
git add Cargo.toml src LICENSE README.md .gitignore rustfmt.toml .github tests
git commit -m "chore: bootstrap Fuselect CLI"
```

### Task 2: Define configuration, domain types, and validation

**Files:**
- Create: `src/config/mod.rs`
- Create: `src/config/types.rs`
- Create: `src/domain/mod.rs`
- Create: `src/domain/worker.rs`
- Create: `src/domain/budget.rs`
- Test: `tests/config_validation.rs`

**Step 1: Write failing validation tests.**

```rust
#[test]
fn rejects_an_eleventh_worker() {
    let config = test_config_with_workers(11);
    assert!(config.validate().is_err());
}

#[test]
fn requires_tools_and_streaming_for_every_worker() {
    let worker = worker_without("tools");
    assert!(worker.validate().is_err());
}
```

**Step 2: Run the test.**

Run: `cargo test --test config_validation`

Expected: FAIL because the domain types do not exist.

**Step 3: Implement the smallest validated types.**

Define versioned `WorkerConfig`, `WorkerCapabilities`, `Pricing`, `BudgetLimits`, `FusionPolicy`, and `GatewayConfig`. `FusionPolicy` is local configuration for deterministic quality-first/cost-second outer-Worker selection and preset defaults; there is no cloud routing/orchestrator model or `OrchestratorConfig` in this architecture. The fixed capability enum is `coding`, `reasoning`, `review`, `debug`, `long_context`, `tools`, `fast`, `low_cost`. Require `tools` and streaming support; enforce `MAX_WORKERS: usize = 10`; reject a non-HTTPS upstream URL except loopback test servers. Every persisted config record has an explicit schema version and migrations are forward-only, transactional, idempotent, and tested from each supported prior version.

**Step 4: Verify.**

Run: `cargo test --test config_validation`

Expected: PASS.

**Step 5: Commit.**

```bash
git add src/config src/domain tests/config_validation.rs
git commit -m "feat: validate Fuselect worker configuration"
```

### Task 3: Add SQLite metadata storage and OS key references

**Files:**
- Create: `src/storage/mod.rs`
- Create: `src/storage/sqlite.rs`
- Create: `src/secrets/mod.rs`
- Create: `migrations/0001_initial.sql`
- Test: `tests/storage_metadata.rs`

**Step 1: Write failing persistence tests.**

```rust
#[tokio::test]
async fn persists_worker_metadata_without_api_key() {
    let store = test_store().await;
    store.save_worker(&worker("coder-a")).await.unwrap();
    assert_eq!(store.list_workers().await.unwrap()[0].id, "coder-a");
}
```

**Step 2: Run it.**

Run: `cargo test --test storage_metadata`

Expected: FAIL because the store is absent.

**Step 3: Implement storage.**

Create tables for `workers`, `fusion_presets`, `gateway`, `request_runs`, `run_participants`, `run_stages`, and `daily_spend`. Store Worker metadata, price rates, capabilities, health state, and a keyring entry name—not an API key. `run_stages` records only stage type, Worker ID, timing, token counts, known cost, attempt number, and error category. Use the OS keyring service name `fuselect`; use `FUSELECT_HOME` when supplied, otherwise the platform config directory.

**Step 4: Verify migration and metadata behavior.**

Run: `cargo test --test storage_metadata`

Expected: PASS.

**Step 5: Commit.**

```bash
git add src/storage src/secrets migrations tests/storage_metadata.rs
git commit -m "feat: persist Fuselect metadata without secrets"
```

### Task 4: Implement Worker and Fusion-policy configuration commands

**Files:**
- Create: `src/commands/worker.rs`
- Create: `src/commands/fusion.rs`
- Modify: `src/main.rs`
- Test: `tests/commands_config.rs`

**Step 1: Write a failing CLI test for a Worker.**

```rust
#[test]
fn worker_add_requires_price_and_capabilities() {
    let output = fuselect(["worker", "add", "--id", "coder-a"]);
    assert!(!output.status.success());
}
```

**Step 2: Run it.**

Run: `cargo test --test commands_config`

Expected: FAIL.

**Step 3: Implement commands.**

Implement `init`; `worker add`, `worker list`, `worker show`, `worker remove`, and `worker test`; and `fusion preset add`, `fusion preset list`, `fusion preset show`, and `fusion preset remove`. Follow the CLI interaction contract: interactive wizards only on a TTY, hidden Keyring-only secret input, confirmation for mutation/network actions, `--yes` for automation, Chinese-first errors, and `--json` where output is inspectable. `worker add` accepts manual ID, display name, Base URL, model ID, input/output/cache prices, capabilities, a non-secret provider terms/privacy URL, and Keyring secret input. Presets define an outer Worker selection policy, 1–8 advisor IDs, a Judge ID, limits, and a quality/cost tier. Ship `coding-high` and `coding-budget`. Redact all secrets in output.

**Step 4: Add a compatibility probe.**

`worker test` calls `/v1/chat/completions` against a configured mock server with `stream: true` and a benign tool schema; it records an explicit pass/fail capability status. Do not admit Workers that fail this test to `enabled` state.

**Step 5: Verify and commit.**

Run: `cargo test --test commands_config`

Expected: PASS.

```bash
git add src/commands src/main.rs tests/commands_config.rs
git commit -m "feat: configure Workers and Fusion presets from CLI"
```

### Task 5: Define the normalized request/response model

**Files:**
- Create: `src/protocol/mod.rs`
- Create: `src/protocol/normalized.rs`
- Create: `src/protocol/chat_completions.rs`
- Create: `src/protocol/responses.rs`
- Create: `src/protocol/tools.rs`
- Test: `tests/protocol_normalization.rs`

**Step 1: Write failing normalization tests.**

```rust
#[test]
fn retains_tools_when_normalizing_a_responses_request() {
    let request = fixture_responses_request_with_function_tool();
    let normalized = NormalizedRequest::from_responses(request).unwrap();
    assert_eq!(normalized.tools.len(), 1);
}
```

**Step 2: Run it.**

Run: `cargo test --test protocol_normalization`

Expected: FAIL.

**Step 3: Implement protocol types.**

Normalize messages/input, model, tools, tool choice, stream, temperature, token caps, metadata, request ID, `previous_response_id`, `plugins`, and server-tool configuration. Implement explicit codecs for `/v1/responses` and `/v1/chat/completions`. Preserve ordinary function tools, namespace-scoped function tools, custom/freeform tools, `tool_search`, and their tool-result message links in a typed internal representation; a freeform tool must round-trip as raw string input rather than be silently coerced to JSON. Parse Fuselect-specific Fusion fields without sending them upstream; reject unsupported media, tool variants, or response options with clear 400 errors rather than silently dropping fields.

Add fixtures for split, duplicate, snapshot-style, and missing-final tool argument deltas, plus a mixed text-then-tool response. These are protocol contracts, not provider-specific behavior.

**Step 4: Verify and commit.**

Run: `cargo test --test protocol_normalization`

Expected: PASS.

```bash
git add src/protocol tests/protocol_normalization.rs
git commit -m "feat: normalize Responses and Chat Completions requests"
```

### Task 6: Build the loopback gateway, authentication, and health endpoint

**Files:**
- Create: `src/gateway/mod.rs`
- Create: `src/gateway/server.rs`
- Create: `src/gateway/auth.rs`
- Create: `src/commands/gateway.rs`
- Test: `tests/gateway_auth.rs`

**Step 1: Write failing gateway tests.**

```rust
#[tokio::test]
async fn rejects_a_request_without_the_gateway_bearer_token() {
    let app = test_gateway().await;
    let response = call(app, "/v1/models", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

**Step 2: Run them.**

Run: `cargo test --test gateway_auth`

Expected: FAIL.

**Step 3: Implement the minimal gateway.**

`fuselect gateway start` binds only to `127.0.0.1`, creates a cryptographically random Gateway Key in Keyring if absent, and serves authenticated `GET /health`, `GET /v1/models`, `POST /v1/responses`, `POST /v1/responses/compact`, and `POST /v1/chat/completions`. `/v1/models` returns `fuselect/auto` and `fuselect/fusion`. Never log Authorization headers. Keep secrets in `SecretString`/zeroizing buffers when feasible, mark outbound authorization headers sensitive, cap request-body size, and reject every route not explicitly served.

**Step 4: Verify and commit.**

Run: `cargo test --test gateway_auth`

Expected: PASS.

```bash
git add src/gateway src/commands/gateway.rs tests/gateway_auth.rs
git commit -m "feat: add authenticated loopback gateway"
```

### Task 7: Add deterministic budget accounting and request metadata logging

**Files:**
- Create: `src/routing/mod.rs`
- Create: `src/routing/budget.rs`
- Create: `src/observability/mod.rs`
- Test: `tests/budget_enforcement.rs`

**Step 1: Write failing budget tests.**

```rust
#[tokio::test]
async fn rejects_a_route_that_exceeds_the_per_task_cap() {
    let budget = BudgetTracker::with_limits(usd("0.10"), usd("5.00"));
    assert!(budget.reserve(usd("0.11")).await.is_err());
}
```

**Step 2: Run them.**

Run: `cargo test --test budget_enforcement`

Expected: FAIL.

**Step 3: Implement reservation before dispatch.**

Reserve the worst-case configured price before every dispatch: initial outer call, full advisor panel, Judge, and outer continuation after Fusion. A reservation must include configured maximum output tokens; if no bounded maximum exists, reject the route instead of inventing a safe estimate. Snapshot the price configuration/version used by each reservation. Enforce per-task and UTC daily caps atomically, detect material backward wall-clock movement conservatively, and reconcile each stage from provider `usage`; flag `cost_unknown` when usage is absent, rather than fabricating zero cost. On startup, expire/reconcile orphaned reservations left by a crash before admitting new work. Persist only metadata and hashed request correlation IDs, never request or response bodies.

**Step 4: Verify and commit.**

Run: `cargo test --test budget_enforcement`

Expected: PASS.

```bash
git add src/routing src/observability tests/budget_enforcement.rs
git commit -m "feat: enforce task and daily Fusion budgets"
```

### Task 8: Implement Fusion request configuration and outer-Worker selection

**Files:**
- Create: `src/fusion/config.rs`
- Create: `src/routing/outer_worker.rs`
- Test: `tests/fusion_config.rs`

**Step 1: Write failing decision parsing tests.**

```rust
#[test]
fn rejects_a_fusion_request_with_nine_advisors() {
    assert!(FusionConfig::with_advisors(ids(9)).validate().is_err());
}
```

**Step 2: Run them.**

Run: `cargo test --test fusion_config`

Expected: FAIL.

**Step 3: Implement configuration and selection.**

Parse a `fusion` plugin object and `fuselect:fusion` tool parameters from normalized public requests. Validate `enabled`, `force`, preset name, 1–8 enabled advisor IDs, enabled Judge ID, `max_completion_tokens`, reasoning settings, and temperature. Explicit model lists override a preset. `fuselect/fusion` sets `force = true`; `fuselect/auto` defaults to `enabled = true, force = false`. Select the outer Worker using deterministic local quality-first/cost-second policy among enabled Workers with `coding`, `reasoning`, and `tools`; do not send prompts, API keys, or hidden configuration to another routing model.

For `auto`, inject `fuselect__fusion` with `tool_choice = auto`. For forced Fusion, require that exact function by name, rather than using a generic “some tool is required” setting that could select a caller-owned tool. Reserve `fuselect__fusion` and reject a caller tool with that name. The injected tool schema accepts only validated Fusion overrides and never an arbitrary prompt or arbitrary model URL.

**Step 4: Verify and commit.**

Run: `cargo test --test fusion_config`

Expected: PASS.

```bash
git add src/fusion/config.rs src/routing/outer_worker.rs tests/fusion_config.rs
git commit -m "feat: configure Fusion and select the outer Worker"
```

### Task 9: Implement direct Worker execution and streamed Chat Completions handling

**Files:**
- Create: `src/workers/mod.rs`
- Create: `src/workers/client.rs`
- Create: `src/workers/profile.rs`
- Create: `src/workers/sse.rs`
- Test: `tests/direct_streaming.rs`

**Step 1: Write a failing SSE forwarding test.**

```rust
#[tokio::test]
async fn forwards_streamed_tool_call_deltas() {
    let events = collect_events(mock_worker_tool_stream()).await;
    assert!(events.iter().any(|event| event.contains("tool_calls")));
}
```

**Step 2: Run it.**

Run: `cargo test --test direct_streaming`

Expected: FAIL.

**Step 3: Implement Worker invocation.**

Convert `NormalizedRequest` to Chat Completions and pass messages, tools, tool choice, stream, and supported generation parameters. Maintain a per-request in-memory SSE state machine keyed by output item/call ID; it must append genuine deltas, deduplicate repeated starts, recognize snapshot argument updates, and use a completed argument payload when a provider omits deltas. Apply per-attempt connect/idle/overall timeouts, classify provider errors, and record no response content. A retry is allowed only before public output is committed.

Use a versioned, explicit Worker compatibility profile selected by the admission probe. The baseline profile is strict OpenAI Chat Completions; a non-baseline profile may normalize only documented request/SSE variations with a sanitized regression fixture. Profiles are declarative capability/codec choices, never arbitrary user scripts. Before dispatch, reject a request that requires a capability the selected profile cannot prove instead of silently omitting fields. Adding or changing a profile requires compatibility-matrix documentation and direct, Fusion, streamed-tool, malformed-stream, and error-fixture coverage.

**Step 4: Verify and commit.**

Run: `cargo test --test direct_streaming`

Expected: PASS.

```bash
git add src/workers tests/direct_streaming.rs
git commit -m "feat: stream direct Worker responses"
```

### Task 10: Implement the Fusion server-tool loop

**Files:**
- Create: `src/fusion/mod.rs`
- Create: `src/fusion/advisors.rs`
- Create: `src/fusion/judge.rs`
- Create: `src/fusion/tool_loop.rs`
- Create: `src/fusion/receipt.rs`
- Test: `tests/fusion_execution.rs`

**Step 1: Write failing Fusion safety tests.**

```rust
#[tokio::test]
async fn advisors_receive_no_client_tools() {
    let captured = fusion_with_capture_worker().await;
    assert!(captured.advisor_request.tools.is_empty());
}

#[tokio::test]
async fn outer_worker_receives_judge_analysis_as_a_tool_result() {
    let captured = fusion_with_capture_worker().await;
    assert!(captured.outer_follow_up.tool_result.contains("consensus"));
    assert_eq!(captured.outer_worker_requests_with_original_tools, 2);
}

#[tokio::test]
async fn does_not_emit_or_replay_an_internal_fusion_tool_call_to_the_client() {
    let result = fusion_with_capture_worker().await;
    assert!(!result.public_events.iter().any(|event| event.contains("fuselect__fusion")));
}
```

**Step 2: Run them.**

Run: `cargo test --test fusion_execution`

Expected: FAIL.

**Step 3: Implement Fusion.**

Inject `fuselect__fusion` as a function-like server tool into the outer Worker's request alongside original caller tools. Buffer and parse the initial outer Worker response before public serialization. If it calls Fusion, require exactly one internal Fusion call and no caller-owned sibling tool calls; retain the assistant tool-call message, run Fusion, append a tool-result message with the same call ID, then request the continuation from the same outer Worker. If it does not call Fusion, serialize the buffered response only after its terminal event.

Run advisor requests concurrently using `FuturesUnordered` or `join_all`; remove client tools and tool choice, add a read-only advisor prompt, and keep responses in memory. Send advisor outputs to the Judge, which must emit validated JSON with `consensus`, `contradictions`, `partial_coverage`, `unique_insights`, `blind_spots`, and `execution_recommendation`. Return that JSON as the Fusion tool result to the same outer Worker. The outer Worker alone receives the original Codex tools and produces the final stream.

Allow degraded execution when at least one advisor succeeds, recording each failed advisor in a non-persisted Fusion receipt. Return typed tool errors—`all_panels_failed`, `insufficient_budget`, `rate_limited`, `fusion_invocation_capped`, `invalid_mixed_tool_turn`, or `unexpected_error`—so the outer Worker can answer without Fusion analysis. Set `fusion_depth = 1` request context marker and reject nested attempts. Record stage-level model ID, latency, token usage, known cost, and outcome; discard all panel/Judge text once the request ends.

**Step 4: Verify and commit.**

Run: `cargo test --test fusion_execution`

Expected: PASS.

```bash
git add src/fusion tests/fusion_execution.rs
git commit -m "feat: add bounded Fusion server-tool execution"
```

### Task 11: Translate normalized streaming output back to both public APIs

**Files:**
- Modify: `src/protocol/responses.rs`
- Modify: `src/protocol/chat_completions.rs`
- Modify: `src/gateway/server.rs`
- Test: `tests/responses_sse.rs`
- Test: `tests/chat_completions_sse.rs`

**Step 1: Write failing contract tests.**

```rust
#[tokio::test]
async fn responses_endpoint_emits_completed_event_after_tool_call_deltas() {
    let events = post_responses_and_collect_sse().await;
    assert_eq!(events.last().unwrap().event_name(), "response.completed");
}
```

**Step 2: Run them.**

Run: `cargo test --test responses_sse --test chat_completions_sse`

Expected: FAIL.

**Step 3: Implement output codecs.**

Map the normalized stream to the explicit event families needed by Codex's Responses API and to OpenAI Chat Completions chunks for future clients. Preserve caller tool call IDs, names, ordering, incremental arguments, finish reasons, usage when available, and error terminal events. The internal `fuselect__fusion` call and tool result must never be emitted to Codex; it is consumed entirely inside the outer Worker loop. Never emit upstream internal model names to Codex; report the requested public virtual model while storing participants only in local metadata.

Use the same state machine for direct and final-continuation output. Include contract fixtures for text-to-tool transitions, duplicate `output_item.added`, split/snapshot/missing-final argument deltas, failed SSE tails, and parallel caller tool calls. A failed internal upstream attempt becomes a typed terminal public error only when no safe continuation exists.

**Step 4: Verify and commit.**

Run: `cargo test --test responses_sse --test chat_completions_sse`

Expected: PASS.

```bash
git add src/protocol src/gateway tests/responses_sse.rs tests/chat_completions_sse.rs
git commit -m "feat: expose streamed Responses and Chat Completions APIs"
```

### Task 12: Add Codex profile setup with backup and rollback

**Files:**
- Create: `src/codex/mod.rs`
- Create: `src/codex/config.rs`
- Create: `src/commands/codex.rs`
- Test: `tests/codex_setup.rs`

**Step 1: Write a failing non-destructive configuration test.**

```rust
#[test]
fn setup_preserves_the_existing_default_model_provider() {
    let before = fixture_codex_config();
    let after = add_fuselect_profile(before.clone(), setup_data()).unwrap();
    assert!(after.contains("model_provider = \"openai\""));
    assert!(after.contains("[profiles.fuselect]"));
}
```

**Step 2: Run it.**

Run: `cargo test --test codex_setup`

Expected: FAIL.

**Step 3: Implement setup.**

`fuselect codex setup` creates/reads the Gateway Key in Keyring, backs up the user-level `~/.codex/config.toml`, and edits it using `toml_edit`. Add a non-reserved custom provider pointing to `http://127.0.0.1:<port>/v1`, use `wire_api = "responses"`, disable WebSockets, and authenticate through Codex's command-backed provider `auth` field. The configured command is the absolute Fuselect binary path with the argument `gateway-token`; it prints only the current Gateway Key from Keyring to stdout, has a short timeout, and no diagnostic output on stdout. Create profile `fuselect` with `model = "fuselect/auto"`. Implement `fuselect codex rollback <backup-id>` and test that neither the key nor the token-command output becomes part of TOML, logs, or backup content.

**Step 4: Verify and commit.**

Run: `cargo test --test codex_setup`

Expected: PASS.

```bash
git add src/codex src/commands/codex.rs tests/codex_setup.rs
git commit -m "feat: safely configure Codex for Fuselect"
```

### Task 13: Add status, metadata-only logs, and health commands

**Files:**
- Create: `src/commands/status.rs`
- Create: `src/commands/logs.rs`
- Modify: `src/commands/gateway.rs`
- Test: `tests/observability_commands.rs`

**Step 1: Write failing privacy tests.**

```rust
#[tokio::test]
async fn logs_never_contain_the_prompt_text() {
    let output = run_logs_after_request("TOP_SECRET_CODE").await;
    assert!(!output.contains("TOP_SECRET_CODE"));
}
```

**Step 2: Run it.**

Run: `cargo test --test observability_commands`

Expected: FAIL.

**Step 3: Implement observability commands.**

`fuselect status` reports gateway state, configured Workers, probe health, remaining daily budget, and current version. `fuselect logs list` shows only request ID, mode, Worker IDs, decision reason, latency, token totals, known cost, and outcome. Both commands implement stable `--json` output. `fuselect privacy` is snapshot-tested: it may name configured provider hosts and user-supplied policy links, but cannot contain API Keys, Gateway Keys, prompt fragments, raw tool/result content, or upstream URL credential/query components. `fuselect gateway start --verbose` may stream the same metadata but must not print payload content; `gateway-token` writes only its token and newline to stdout.

**Step 4: Verify and commit.**

Run: `cargo test --test observability_commands`

Expected: PASS.

```bash
git add src/commands tests/observability_commands.rs
git commit -m "feat: add privacy-preserving status and logs"
```

### Task 14: Write end-to-end compatibility fixtures and manual acceptance tests

**Files:**
- Create: `tests/e2e/mod.rs`
- Create: `tests/e2e/mock_provider.rs`
- Create: `tests/e2e/codex_responses.rs`
- Create: `docs/compatibility.md`
- Create: `docs/security.md`
- Modify: `README.md`

**Step 1: Add an end-to-end failing test.**

```rust
#[tokio::test]
async fn a_codex_style_responses_request_can_complete_a_tool_call_via_fusion() {
    let result = run_fixture_gateway_with_fusion().await;
    assert_eq!(result.public_model, "fuselect/auto");
    assert!(result.tool_call_completed);
}
```

**Step 2: Run it.**

Run: `cargo test --test e2e`

Expected: FAIL until all gateway pieces integrate.

**Step 3: Implement fixtures and document acceptance.**

Use local mock upstreams only. Cover: direct outer response, virtual `previous_response_id` continuation, caller tool-result continuation, unary/v2 compaction, context-limit routing/denial, internal Fusion call interception, forced Fusion selecting the exact internal tool, disabled Fusion, 1-advisor and 8-advisor configurations, advisor failure, Judge failure, typed Fusion tool errors, invalid mixed internal/client tool turns, outer Worker Codex tool call after Fusion, missing `usage`, bounded-cost rejection, per-task budget denial, daily budget denial, invalid Gateway Key, recursion rejection, duplicate/split/snapshot/missing-final argument deltas, failed SSE tails, and response stream termination. Assert that public events never contain `fuselect__fusion`, provider API keys, panel output, Judge output, upstream response IDs, or raw conversation state. Document the Provider admission checklist and exact Codex profile usage command:

```bash
fuselect gateway start
codex -p fuselect
```

**Step 4: Run the full suite.**

Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --all-targets`

Expected: all PASS.

**Step 5: Commit.**

```bash
git add tests docs README.md
git commit -m "test: verify Codex Fusion gateway end to end"
```

### Task 15: Package for Windows and Linux ARM64

**Files:**
- Create: `.github/workflows/release.yml`
- Create: `docs/install.md`
- Modify: `README.md`

**Step 1: Write a release artifact assertion.**

Add a CI script or workflow check that verifies artifact names exactly match:

```text
fuselect-windows-x86_64.zip
fuselect-linux-aarch64.tar.gz
```

**Step 2: Run the workflow validation locally where possible.**

Run: `cargo build --release`

Expected: a `target/release/fuselect` or `fuselect.exe` binary on the host.

**Step 3: Implement release automation.**

Use GitHub Actions matrix runners/cross compilation appropriate for the target. Generate SHA-256 checksums. Never package local config, SQLite databases, API keys, or test fixtures containing real credentials.

**Step 4: Verify and commit.**

Run: `cargo test --all-targets`

Expected: PASS.

```bash
git add .github/workflows/release.yml docs/install.md README.md
git commit -m "chore: package Fuselect for Windows and Linux ARM64"
```

### Task 16: Add the optional terminal control console

**Files:**
- Create: `src/tui/mod.rs`
- Create: `src/tui/app.rs`
- Create: `src/tui/event.rs`
- Create: `src/tui/views/dashboard.rs`
- Create: `src/tui/views/workers.rs`
- Create: `src/tui/views/presets.rs`
- Create: `src/tui/views/runs.rs`
- Create: `src/tui/views/codex.rs`
- Create: `src/tui/views/help.rs`
- Create: `src/tui/widgets/confirm.rs`
- Create: `src/tui/widgets/form.rs`
- Modify: `src/main.rs`
- Test: `tests/tui_reducer.rs`
- Test: `tests/tui_render.rs`

**Step 1: Write failing state tests.**

```rust
#[test]
fn delete_worker_requires_a_confirmation_modal() {
    let app = AppState::with_selected_worker("coder-a");
    let next = reduce(app, UiEvent::DeleteSelected);
    assert!(matches!(next.modal, Some(Modal::ConfirmDeleteWorker { .. })));
}

#[test]
fn api_keys_are_never_rendered() {
    let screen = render_test_frame(AppState::with_worker_secret("TOP_SECRET"));
    assert!(!screen.contains("TOP_SECRET"));
}
```

**Step 2: Run them.**

Run: `cargo test --test tui_reducer --test tui_render`

Expected: FAIL.

**Step 3: Implement the TUI.**

Implement `fuselect tui` using Ratatui with its Crossterm backend, which supports Windows and Linux terminals. It calls the same application services as the CLI commands, so command behavior and TUI behavior share validation, Keyring access, budget checks, audit metadata, and confirmation semantics. Run the gateway as a Tokio task in the same process only when the user selects “start gateway”; the existing `gateway start` command remains available for headless operation. If another Fuselect gateway already owns the port, the TUI shows its authenticated health/readiness state as external and never attempts to stop or reconfigure that process.

The screen has five keyboard-first tabs:

```text
Overview   gateway state, loopback URL, daily remaining budget, recent route outcomes
Workers    add/edit/test/enable/remove Workers; no API Key is ever shown after entry
Fusion     create/edit presets, select 1–8 advisors and Judge, show worst-case cost before save
Runs       metadata-only live run list: route, stages, latency, tokens, known cost, result
Codex      profile/setup status, setup and rollback actions, copy-safe next command
```

The empty-state path is deliberately useful for a first-time user: `Overview → Add Worker → Test Worker → Add Fusion preset → Start gateway → Set up Codex`. Each form validates fields inline before moving forward and presents a final cost/impact summary. API Key entry uses a masked, one-time input widget that writes directly to Keyring and immediately zeroizes its local buffer; editing a Worker can replace a key but can never retrieve or display one. Runs remain strictly metadata-only, including in detail panes and copied text.

Use `Tab`/`Shift-Tab` for tab navigation, arrows or `j`/`k` for lists, `Enter` to open a detail/action, `a` to add, `e` to edit, `t` to probe a Worker, `d` to request delete, `r` to refresh, `?` for key help, and `q` to quit. `Esc` closes a modal without mutation; every destructive, network, or configuration-writing action uses a focused confirmation modal. The screen must degrade gracefully below a minimum terminal size by rendering a one-line resize notice instead of panicking. Mouse input is optional enhancement only; the complete workflow must remain keyboard accessible.

Use a reducer-style `AppState + UiEvent` core so navigation, modals, validation, and refresh results have unit tests independent of the terminal. Use Ratatui's test backend/snapshots for layout rendering. Add keyboard-flow tests for the first-run path, `Esc` cancellation, focus trapping in confirmations, external-gateway detection, and a terminal-resize frame. Enter alternate-screen/raw mode only after all startup validation succeeds; restore terminal state via RAII and a panic hook. Never put prompts, provider requests, provider responses, Gateway Keys, or Worker API Keys into the application state used for rendering.

**Step 4: Verify and commit.**

Run: `cargo test --test tui_reducer --test tui_render; cargo test --all-targets`

Expected: PASS.

```bash
git add src/tui src/main.rs tests/tui_reducer.rs tests/tui_render.rs
git commit -m "feat: add Fuselect terminal control console"
```

### Task 17: Production hardening, resilience, and lifecycle management

**Files:**
- Create: `src/gateway/lifecycle.rs`
- Create: `src/gateway/limits.rs`
- Create: `src/workers/health.rs`
- Create: `src/workers/retry.rs`
- Create: `src/commands/doctor.rs`
- Create: `docs/operations.md`
- Create: `docs/troubleshooting.md`
- Test: `tests/gateway_resilience.rs`
- Test: `tests/worker_health.rs`

**Step 1: Write failing resilience tests.**

```rust
#[tokio::test]
async fn cancels_all_in_flight_fusion_stages_when_the_client_disconnects() {
    let result = disconnect_during_fusion().await;
    assert!(result.all_stage_tasks_cancelled);
}

#[tokio::test]
async fn opens_a_circuit_after_repeated_retryable_worker_failures() {
    let health = fail_worker_repeatedly().await;
    assert_eq!(health.state, WorkerHealth::OpenCircuit);
}
```

**Step 2: Implement operational boundaries.**

Add a single-instance lock, graceful Ctrl+C/shutdown, bounded request-body size, concurrent-request semaphore, per-request deadline, stage cancellation propagation, and bounded in-memory buffers. Add retry with exponential backoff and jitter only for idempotent/pre-public-output upstream stages; honor `Retry-After`, never retry a caller tool call after public commitment, and add a circuit breaker with half-open health probes. Expose unauthenticated loopback-only `GET /health/live` and authenticated `GET /health/ready`; readiness fails when database migration, Keyring, orphaned-budget reconciliation, or required enabled Worker health is unavailable. Add configurable metadata-retention and `fuselect logs prune`, with a privacy-preserving default retention period, batched deletion, SQLite checkpoint/VACUUM maintenance, and a dry-run mode.

Implement `fuselect doctor` to verify version, config schema, SQLite migration state, Keyring access, writable data directory, gateway reachability, Codex profile/token command, and enabled Worker probes. Give every failure a concrete repair command. Add `fuselect privacy` to report, without reading or revealing payload content, which data remains local, which configured Worker endpoints can receive request/tool context, the current metadata retention period, durable-session opt-in state, and that Fuselect has no remote telemetry by default. It must distinguish a local capability probe from a real coding request and link to the applicable provider terms/privacy documentation stored by the user with each Worker configuration.

**Step 3: Verify and commit.**

Run: `cargo test --test gateway_resilience --test worker_health; cargo test --all-targets`

Expected: PASS.

```bash
git add src/gateway src/workers src/commands/doctor.rs docs tests
git commit -m "feat: harden gateway lifecycle and Worker resilience"
```

### Task 18: Security and supply-chain release gate

**Files:**
- Create: `SECURITY.md`
- Create: `deny.toml`
- Create: `.github/workflows/security.yml`
- Create: `docs/threat-model.md`
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/sse_parser.rs`
- Create: `fuzz/fuzz_targets/responses_codec.rs`
- Modify: `README.md`
- Test: `tests/security_boundaries.rs`

**Step 1: Document the threat model.**

Cover a malicious local process, a malicious or compromised upstream Provider, prompt-injected tool output, stolen local configuration, dependency compromise, and a malicious Codex config edit. Define the trust boundary: Fuselect does not execute upstream model instructions, it only proxies model-selected Codex tools; advisors and Judge never receive those tools. Explicitly state that loopback is not automatically trusted, model output is untrusted data, same-user malware cannot be fully contained by a user-space gateway, and binding beyond loopback/adding remote sync requires a new threat-model review.

**Step 2: Implement and test controls.**

Add secret rotation (`fuselect gateway rotate-key`), restrictive data-directory permissions where supported, SQLite integrity checks/backups, constant-time Gateway Key comparison, request rate/concurrency limits, redaction regression tests, and validation that the Codex token command path is the installed Fuselect binary rather than a shell string. Add negative tests proving that an internal Fusion call never becomes a Codex tool call, mixed internal/client tool turns are rejected, malformed streams cannot cause duplicate public tool calls, and no retry occurs after public commitment.

Add deterministic property tests for request normalization, tool-call aggregation, and public-event serialization: arbitrary valid internal streams must serialize to protocol-valid terminal sequences, and arbitrary malformed/oversized fragments must return a typed bounded error without panicking, leaking internal content, or emitting a caller tool call. Add `cargo-fuzz` targets for SSE parsing and Responses codec decoding, with a checked-in seed corpus containing historical sanitized failures. Run a bounded fuzz smoke job on every Linux security workflow and a longer corpus/regression job on a scheduled workflow; fuzz failures become minimized regression fixtures. Add `cargo audit`, `cargo deny` licenses/advisories/bans, locked dependency builds, SBOM generation, and signed release provenance/checksums in CI.

**Step 3: Verify and commit.**

Run: `cargo test --test security_boundaries; cargo test --all-targets; cargo audit; cargo deny check; cargo +nightly fuzz run sse_parser -- -max_total_time=60`

Expected: PASS with reviewed, documented exceptions only.

```bash
git add SECURITY.md deny.toml .github docs tests README.md
git commit -m "chore: add Fuselect security and supply-chain gate"
```

### Task 19: Evaluation harness and cost-quality calibration

**Files:**
- Create: `src/eval/mod.rs`
- Create: `src/commands/eval.rs`
- Create: `evals/cases/*.json`
- Create: `evals/rubrics/*.md`
- Create: `docs/evaluation.md`
- Test: `tests/eval_runner.rs`

**Step 1: Define reproducible evaluation cases.**

Create a versioned, non-secret suite covering direct coding changes, tool selection, test-failure diagnosis, multi-file refactors, long-context planning, malformed provider streams, and Fusion-required reasoning. Each case has a deterministic mock-provider fixture plus a rubric for optional live-provider runs.

**Step 2: Implement `fuselect eval`.**

Run the same case through a baseline direct Worker, `coding-budget`, and `coding-high`; report success/failure, latency, known/unknown cost, Fusion activation rate, and cost per successful case. Support `--offline` for deterministic mocks and explicit `--live` plus confirmation for paid calls. Live evaluation stores only aggregates and case IDs, never prompts or responses.

**Step 3: Establish release policy.**

No default preset may replace the previous release's preset unless it is non-regressing on protocol/tool correctness and either improves task success or lowers cost per success on the versioned suite. Publish the result table with each release rather than claiming Fusion saves cost without evidence.

**Step 4: Verify and commit.**

Run: `cargo test --test eval_runner; fuselect eval --offline`

Expected: PASS and a deterministic aggregate report.

```bash
git add src/eval src/commands/eval.rs evals docs tests
git commit -m "test: calibrate Fuselect quality and cost policies"
```

### Task 20: Mature documentation, recovery, and contributor experience

**Files:**
- Create: `docs/architecture.md`
- Create: `docs/compatibility-matrix.md`
- Create: `docs/config-reference.md`
- Create: `docs/privacy.md`
- Create: `docs/recovery.md`
- Create: `docs/adr/0001-local-routing-and-fusion-tool-loop.md`
- Create: `docs/adr/0002-privacy-and-secret-boundary.md`
- Create: `docs/adr/0003-protocol-and-provider-compatibility.md`
- Create: `CONTRIBUTING.md`
- Create: `CODE_OF_CONDUCT.md`
- Create: `.github/ISSUE_TEMPLATE/bug.yml`
- Create: `.github/ISSUE_TEMPLATE/provider-compatibility.yml`
- Modify: `README.md`

**Step 1: Document the contract, not only installation.**

Document the public APIs, exact supported and rejected Responses/Chat fields, Worker admission probe, Fusion state machine, cancellation/retry semantics, privacy model, local data locations, upstream-data-egress model, provider terms/privacy-link responsibility, backup/restore, upgrade/migration procedure, and compatibility status by Codex and Provider version. State that Fuselect itself sends no analytics, crash reports, or telemetry unless a future explicit opt-in feature is accepted and documented.

**Step 2: Make recovery self-service.**

Implement and document `fuselect backup create`, `fuselect backup list`, `fuselect backup restore <id>`, `fuselect config validate`, and `fuselect config export --redact`. Backups and redacted exports contain metadata/presets but never API Keys, Gateway Keys, raw session state, or request content; restore is atomic and creates a pre-restore backup. Importing an export is an explicit merge preview with conflict choices and never replaces a local secret reference. Test downgrade refusal and failed-migration rollback so a corrupt or too-new configuration cannot be partially applied.

**Step 3: Release and contribution checks.**

Add conventional issue templates, a minimal reproducible bug-report command that redacts secrets, contribution setup, architecture decision record policy, semantic versioning policy, changelog policy, and deprecation policy for protocol/config fields. Ship the first three ADRs: why routing is deterministic and local rather than a cloud orchestrator; why Fusion uses one outer Worker plus an internal read-only advisor/Judge tool loop; and how strict/provider-specific protocol profiles are admitted, versioned, tested, deprecated, or rejected. Any future change that affects public Responses semantics, secret/data-egress boundaries, Fusion safety, persistence/migration, or a provider profile requires an ADR before merging.

**Step 4: Verify and commit.**

Run: `fuselect config validate; cargo test --all-targets`

Expected: PASS.

```bash
git add docs .github CONTRIBUTING.md CODE_OF_CONDUCT.md README.md src/commands
git commit -m "docs: prepare Fuselect for long-term maintenance"
```

### Task 21: Optional background service for daily use

**Files:**
- Create: `src/service/mod.rs`
- Create: `src/service/windows.rs`
- Create: `src/service/linux.rs`
- Create: `src/commands/service.rs`
- Create: `templates/fuselect.service`
- Test: `tests/service_definition.rs`
- Modify: `docs/operations.md`

**Step 1: Define user-scoped service behavior.**

Implement `fuselect service install|uninstall|start|stop|status`. On Linux ARM64, generate and manage a `systemd --user` unit; on Windows, create a current-user Task Scheduler entry rather than requiring an administrator-installed system service. Both invoke the absolute Fuselect binary with `gateway start`, restart after failure with bounded backoff, and never pass API Keys as command-line arguments or environment variables.

**Step 2: Test without modifying the host service manager.**

Unit-test generated unit/task definitions, absolute-path validation, command-line escaping, no-secret invariants, and idempotent install/uninstall decision logic. Keep actual registration as an explicit user command and document platform-specific permissions.

**Step 3: Verify and commit.**

Run: `cargo test --test service_definition; cargo test --all-targets`

Expected: PASS.

```bash
git add src/service src/commands/service.rs templates tests docs
git commit -m "feat: add user-scoped Fuselect gateway service"
```

### Task 22: Preserve Codex multi-turn state and compaction semantics

**Files:**
- Create: `src/conversation/mod.rs`
- Create: `src/conversation/store.rs`
- Create: `src/conversation/context_window.rs`
- Create: `src/conversation/compaction.rs`
- Modify: `src/protocol/normalized.rs`
- Modify: `src/gateway/server.rs`
- Test: `tests/conversation_state.rs`
- Test: `tests/compaction_contract.rs`

**Step 1: Write failing multi-turn contract tests.**

```rust
#[tokio::test]
async fn expands_a_virtual_previous_response_id_before_calling_a_chat_worker() {
    let result = two_turn_codex_fixture().await;
    assert!(result.second_worker_request_contains_prior_turn_context);
    assert_ne!(result.public_response_ids[0], result.upstream_response_ids[0]);
}

#[tokio::test]
async fn compaction_trigger_returns_exactly_one_compaction_output_item() {
    let events = run_compaction_trigger_fixture().await;
    assert_eq!(events.compaction_item_count(), 1);
}
```

**Step 2: Implement virtual response state.**

Issue opaque Fuselect-owned response IDs and map them to bounded, in-memory normalized conversation state: prior messages, caller tool-call/result links, selected outer Worker metadata, and context estimate. On a request with `previous_response_id`, expand the matching state plus incremental input before converting to Chat Completions; never forward a Fuselect virtual ID to a Worker and never expose an upstream response ID to Codex. Use TTL, per-session and global memory caps, LRU eviction, cancellation cleanup, and zeroization on removal. Persist only state metadata by default; raw session context is never written to SQLite, logs, backups, or the TUI.

If state is unavailable after gateway restart or eviction, return a documented typed `conversation_state_expired` error rather than silently continuing with incomplete context. Offer explicitly opt-in encrypted durable session state only after documenting its privacy/cost trade-off; it is disabled by default.

**Step 3: Implement context limits and compaction.**

Track each Worker's context limit and use a conservative token estimator plus configured output reserve before dispatch. Prefer an enabled long-context Worker when the selected Worker cannot fit the expanded conversation; otherwise initiate local compaction or return `context_limit_exceeded`. Never silently truncate messages, tool results, or instructions.

Support both Codex compaction paths: unary `POST /v1/responses/compact` and v2 `compaction_trigger` input. Treat compaction as a bounded, non-Fusion maintenance request to the selected outer Worker, retain the recent required conversation tail, and emit exactly one Fuselect-defined compaction output item expected by Codex. Keep compaction summaries only in live memory unless the operator explicitly enables encrypted durable session state.

**Step 4: Verify and commit.**

Run: `cargo test --test conversation_state --test compaction_contract; cargo test --all-targets`

Expected: PASS.

```bash
git add src/conversation src/protocol src/gateway tests
git commit -m "feat: support Codex multi-turn state and compaction"
```

### Task 23: Establish final stable-release compatibility and performance evidence

**Files:**
- Create: `tests/compatibility/matrix.rs`
- Create: `tests/load/mock_gateway.rs`
- Create: `docs/performance.md`
- Create: `docs/release-process.md`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `README.md`

**Step 1: Write failing deterministic compatibility and load tests.**

Cover the supported public surface with local mocks: authenticated health/readiness, `/v1/models`, unary and streamed Responses, Chat Completions, direct tool calls, Fusion tool-loop suppression, virtual continuation, compaction, cancellation, and typed budget/context errors. Add a bounded concurrent mixed-workload fixture that proves the configured queue, request-body, event-buffer, deadline, and cancellation limits; it must assert correctness and no leaked internal event rather than timing numbers that fluctuate on shared CI runners.

**Step 2: Add a reproducible performance protocol.**

`docs/performance.md` defines fixed mock scenarios plus an opt-in, explicitly billed live-provider benchmark. It records p50/p95 end-to-end latency, time-to-first-public-token, throughput, error rate, stage token usage, known cost, and Fusion/direct comparison. Live benchmarks require an explicit `--allow-paid-network` flag and cannot run in pull-request CI. Report environment, version, preset, Worker configuration with secrets redacted, sample count, and confidence caveats; never publish prompts or model outputs.

**Step 3: Build the release-candidate workflow.**

For a version tag, require the full compatibility suite, offline evaluation non-regression, dependency/security gates, license/SBOM/provenance artifacts, and platform builds for Windows x64 and Linux ARM64. Generate checksums and a machine-readable release manifest containing source revision, Rust version, test/evaluation result identifiers, supported Codex-version range, and compatibility-matrix revision. Re-run package validation after all mature tasks—not only the earlier MVP packaging task.

**Step 4: Define rollback and support proof.**

Document a release rollback path: retain prior signed artifacts, preserve schema compatibility or provide a tested migration/restore path, and state when an upstream Codex or Worker compatibility regression withdraws a release. Add an issue-template field for redacted `fuselect doctor --json`, version, platform, and correlation ID; it must explicitly prohibit prompt, code, and API-key submission.

**Step 5: Verify and commit.**

Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --all-targets; fuselect eval --offline; cargo deny check`

Expected: PASS. Run the documented live benchmark manually only with non-production secrets and record its redacted report separately from the repository.

```bash
git add tests/compatibility tests/load docs .github README.md
git commit -m "chore: add stable release compatibility and performance gates"
```

## Explicit non-goals for v1

- Desktop/graphical interface (the terminal control console is supported; a native GUI is not).
- Remote configuration synchronization or cloud storage of API keys.
- Automatic discovery of models through `/v1/models`.
- Training a local router or persisting raw Trace-to-Train data.
- Supporting upstream protocols other than verified OpenAI Chat Completions.
- Letting Fusion advisors or Judges invoke Codex's tools.
- Web search and web fetch for advisors/Judge in the first release; add them in a later server-tool extension after defining privacy, citation, and network policies.
- Nested or unbounded Fusion.

## Release acceptance checklist

1. A fresh Windows installation can configure Workers, Fusion presets, and a Codex profile without exposing a secret in stdout, SQLite, or logs.
2. A fresh Linux ARM64 device can run `fuselect gateway start` and the same profile configuration.
3. Codex calling either public virtual model receives valid Responses SSE for text and a streamed caller function/tool call.
4. An outer Worker can answer directly, or invoke Fusion and continue from its Judge analysis without exposing the internal `fuselect__fusion` call to Codex.
5. A Fusion route starts the configured 1–8 advisors concurrently and one Judge; only the outer Worker sees original Codex tools.
6. Per-task and daily budgets reserve each bounded stage before dispatch, stop work before an over-budget call is sent, and mark missing provider usage as unknown.
7. The metadata database contains no raw prompt, source code, response, tool argument, tool output, API key, or Authorization header.
8. Internal server-tool calls, panel answers, Judge output, and upstream Worker IDs never appear in the public Responses/Chat Completions stream.
9. `cargo fmt --check`, strict Clippy, all unit/integration tests, and the compatibility fixture suite pass on CI.
10. Multi-turn fixtures prove that virtual `previous_response_id` expansion, caller tool-result continuation, compaction, eviction, and gateway-restart expiry never silently lose context or expose upstream IDs.

## Mature stable-release gate

Do not label a Fuselect release “stable” until the compatibility checklist above and all of the following have evidence:

1. Resilience tests cover cancellation, concurrency limits, circuit open/half-open recovery, graceful shutdown, and no retry after public output commitment.
2. `cargo audit`, `cargo deny check`, SBOM/provenance generation, secret-redaction tests, deterministic protocol property tests, scheduled parser/codec fuzzing, and the documented threat-model review pass.
3. The offline evaluation suite is non-regressing on tool/protocol correctness, and published aggregate results justify each default preset's quality/cost trade-off.
4. `fuselect doctor`, backup/restore, config migration, Codex setup/rollback, and service-definition tests pass on Windows x64 and Linux ARM64 CI/manual acceptance environments.
5. Documentation includes installation, compatibility matrix, architecture, operations, recovery, security reporting, performance methodology, release rollback, and a reproducible redacted bug-report path.
6. The version-tag release workflow from Task 23 produces validated Windows x64 and Linux ARM64 artifacts, checksums, SBOM/provenance, and a release manifest tied to the exact test/evaluation evidence.
