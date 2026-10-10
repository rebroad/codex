# Goals

## Completed

- [x] Make sccache usable from the normal sandboxed shell, without requiring
  shell escalation. Keep sccache enabled by default, and use an escalated shell
  for builds only when the sandbox prevents sccache from running.
