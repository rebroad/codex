# MemPalace Tool Guide

This is a model-facing usage guide for the MemPalace MCP tools currently exposed to Codex.

The core rule I would follow is:

- use read tools before write tools
- use semantic search for fuzzy recall
- use the knowledge graph for explicit entities and relationships
- use maintenance tools only when asked or when a memory lifecycle task is clearly needed
- never rewrite or delete memory just because a newer answer exists in the context window

## Session And Discovery

### `mempalace_status`

Use this first when I need the current shape of the MemPalace installation.

- I would use it to confirm the service is alive, what tool families are present, and whether the protocol/spec guidance is available.
- I would not use it for finding a specific fact or drawer.

### `mempalace_get_aaak_spec`

Use this when I need to read or write AAAK-formatted memory correctly.

- I would use it before creating or editing stored content that should follow the compressed AAAK dialect.
- I would not use it for ordinary factual retrieval.

### `mempalace_reconnect`

Use this when the server’s in-memory search/index state may be stale.

- I would use it after external scripts, direct filesystem writes, or any operation that may have changed the palace outside the running process.
- I would not use it as a normal refresh button during ordinary lookup.

## Semantic Recall

### `mempalace_search`

Use this for fuzzy retrieval over drawer content.

- I would use it when I know keywords, concepts, or approximate wording but not the exact drawer or entity.
- I would use it before assuming a memory does not exist.
- I would not use it when I already know the exact entity and relationship I want from the knowledge graph.

### `mempalace_get_drawer`

Use this for exact retrieval of a known drawer ID.

- I would use it when search already identified a drawer and I want the full verbatim content.
- I would not use it to discover drawers or browse broadly.

### `mempalace_list_drawers`

Use this for browsing drawers by wing or room, or for pagination over a set.

- I would use it when I need inventory-style inspection of stored memories.
- I would not use it when I already have a drawer ID or a precise KG entity.

## Knowledge Graph

### `mempalace_kg_query`

Use this for explicit facts and relationships.

- I would use it when I know the entity and want structured facts such as `works_on`, `loves`, `child_of`, or similar typed relationships.
- I would use `as_of` when the time dimension matters.
- I would not use it for broad keyword search.

### `mempalace_kg_stats`

Use this for graph-level health and scale.

- I would use it to inspect how many entities, triples, and relationship types exist.
- I would not use it to answer a user’s factual question.

### `mempalace_kg_timeline`

Use this for chronological history of an entity or the whole graph.

- I would use it when I need to understand how a fact changed over time.
- I would not use it when a single current fact is enough.

## Writing And Editing Memory

### `mempalace_add_drawer`

Use this to store new memory.

- I would use it for durable preferences, project facts, recurring constraints, decisions, or timelines that should survive beyond the current context window.
- I would not use it for transient task details already present in the current conversation.

### `mempalace_update_drawer`

Use this to modify an existing drawer while preserving the same memory entry.

- I would use it when the drawer remains the right home but the content or room should change.
- I would not use it if a fact has become false and should be invalidated instead.

### `mempalace_delete_drawer`

Use this for irreversible deletion.

- I would use it only when a drawer should be removed entirely and the user explicitly wants that.
- I would not use it for ordinary corrections.

### `mempalace_forget_drawer`

Use this for safe forgetting.

- I would use it when a memory should be tombstoned, linked knowledge invalidated, and dependent closets rebuilt.
- I would prefer this over hard delete when the memory graph should stay internally consistent.

### `mempalace_forget_run`

Use this for lifecycle maintenance across the palace.

- I would use it for bulk decay/purge maintenance, not for a single factual correction.
- I would not use it in the middle of a normal retrieval request.

### `mempalace_forget_stats`

Use this to inspect forgetting state.

- I would use it when debugging lifecycle policy, decay, tombstoning, or purge eligibility.
- I would not use it for ordinary retrieval.

## Structure And Navigation

### `mempalace_get_taxonomy`

Use this to inspect the wing/room/drawer structure.

- I would use it when I need a map of the palace layout or want to understand where content tends to live.
- I would not use it as a substitute for actual memory retrieval.

### `mempalace_list_wings`

Use this to enumerate wings and their drawer counts.

- I would use it when I need a top-level inventory view.
- I would not use it when I already know the wing.

### `mempalace_list_rooms`

Use this to enumerate rooms within a wing.

- I would use it after choosing a wing and wanting to narrow the browse scope.
- I would not use it as a first-pass recall tool.

### `mempalace_list_tunnels`

Use this to inspect explicit cross-wing connections.

- I would use it when I need to understand how ideas relate across wings.
- I would not use it for direct fact lookup.

### `mempalace_traverse`

Use this to walk the palace graph from a room.

- I would use it when I want a guided exploration path from one memory location to nearby connected ideas.
- I would not use it when I already know the exact drawer or entity.

### `mempalace_follow_tunnels`

Use this to inspect the explicit bridges out of a room.

- I would use it when I want to see where one room connects in other wings.
- I would not use it for general search or graph statistics.

### `mempalace_create_tunnel`

Use this to create an explicit cross-wing link.

- I would use it when two memories in different wings should be connected deliberately.
- I would not use it just because two drawers happen to be semantically related.

### `mempalace_delete_tunnel`

Use this to remove an explicit cross-wing link.

- I would use it when the bridge is wrong or no longer meaningful.
- I would not use it for changing drawer content itself.

## Operational Preference

If I were deciding which MemPalace tool to use, I would choose in this order:

1. `mempalace_status` if I need to know the current capability surface.
2. `mempalace_search` if I am not sure what exact memory exists.
3. `mempalace_kg_query` if I know the entity and want structured facts.
4. `mempalace_get_drawer` if I already know the drawer ID.
5. `mempalace_add_drawer` or `mempalace_update_drawer` only when I need to persist or correct memory.

That is the practical distinction I would keep in mind:

- search is for recall
- KG is for facts
- drawers are for verbatim stored content
- tunnels are for relationships between drawers
- forget/delete are for lifecycle management
- status/taxonomy/stats are for inspection

