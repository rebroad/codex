# Generalized Codex Prompt-Cache Improvements

## Summary

Codex already sends a `prompt_cache_key` on Responses requests. Guardian review sessions override that key to `guardian:<parent-thread-id>`, and delta-mode Guardian follow-ups can reuse most of their preceding review context.

The remaining problem is cache-prefix selection. Codex cannot currently serialize the GPT-5.6 `prompt_cache_breakpoint` field or `prompt_cache_options`, so the backend must choose an implicit breakpoint. When a stable prompt is followed by a new approval action or transcript delta, that implicit boundary can move and cause a large prefix rewrite.

This document describes a generalized cache-control foundation, with Guardian as the first measured consumer. The same foundation can later be applied to main turns, delegated subagents, review requests, and compaction paths when those consumers have a demonstrably stable rendered prefix.

## Evidence and motivation

The 19 August 2026 session with parent thread `01a01a34-548f-7db0-9a42-c204685f42d0` provides a direct Guardian example:

| UTC time | Request source | Total input | Cached input | Uncached input | Approx. cached share |
| --- | --- | ---: | ---: | ---: | ---: |
| 22:19:39.624 | Guardian | 136,538 | 4,864 | 131,674 | 3.6% |
| 22:19:52.213 | Guardian follow-up | 137,701 | 135,936 | 1,765 | 98.7% |
| 22:20:07.560 | Guardian follow-up | 138,530 | 136,960 | 1,570 | 98.9% |
| 22:24:42.095 | Guardian | 140,101 | 4,864 | 135,237 | 3.5% |

The Guardian rollout identifies itself as a `guardian` subagent, while the cost display attributes usage to the parent session ID. This explains why the expensive requests can look like ordinary parent-session entries. The corresponding raw records are in:

- [Guardian rollout](</home/rebroad/.codex/sessions/2026/08/19/rollout-2026-08-19T21-31-08-01a01bb8-7377-7b43-aa38-ff714f41cdf4.jsonl:797>)
- [Parent rollout](</home/rebroad/.codex/sessions/2026/08/19/rollout-2026-08-19T14-27-12-01a01a34-548f-7db0-9a42-c204685f42d0.jsonl:2686>)

The follow-up requests demonstrate that Guardian's own cache can work well. The fix should improve the cold/renewal requests without trying to treat the parent cache as a transferable token balance.

## Goals

- Make explicit cache-breakpoint controls representable on Codex Responses requests.
- Preserve provider and model compatibility through capability gating.
- Give prompt-producing subsystems a structural way to identify a stable rendered prefix.
- Improve Guardian cache reuse first, then extend the mechanism to other measured consumers.
- Measure cache reads, cache writes, uncached input, latency, and behavioral correctness separately.
- Leave approval policy, prompt meaning, model selection, and fail-closed safety behavior unchanged.

## Non-goals

- Do not use one global cache key for unrelated threads.
- Do not treat cached tokens as a session-wide shared budget.
- Do not locate breakpoints with text searches, token offsets, or fragile request-shape guesses.
- Do not mark every long prompt as cacheable without proving that the prefix is stable.
- Do not claim cost savings from `cached_input_tokens` alone; cache reads, cache writes, and uncached input must remain distinct.

## Current implementation constraints

The relevant existing behavior is:

- `ModelClient` sends `prompt_cache_key` on every Responses request, defaulting to the session ID unless an override is supplied.
- Guardian overrides the key with `guardian:<parent-thread-id>`.
- Guardian review sessions use `Full` mode initially and `Delta` mode after a review cursor is established.
- The Guardian prompt builder returns `Vec<UserInput>` but does not return a cache-boundary marker.
- `ResponsesApiRequest` and `ResponseCreateWsRequest` contain `prompt_cache_key`, but not `prompt_cache_options` or breakpoint metadata.
- `ContentItem::InputText` and other supported input content variants do not contain a `prompt_cache_breakpoint` field.
- GPT-5.6 catalog entries use Responses Lite, while other providers and models can use the non-Lite layout.

The key implementation paths are:

- `codex-rs/core/src/client.rs`
- `codex-rs/codex-api/src/common.rs`
- `codex-rs/protocol/src/models.rs`
- `codex-rs/core/src/guardian/prompt.rs`
- `codex-rs/core/src/guardian/review_session.rs`

## Proposed architecture

### 1. Generic cache-control wire types

Add typed, optional cache-control fields to the shared Responses request model:

- `prompt_cache_options` on the HTTP request.
- `prompt_cache_options` on the WebSocket request.
- `prompt_cache_breakpoint` on supported input content blocks.

The fields must serialize identically on both transports. They must be omitted by default so existing providers and models receive the current request shape unless the capability gate opts in.

The initial content-block coverage should include `InputText` and `InputImage`. Add `InputFile` when the current protocol representation supports it. Unsupported content types, including proprietary structural items such as Responses Lite's `additional_tools`, must not be given fabricated breakpoint fields.

`prompt_cache_options.mode = "explicit"` should be available as an optional control. The first effectiveness experiment can use only the content-block breakpoint, because the upstream controlled reproduction recovered cache hits without sending top-level options. Explicit mode should be enabled only when the provider/model capability is confirmed.

### 2. Capability gating

Add an explicit provider/model capability for prompt-cache breakpoints and explicit cache mode. Do not infer support solely from a model name: custom providers can expose different request capabilities, and older models may reject GPT-5.6-only fields.

The capability must be available where request construction already receives model metadata. The default must be disabled unless the model/provider catalog or an equivalent authoritative source enables it.

Capability-gated behavior must cover:

- HTTP Responses requests.
- WebSocket Responses requests.
- Responses Lite requests.
- Non-Lite Responses requests.
- Custom providers with compatible model metadata.

### 3. Structural stable-prefix boundary

Prompt builders should return a structural boundary alongside rendered input, rather than requiring the client to infer it from text. The boundary should identify the final supported content block in the stable prefix.

The boundary must be applied after prompt rendering and before request serialization, because the final placement depends on the request layout. It must survive content preparation steps such as image admission, truncation, and Responses Lite conversion.

The boundary should be absent when:

- The prompt builder cannot prove the prefix is stable.
- The model/provider does not advertise support.
- The request is a special endpoint whose cache semantics have not been validated.
- A transformation would move or remove the marked content block.

## Consumer rollout

### Stage 1: Guardian

Guardian is the first consumer because the local rollout demonstrates the exact failure pattern.

Keep the current cache-key isolation:

```text
guardian:<parent-thread-id>
```

Do not replace it with the parent thread's ordinary key. Guardian has different instructions, tools, policy context, and prompt structure, so parent cache entries are not directly reusable even when they share a parent thread.

The Guardian prompt builder should expose the stable-prefix boundary explicitly. The breakpoint belongs immediately before the volatile approval assessment material, such as the current action, retry reason, and newly admitted transcript delta. The exact boundary must be derived from the rendered Guardian request, not from a hard-coded token index.

The implementation must support both Guardian prompt modes:

- Initial `Full` review.
- Subsequent `Delta` reviews using the saved transcript cursor.

The expected behavior is that a cold request remains cold, while a follow-up or renewal request with an unchanged stable Guardian prefix reads the large Guardian cache and writes only the changing suffix.

### Stage 2: Main interactive turns

Audit normal parent requests for a large stable startup prefix followed by changing user/tool content. Main turns already achieve high cache reuse in ordinary append-only sequences, so this stage should target only measured independent-request or changing-tail misses.

The implementation must account for dynamic instructions, workspace configuration, permissions, available tools, current context, and provider-specific request layout. A developer message is not automatically stable merely because it appears before user content.

### Stage 3: Delegated and forked subagents

Audit delegated sessions for intentionally inherited context. A subagent may use an inherited cache lineage only when the inherited instructions, tools, provider identity, permissions, and security context are compatible.

Where a child intentionally reuses a parent prefix, represent both:

- the cache-key policy for the child;
- the explicit inherited-prefix boundary.

Otherwise retain child isolation. Relationship alone is not sufficient reason to share a cache key.

### Stage 4: Manual review and compaction

Audit manual review, remote review, and compaction requests separately. These paths may use different endpoints, input representations, or model capabilities.

Do not apply ordinary Responses breakpoint logic to a compaction endpoint until its request schema and backend cache behavior are verified. Add endpoint-specific tests before enabling the fields there.

## Testing strategy

### Protocol serialization tests

Add focused tests that:

- Serialize a breakpoint on `InputText`.
- Serialize a breakpoint on `InputImage`.
- Omit the field when unset.
- Serialize the same request through HTTP and WebSocket models.
- Serialize `prompt_cache_options` when enabled.
- Omit GPT-5.6-only fields when capability gating is disabled.
- Cover both Lite and non-Lite request construction.
- Reject or prevent marking unsupported structural content.

The tests should compare complete serialized request objects where practical, not isolated fields only.

### Guardian prompt-layout tests

Add tests in the Guardian prompt test area that verify:

- The initial full prompt exposes the expected stable boundary.
- A delta prompt preserves the previous stable prefix and places new transcript/action material after the boundary.
- Changing the command, justification, retry reason, or transcript delta does not alter content before the boundary.
- Dynamic permission or policy context invalidates or moves the boundary when it genuinely changes the stable prefix.
- The breakpoint remains attached to a supported content block after image filtering and prompt preparation.
- Guardian snapshots show the boundary in both full and follow-up request layouts.

### End-to-end request tests

Capture outbound Guardian requests through the existing response mocks and assert:

- The Guardian cache key remains stable across reviews in one parent thread.
- Parent requests retain their existing cache key and request shape.
- Guardian requests contain the breakpoint only when capability gating permits it.
- Follow-up requests preserve prior Guardian history and place new review material after the marked prefix.
- Multiple approval action types use the same safe placement policy.
- Failure, cancellation, denial, retry, and fail-closed paths are behaviorally unchanged.

### Live A/B effectiveness test

Run the same representative workload against baseline and fixed binaries. Use the same model, provider, service tier, cache key, parent transcript, approval actions, and timing as far as possible.

Include:

1. A cold Guardian review.
2. Two or more immediate follow-up reviews.
3. A new review after a parent transcript delta.
4. A retry after a failed or denied review.
5. Reviews separated by several minutes.
6. Sequential and concurrent approval requests.
7. At least one full-mode and one delta-mode review.
8. At least one Responses Lite and one non-Lite-compatible fixture where available.

For every request, record:

- timestamp;
- parent thread ID and Guardian session ID;
- request source/subsystem;
- model and provider;
- cache key;
- total input tokens;
- cached input tokens;
- cache-write input tokens;
- uncached input tokens;
- breakpoint mode and location;
- full versus delta prompt mode;
- response latency;
- Guardian outcome.

The primary effectiveness metrics are:

```text
cache_read_ratio = cached_input_tokens / input_tokens
uncached_ratio   = (input_tokens - cached_input_tokens) / input_tokens
write_ratio      = cache_write_input_tokens / input_tokens
```

These metrics must be reported separately for cold starts, warm follow-ups, renewal requests, retries, and concurrent requests. A single aggregate hit rate could hide the exact Guardian renewal failure being fixed.

## Acceptance criteria

The generalized foundation is acceptable when:

- All supported transports serialize identical cache-control semantics.
- Unsupported providers/models continue receiving the old request shape.
- No existing approval or model behavior changes.
- No breakpoint is placed on an unstable or unsupported prefix.
- Guardian warm follow-ups retain approximately the observed 98% cache-read behavior.
- Guardian requests currently showing approximately 4,864 cached tokens reuse the large stable Guardian prefix when that prefix is unchanged.
- Cache writes are concentrated in genuinely new prompt suffixes.
- Parent and Guardian cache identities remain isolated.
- Fail-closed Guardian behavior remains unchanged for malformed, failed, timed-out, or unsupported reviews.
- Live measurements show the change improves cache reads without introducing request rejection, latency regression, or prompt-layout drift.

## Rollout and observability

Enable the feature behind capability metadata first, with a diagnostic mode that records breakpoint decisions and token usage without changing the request when the provider is not explicitly enabled.

Roll out in this order:

1. Protocol serialization with all capabilities disabled by default.
2. Guardian on a controlled GPT-5.6-capable provider.
3. Main turns only where measured misses justify a boundary.
4. Delegated/forked sessions after cache-lineage policy is reviewed.
5. Review and compaction endpoints after endpoint-specific validation.

Monitor:

- unsupported-parameter errors;
- cache-read and cache-write token distributions;
- Guardian approval latency and timeout rates;
- Guardian malformed-response and fail-closed rates;
- cache-key cardinality;
- concurrent-request hit/miss behavior.

Disable the capability for a provider/model if it rejects the fields or if prompt behavior changes unexpectedly. The fallback must be the current request shape, not a different approval policy.

## Prior art

- [OpenAI Codex issue #35300](https://github.com/openai/codex/issues/35300) reports the missing `prompt_cache_breakpoint` serialization and includes a controlled experiment where explicit breakpoints improved warm hit rates from 0% to approximately 98.6%.
- [OpenAI Codex issue #37674](https://github.com/openai/codex/issues/37674) reports high GPT-5.6 cache-write spend and proposes explicit cache options, typed breakpoints, capability gating, stable-prefix placement, and telemetry.

Both issues remain useful design evidence, but neither records a landed upstream implementation. The local Guardian rollout supplies the consumer-specific evidence needed to apply the generalized mechanism safely.
