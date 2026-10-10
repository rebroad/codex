# Goals

## Completed

- [x] Verify on Flip7 (SM-F766B, Android 16) using Codex CLI
  `0.162.0-alpha.20-96b2ce9668-202610081840` in Termux 0.118.4 on a separate
  scrcpy virtual display. With the keyboard hidden, tapping the empty prompt
  displayed HoneyBoard on that virtual display. Tapping between `b` and `c` in
  an unsent `abcdef` draft and typing `X` produced `abXcdef`, confirming cursor
  placement was preserved. The draft was cleared, the test shell exited, and
  the virtual display was closed; the built-in displays remained unchanged.
