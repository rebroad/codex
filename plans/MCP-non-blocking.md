# Make Codex Apps MCP startup non-blocking and diagnostically useful

## Summary

Use the persisted Apps tool cache during app-server startup, initiate the backend connection in the
background, and preserve the actual startup failure cause for user-visible status/logging. The 30-
second timeout remains as a safety limit for the background connection rather than delaying normal
startup.

## Implementation changes

- Change host-owned codex_apps startup behavior:
    - If valid cached Apps tools exist, publish/use them immediately.
    - Start the ChatGPT backend MCP connection asynchronously.
    - Replace the cache only after a successful authenticated refresh.
    - Keep cached tools usable while reconnect attempts fail.
    - If no cache exists, retain the current initial connection attempt, since there is no tool
      catalog to use.

- Preserve startup failure causes:
    - Do not collapse the timeout branch into a generic string while discarding the underlying
      error.

    - Record the startup phase: connection, authentication, MCP initialization, or tool discovery.
    - Classify and expose:
        - missing/invalid local credentials;
        - HTTP authentication rejection;
        - network/DNS/proxy failure;
        - MCP protocol/server failure;
        - actual timeout.

    - Keep credentials, tokens, and authorization headers out of errors and logs.

- Improve user-visible reporting:
    - Show cached Apps availability separately from backend connection status.
    - Report a concise reason when Apps cannot reconnect, for example:
      Apps MCP unavailable: authentication required
      or
      Apps MCP connection timed out contacting ChatGPT.

    - Retain detailed causal errors in debug/trace logs.
    - Continue automatic retry/re-authentication when auth.json is replaced.

- Keep the existing persisted cache identity scoped to the authenticated account/workspace, so
  credentials for one account cannot reuse another account’s Apps catalog.

## Tests

Add focused MCP tests covering:

- cold start with a valid persisted tool cache does not block startup;
- background reconnect success replaces the cached catalog;
- background reconnect timeout leaves cached tools available;
- malformed/replaced auth.json produces an authentication-specific status;
- HTTP 401/403 is not reported as a generic timeout;
- DNS/network failure and MCP protocol failure retain distinct causes;
- error output never contains bearer tokens or credential contents;
- no-cache startup still reports a genuine backend timeout correctly.

Run the affected codex-mcp, codex-connectors, and relevant core tests, followed by formatting and
the repository-required validation.

## Assumptions

- Apps should remain enabled by default.
- The existing 30-second timeout remains unchanged initially; avoiding the blocking wait is the
  primary fix.

- No new config.toml timeout setting is needed unless testing shows that a no-cache first
  connection still needs product-specific tuning.
