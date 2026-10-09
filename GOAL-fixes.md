RESOLVED: `DOWNSTREAM.md` explains that `760ede5a9f` adds `rollout_uncompressed_size` and does not remove `read_rollout_lines`; the later change scopes `read_rollout_lines` to tests because production uses the incremental `open_rollout_line_reader` API. The production size API remains exported and used by rollout recording and thread-store code.

