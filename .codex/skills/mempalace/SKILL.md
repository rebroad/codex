---
name: mempalace
description: Use MemPalace for durable memory, prior decisions, rationale, preferences, and project history. Use when a question is about why code is the way it is, whether something was already decided, or whether a turn should be remembered for future sessions. Also use when MemPalace tools are visible and the task involves deciding whether to retrieve or write back durable context.
---

# MemPalace

## When to Use

- Use MemPalace for durable context, not transient task chatter.
- Ask MemPalace when the user asks about prior decisions, rationale, preferences, long-lived project facts, or "why is this code like this?".
- If the code answer is obvious from the repository, use `rg` first; if the reason is historical or editorial, check MemPalace next.

## Retrieval Policy

- Prefer MemPalace for questions about intent, tradeoffs, prior approvals, and previous conclusions.
- If MemPalace has no relevant result, say so plainly instead of inventing a rationale.
- Treat a MemPalace hit as historical context, not a replacement for current source inspection.

## Write-Back Policy

- Durable turn outcomes should be written back automatically by runtime code after the turn completes.
- Use drawers for concise decisions, rationale summaries, and durable notes.
- Use KG entries only for explicit stable facts that fit a subject / predicate / object shape.
- Do not rely on prompt wording alone to persist memory. The skill guides retrieval; code performs persistence.

## Tooling

- Use the MemPalace MCP tools directly when retrieving or verifying memory.
- Prefer the smallest tool call that answers the question.
- Keep tool arguments precise and short.
