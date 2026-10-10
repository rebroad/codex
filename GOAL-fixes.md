# Goals

## Completed

- [x] Make sccache usable from the normal sandboxed shell, without requiring
  shell escalation. Keep sccache enabled by default, and use an escalated shell
  for builds only when the sandbox prevents sccache from running.
- [x] Preserve prompt-cache affinity when forking persistent conversations.
  The 2026-10-09 rollout samples showed a recent parent request using 76,544
  cached tokens, then its fork starting with only 1,792 cached tokens on a
  115,075-token input. The following fork request used 116,480 cached tokens.
  The recent side-chat change had moved its boundary instructions after the
  inherited history, but only ephemeral forks reused the inherited
  `prompt_cache_key`; persistent forks still used their new session ID. The
  first live check showed the regression remained for paginated forks: 1,792
  cached of 74,775 input tokens because reference-backed history omits
  `SessionMeta`. Root forks now use the inherited metadata session ID when
  present and fall back to the parent thread ID for reference-backed history,
  while keeping their own thread and storage identities. A repeat live
  `gpt-6-luna` fork test then reported 74,496 cached of 74,777 input tokens.
  OpenAI documents `prompt_cache_key` as a routing aid for related requests
  sharing a prefix; it does not guarantee a cache hit.
