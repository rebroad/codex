# Codex Turn Lifecycle, Modes, and Survival Mode

## Scope
This document summarizes:
- how a turn starts, runs, and ends in `codex-rs/core`
- where mode rules are enforced
- where compaction is chosen (local vs remote)
- how Survival Mode is now implemented

## Turn Lifecycle (Core)
### 1) Turn submission and routing
- Entry point: `submission_loop()` in `codex-rs/core/src/codex.rs`.
- `Op::UserInput` / `Op::UserTurn` routes to `handlers::user_input_or_turn(...)`.
- `user_input_or_turn` creates a fresh `TurnContext` (`new_turn_with_sub_id`) and then:
  - tries `steer_input(...)` if a regular turn is already active
  - otherwise starts a new regular task via `spawn_task(..., RegularTask::new())`

### 2) Task startup / active-turn ownership
- `Session::start_task(...)` in `codex-rs/core/src/tasks/mod.rs`:
  - marks turn-start timing
  - initializes/uses `active_turn` + `TurnState`
  - spawns Tokio task for the selected session task

### 3) Start event emission
- Regular turns emit `EventMsg::TurnStarted` inside `RegularTask::run(...)` (`core/src/tasks/regular.rs`).
- Compact/review/shell standalone task paths have their own lifecycle semantics.

### 4) Main regular-turn execution loop
- `run_turn(...)` in `core/src/codex.rs`:
  - pre-sampling compact check
  - records context updates + user input into history
  - loops sampling requests (`run_sampling_request(...)`) until done/error/abort
  - processes tool calls, assistant output, pending steer input, hooks

### 5) Finish path
- When task future returns, `Session::on_task_finished(...)` in `core/src/tasks/mod.rs`:
  - emits metrics
  - emits `EventMsg::TurnComplete`
  - clears `active_turn`
  - may schedule follow-up turn for queued pending work

## “Phases” in Current System
There is no explicit multi-phase turn state machine in core beyond task lifecycle and stream loops.

Practical phase-like boundaries are:
- turn lifecycle events: `TurnStarted` → `TurnComplete` / `TurnAborted`
- item lifecycle events: `ItemStarted` / `ItemCompleted`
- assistant message phase metadata: `MessagePhase::{Commentary, FinalAnswer}` in protocol model items

## Collaboration Modes and Rule Surfaces
### Mode definitions
- `ModeKind` lives in `codex-rs/protocol/src/config_types.rs`.
- User-visible modes are currently `Default` and `Plan` (`TUI_VISIBLE_COLLABORATION_MODES`).

### Where mode rules are enforced
- Mode-specific instructions are provided by collaboration-mode templates and presets (`core/src/models_manager/collaboration_mode_presets.rs`, `core/templates/collaboration_mode/*.md`).
- Tool gating for `request_user_input` is enforced in:
  - `core/src/tools/handlers/request_user_input.rs`
  - plus mode helper `ModeKind::allows_request_user_input()`

## Compaction Selection
### Existing behavior
- Compaction task choice:
  - `core/src/tasks/compact.rs` for manual `/compact`
  - `run_auto_compact(...)` in `core/src/codex.rs` for automatic compaction
- Remote compaction is used for OpenAI providers unless local compaction is enabled (`Feature::LocalCompaction`).

## Survival Mode: Implemented Behavior
Implemented in this change set.

### Activation trigger
- In `Session::update_rate_limits(...)` (`core/src/codex.rs`), Survival Mode activates when backend rate-limit snapshot indicates weekly usage has reached `>=100%`.
- Detection rule:
  - `secondary.used_percent >= 100.0`, or
  - `primary.used_percent >= 100.0` with weekly-like window duration (`>= 10080` minutes)

### Deactivation trigger
- In `Session::record_usage_limit_reached(...)`, Survival Mode deactivates when backend hard-rejects a request (`UsageLimitReached`; usage-log “101%” path).

### Runtime effects
1. **Do not allow regular turn to end while active**
   - In `run_turn(...)`, when model reports `!needs_follow_up`, we now:
     - park the turn loop while Survival Mode is active and no pending same-turn input exists
     - continue immediately once pending input arrives
     - still exit on cancellation/interrupt, or once Survival Mode deactivates

2. **Force local compaction while active**
   - `run_auto_compact(...)` now treats Survival Mode as local-compaction-on.
   - `CompactTask::run(...)` also forces local compaction while Survival Mode is active.

3. **Prompt guidance without persisting thread-history mode churn**
   - `run_sampling_request(...)` appends ephemeral Survival guidance into prompt base instructions while Survival Mode is active.
   - This is transient (request-time prompt shaping), not a persisted history mutation.

4. **Allow request_user_input during Survival Mode**
   - `request_user_input` handler now checks session survival state.
   - The tool is allowed when Survival Mode is active even if current collaboration mode would normally reject it.
   - Tool description text now reflects this availability.

## State Added
- `SessionState.survival_mode_active: bool` in `core/src/state/session.rs`.
- Accessors were added and consumed from core turn/task/tool paths.

## Key Files Changed
- `codex-rs/core/src/state/session.rs`
- `codex-rs/core/src/codex.rs`
- `codex-rs/core/src/tasks/compact.rs`
- `codex-rs/core/src/tools/handlers/request_user_input.rs`
- `codex-rs/core/src/tools/handlers/request_user_input_tests.rs`

## Remaining Considerations
- Survival Mode is currently session-internal, not a new user-selectable collaboration mode.
- TUI/UI can infer behavior from warning events + active-turn persistence, but no dedicated “Survival” mode indicator has been added yet.
