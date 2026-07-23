# Fusion and protocol research notes

## Scope and reuse boundary

Fuselect will implement its own Rust architecture. The checked-out projects are used for behavior, test cases, and design ideas. Copying code is not required. If code is ever copied from an MIT or Apache-2.0 source, preserve its copyright and license notices and add an attribution entry before release.

## 1. Chimera: the closest Fusion implementation

Relevant files:

- `chimera-agent/chimera/fusion/engine.py`
- `chimera-agent/chimera/fusion/cascade.py`
- `chimera-agent/chimera/fusion/router.py`
- `chimera-agent/tests/test_fusion.py`
- `chimera-agent/tests/test_fusion_ab.py`

Useful patterns:

1. Use an explicit `PanelResponse` type that represents either content or a per-model error. One failed advisor must not discard successful advisors.
2. Track token usage by stage and preserve `unknown`, rather than turning missing provider usage into fabricated zero cost.
3. Run the panel concurrently with a bounded worker count.
4. Make Judge output an analysis rather than a final answer: consensus, contradictions, partial coverage, unique insights, and blind spots.
5. Keep a full in-memory Fusion receipt for the duration of a request; persist only its metadata under Fuselect's privacy policy.
6. Test the economic claim. Chimera's selective/probe mode is interesting future work, but it is not part of the initial OpenRouter-compatible behavior.

Required adaptation:

Chimera ends Fusion with a dedicated synthesizer. Fuselect must not do that. Its Judge JSON becomes the result of `fuselect:fusion`, which is returned to the same outer Worker. The outer Worker makes the only final reply or Codex tool call.

## 2. OpenCodex: Responses and streaming test oracle

Relevant files:

- `opencodex/src/server/responses.ts`
- `opencodex/src/responses/state.ts`
- `opencodex/src/responses/parser.ts`
- `opencodex/src/lib/eventstream-decoder.ts`
- `opencodex/tests/responses-stream-tool-events.test.ts`
- `opencodex/tests/openai-chat-parallel-stream.test.ts`
- `opencodex/tests/sse-failed-tail.test.ts`

Useful patterns:

1. Treat SSE conversion as a state machine, not as JSON pass-through.
2. Buffer tool-argument deltas by output item/call ID; providers can split, duplicate, replace, or omit final argument events.
3. Test malformed and truncated SSE tails explicitly.
4. Preserve ordering among text, reasoning, and tool events.
5. Separate request parsing, response state, and provider adapters so protocol behavior can be tested without live providers.

Fuselect decision:

Build a provider-neutral `NormalizedStreamEvent` state machine. The internal `fuselect:fusion` tool call is consumed before public serialization and must never appear in Codex Responses SSE.

## 3. CC Switch: Rust adapter and failure-isolation patterns

Relevant files:

- `cc-switch-cli/src-tauri/src/proxy/providers/transform_responses.rs`
- `cc-switch-cli/src-tauri/src/proxy/providers/streaming_responses.rs`
- `cc-switch-cli/src-tauri/src/proxy/forwarder.rs`
- `cc-switch-cli/src-tauri/src/proxy/circuit_breaker.rs`
- `cc-switch-cli/src-tauri/tests/stream_check_claude_openai_responses.rs`

Useful patterns:

1. Divide the adapter into request transforms, streaming transforms, response/error normalization, and usage accounting.
2. Lock behavior with fixture-driven streaming tests before adding a new provider dialect.
3. Separate upstream errors from client-facing error summaries.

Fuselect decision:

Start with verified OpenAI Chat Completions Workers only. Do not add Claude/Gemini native adapters in v1, but keep the adapter interface narrow enough to add them later.

## 4. Official Codex Responses proxy: security baseline

Relevant files:

- `codex/codex-rs/responses-api-proxy/src/lib.rs`
- `codex/codex-rs/responses-api-proxy/src/read_api_key.rs`
- `codex/codex-rs/responses-api-proxy/src/dump.rs`

Useful patterns:

1. Bind to `127.0.0.1`, not all interfaces.
2. Accept only intended routes and reject all other routes.
3. Replace incoming authorization rather than forwarding it upstream.
4. Mark authorization headers sensitive; never log them.
5. Disable default HTTP client timeouts for long-lived streams while enforcing Fuselect's own request timeout policy.

Fuselect decision:

Use Axum/Reqwest equivalents for these controls. Fuselect differs by serving both Responses and Chat Completions, so it will authenticate callers with a generated local Gateway Key and retrieve upstream keys from the OS keyring.

## Implementation order after research

1. Implement normalized request and stream-event types, with OpenCodex-inspired SSE fixture tests.
2. Implement a loopback Responses gateway using the security boundary above.
3. Implement one direct OpenAI Chat Completions Worker with streamed caller tool calls.
4. Add the outer Worker tool loop and intercept `fuselect:fusion`.
5. Add parallel advisors, Judge JSON, typed Fusion errors, recursion protection, and per-stage budget reservations.
6. Add compatibility fixtures for duplicate deltas, malformed stream tails, parallel tool calls, advisor partial failure, and forced Fusion.

## Explicitly deferred

- Native Anthropic/Gemini adapter implementations.
- Fusion advisor/Judge web search and web fetch.
- Chimera-style selective Fusion, voting, and self-improvement loops.
- UI, account pools, OAuth, and remote configuration synchronization.

## Deep-research findings that changed the main plan

### Buffered outer-turn boundary is mandatory

The streaming adapters in `opencodex` and `cc-switch-cli` demonstrate that tool calls are reconstructed only after an SSE state machine has correlated item IDs, starts, deltas, snapshots, and terminal events. Therefore Fuselect cannot expose the first outer Worker's stream as it arrives: it would risk sending an internal Fusion call to Codex before it can be intercepted. Fuselect buffers the initial outer turn, executes Fusion when requested, then serializes only the final direct or continuation result.

### Function names and tool-turn ordering need a strict contract

OpenAI-compatible function names are more portable than a colon-delimited pseudo-type. Fuselect's public concept remains `fuselect:fusion`, but the upstream function is named `fuselect__fusion`. It is reserved and supplied by Fuselect alone. A response that calls this internal tool alongside a Codex-owned tool is rejected, because the correct order cannot safely be inferred; the model must receive Fusion analysis first and only then issue a real tool call.

### Budget reservation must be stage-based

Chimera's receipts correctly attribute panel, Judge, and synthesis cost separately and preserve unknown usage. Fuselect's equivalent needs a reservation before every call, including the outer continuation after a successful Fusion call. A missing output bound makes a safe reservation impossible, so it is a configuration/request error instead of an estimate.

### Protocol fixtures are a product requirement

OpenCodex and CC Switch both contain tests for behavior that naïve proxies miss: split and repeated argument deltas, snapshot deltas, missing final arguments, text-to-tool transitions, and malformed SSE tails. Fuselect's normalized stream state machine must own these tests independently of any specific provider.

### Security baseline

The official Codex proxy binds only loopback, rejects unknown routes, replaces caller authorization, marks sensitive headers, avoids default stream timeouts, and redacts diagnostics. Fuselect adopts these boundaries while using OS-keyring-managed secrets rather than stdin keys.
