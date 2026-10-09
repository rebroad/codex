TODO: `codex app-server reload` isn't working (it fails with `Error: timed out waiting for config/reload response` and `Caused by: deadline has elapsed`). Fix this.

Also, side-chats are not using cached_input tokens (which they should given their context window is almost identical to the conversation they are a "side chat" to). Can we fix this? See these example rollout files (the middle one is the ephemeral side-chat rollout):-
2026-10-09T18:35:02.819Z cost=$0.001455 input=2300 cached=116480 output=120 gpt-6-luna 01a121e2-d5e7-7683-bc9f-358d4c66cfe7
2026-10-09T18:36:27.319Z cost=$0.012187 input=120132 cached=1792 output=311 gpt-6-luna 01a121f2-eb2f-77d2-8755-9b6b016951f0
2026-10-09T18:38:07.333Z cost=$0.001459 input=2021 cached=118528 output=144 gpt-6-luna 01a121e2-d5e7-7683-bc9f-358d4c66cfe7

