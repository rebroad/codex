# Codex usage and cost accounting for third-party consumers

This document describes how an application such as Codeburn should calculate
token usage from Codex rollout data. It covers ordinary model requests,
cache-write tokens, context compaction, forks, and the boundary between local
usage reconstruction and authoritative account billing.

## Executive summary

Use one exact-usage record per upstream model completion:

```text
RolloutItem::EventMsg(EventMsg::RawResponseCompleted {
    response_id,
    token_usage: Some(...),
})
```

Sum the fields in `token_usage` across the logical thread history:

- `input_tokens`
- `cached_input_tokens`
- `cache_write_input_tokens`
- `output_tokens`
- `reasoning_output_tokens`
- `total_tokens`

Do not calculate billing by summing `TokenCountEvent.info.total_token_usage`.
That value is cumulative, can be emitted multiple times, and is recomputed as
an estimate after compaction.

For USD, apply a versioned price table using the model/provider/service-tier
metadata associated with each request. Codex records token quantities; it does
not calculate USD and does not expose the account's authoritative credit ledger
in rollout files.

For credits, there is no general token-to-credit conversion that can be safely
derived from a rollout. Report reconstructed token usage and estimated USD
separately from backend-reported credits or account billing.

## Source of truth inside a rollout

`RawResponseCompletedEvent` is defined in
`codex-rs/protocol/src/protocol.rs`. It represents the exact usage reported by
one upstream Responses API completion and is explicitly different from the
accumulated `TokenCountEvent` model.

The event has this shape:

```json
{
  "type": "event_msg",
  "payload": {
    "type": "raw_response_completed",
    "response_id": "resp_...",
    "token_usage": {
      "input_tokens": 1000,
      "cached_input_tokens": 800,
      "cache_write_input_tokens": 100,
      "output_tokens": 250,
      "reasoning_output_tokens": 50,
      "total_tokens": 1250
    }
  }
}
```

The exact outer JSON names depend on the rollout serialization version. A
consumer should deserialize the protocol types or tolerate historical aliases,
rather than hard-code only the illustrative shape above.

`token_usage` is optional. A missing value means that the upstream response did
not provide usage, or that the response did not reach a usable completion. It
must be represented as unknown, not as zero.

The Responses SSE parser maps upstream
`input_tokens_details.cache_write_tokens` to
`TokenUsage.cache_write_input_tokens`. The value is therefore the upstream
reported number of input tokens written to the prompt cache for that response;
it is not an estimate made by Codex.

## Which records to count

Count every distinct successful upstream model completion that has a
`RawResponseCompleted` event with non-null `token_usage`.

This includes:

- The ordinary response request for a user turn.
- Follow-up response requests made during the same turn, such as after tool
  calls.
- Model requests used to produce a local compaction summary.
- Model requests used by remote compaction v2.
- Model fallback requests, when they complete successfully.
- Model requests made by subagents, if the application is calculating the
  aggregate cost of the parent thread and its child threads.

Do not count these as additional model usage:

- `TokenCountEvent` records.
- `TurnTokenUsageUpdated` notifications.
- `RawResponseItem` records.
- `Compacted` markers.
- `ContextCompacted` notifications.
- Rollout-trace records if the same completion was already counted from
  `RawResponseCompleted`.

The raw response event is persisted by `Session::send_event()` and is not an
accumulated or replay-only record. A replayed token-usage notification must not
be counted again.

## Deduplication and lineage

An application must calculate over a logical history, not blindly concatenate
all files it can find.

### Ordinary append-only history

Within one rollout, count each `RawResponseCompleted` event once. The safest
deduplication key is the rollout record identity/ordinal. `response_id` can be
used as a secondary diagnostic key, but do not discard two records solely
because they have similar metadata: separate upstream requests are separately
billable even when they are part of one Codex turn.

### Forked threads

Forked threads can inherit a prefix from another rollout. Count inherited
records once for the aggregate logical conversation. Then count only the child
records after the inherited boundary.

The relevant persisted metadata is `SessionMeta.history_base`, which contains
the source thread and an exclusive end ordinal/byte offset. If the consumer
cannot resolve lineage, it should mark the aggregate as uncertain rather than
silently double-counting the shared prefix.

### Retries

A failed pre-stream attempt normally has no exact completion usage. A completed
retry with its own `RawResponseCompleted` event is a real upstream completion and
must be counted. Do not collapse all records belonging to one Codex turn into
one request.

## Token fields and cost calculation

For each counted completion, retain the raw integer fields. In particular:

```text
cache_write_input_tokens
```

must be kept independently from:

```text
cached_input_tokens
```

They have different pricing meanings. A typical USD calculation is:

```text
usd =
    input_tokens              * input_price_per_token
  + cached_input_tokens       * cached_input_price_per_token
  + cache_write_input_tokens  * cache_write_price_per_token
  + output_tokens             * output_price_per_token
```

Reasoning output is normally a component of output usage for pricing, but the
price table and provider contract are authoritative. Do not add
`reasoning_output_tokens` on top of `output_tokens` unless the applicable
provider pricing explicitly requires that treatment; Codex's
`reasoning_output_tokens` is a breakdown field, not automatically an additional
billable category.

Use decimal arithmetic for money and retain the unrounded token totals. Round
only for presentation.

Prices are per model and can vary by:

- Provider/backend.
- Model identifier.
- Short versus long context.
- Service tier, such as standard, batch, flex, or fast/priority.
- Region/data-residency processing.
- Pricing effective date.

The price table must be external, versioned, and auditable. The OpenAI pricing
page currently lists cache writes as a distinct price category for models that
support it: <https://developers.openai.com/api/docs/pricing>.

## Finding request metadata

Token usage alone does not identify its price. Join each completion to the
nearest applicable request/turn metadata.

Useful rollout metadata includes:

- `TurnContextItem.model` — resolved model slug.
- `SessionMeta.model_provider` — provider identifier when available.
- `TurnContextItem` configuration — the resolved model is persisted here, but
  the ordinary item does not reliably persist the service tier. Treat service
  tier as unknown unless it is available from another source.
- Compaction metadata — whether the request was a compaction request and which
  compaction implementation was used, when available from rollout-trace or
  adjacent lifecycle records.
- Raw rollout-trace `InferenceCall` records, when enabled — these contain model,
  provider, response ID, usage, and raw request/response payload references.

If the model, provider, context-length tier, or service tier cannot be resolved,
the application may report token usage exactly but must label the USD result as
an estimate or unknown.

## Context compaction

Compaction is itself model activity and can incur input, cached-input,
cache-write, and output costs. It must be included in a complete cost report.

### Local Responses-based compaction

The local compaction path calls the normal Responses stream. Its
`response.completed` usage is persisted as `RawResponseCompleted`, so count it
like any other model completion.

### Remote compaction v2

Remote compaction v2 also persists a `RawResponseCompleted` event with the exact
usage returned by the compact endpoint. Count that event.

Codex additionally records compaction analytics containing cached-input and
cache-write counts. Those analytics are useful for diagnostics, but should not
be added to the rollout total if the corresponding raw response event was
already counted.

After compaction is installed, Codex calls `recompute_token_usage()` and emits a
new `TokenCountEvent`. That event describes the new live context estimate. It
usually has zero detailed input/cache/output fields and must not replace or
supersede the exact `RawResponseCompleted` record for compaction billing.

### Legacy remote compaction endpoint

The older remote compact endpoint currently returns compacted history without
returning structured `TokenUsage` through the Codex client path. Its trace may
contain request/response evidence, but a third-party consumer cannot always
recover exact billable token counts from the ordinary rollout records.

For this case, report compaction cost as unknown unless the backend response or
an independently captured provider billing record supplies the usage. Do not
substitute the post-compaction `TokenCountEvent` estimate for the missing
billable usage.

## Why `TokenCountEvent` is not a billing record

`TokenCountEvent.info` contains cumulative session/thread state:

- `total_token_usage` accumulates model usage.
- `last_token_usage` contains the latest known delta or context estimate.
- `model_context_window` describes the context limit.

Codex can emit this event multiple times, including after rate-limit changes,
completion, context-window recomputation, and compaction. It is intended for
context-budget UI and thread restoration. Summing it will double-count, and
post-compaction values can be estimates.

Its optional `rate_limits` field may carry backend-reported credit metadata, but
that is a snapshot, not a local debit operation.

## Credits versus USD

There are three different meanings of “credits” that must not be conflated:

1. **Prompt-cache pricing units** — cache-write and cached-input tokens used in
   a provider's USD calculation.
2. **Backend account credits** — account/workspace balance or entitlement state
   returned by the Codex/ChatGPT backend.
3. **Rate-limit reset credits** — earned reset allowances that can be consumed
   through a backend API.

Rollout token usage can support calculation of the first category when a price
table is available. It does not provide a reliable local ledger for the second
or third categories.

For account credits, use the authenticated backend's account/rate-limit APIs or
the provider's billing API. The `RateLimitSnapshot.credits` field is useful as a
display snapshot, but it is not a token-derived balance and should not be
recomputed from rollout totals.

## Recommended output model

A cost-calculating application should retain one row per counted completion:

```text
thread_id
rollout_ordinal
turn_id
response_id
request_kind: normal | local_compaction | remote_compaction_v2 | other
model
provider
service_tier
input_tokens
cached_input_tokens
cache_write_input_tokens
output_tokens
reasoning_output_tokens
total_tokens
usage_source: raw_response_completed | trace_inference_call | backend_billing
price_table_version
usd_estimate
confidence: exact_usage_estimated_price | exact_usage_exact_price | unknown
```

At the aggregate level, expose at least:

- Exact token totals by token category.
- Compaction token totals separately from ordinary-turn totals.
- USD estimate and price-table version.
- Missing-usage count and unknown-cost amount.
- Backend-reported credits separately from locally reconstructed usage.
- A lineage/deduplication warning when fork metadata is incomplete.

## Minimal algorithm

```text
records = read_rollout_lineage(thread)
records = remove_inherited_prefixes_already_counted(records)

for record in records:
    if record is RawResponseCompleted and record.token_usage exists:
        request = associate_request_metadata(record)
        add_exact_usage(request, record.token_usage)

for usage in exact_usage_rows:
    price = price_table.lookup(
        model=usage.model,
        provider=usage.provider,
        service_tier=usage.service_tier,
        context_tier=usage.context_tier,
        effective_at=usage.timestamp,
    )
    if price is available:
        usage.usd_estimate = price.calculate(usage.token_fields)
    else:
        usage.usd_estimate = unknown

never add TokenCountEvent totals to exact_usage_rows
never add trace usage to a row already sourced from RawResponseCompleted
never infer backend credits from token counts
```

## Correctness boundary

With complete rollout data and resolvable request metadata, a third-party app
can reconstruct exact upstream token usage—including cache writes and the
compaction paths that emit `RawResponseCompleted`.

It cannot guarantee an exact account charge when any of the following applies:

- The upstream response omitted usage.
- A legacy remote compaction response omitted structured usage.
- The rollout is forked and lineage is unavailable.
- The model/provider/service tier is ambiguous.
- Pricing changed or the applicable price table is unavailable.
- Backend credits, discounts, entitlements, or billing adjustments affect the
  final account charge.

In those cases, preserve the known token counts, report an explicit unknown or
estimated amount, and use backend billing/account APIs for authoritative credits
and charges.
