#!/usr/bin/env node
const path = require("node:path");
const { spawn } = require("node:child_process");
const http = require("node:http");

const DEFAULT_PORT = 8788;
const HEALTH_TIMEOUT_MS = 15_000;
const HEALTH_RETRY_MS = 250;

function usage() {
  console.log(`Usage:
  node tools/prompt-debug-inspector/view-prompt-debug.js [target] [options]

Arguments:
  target               Capture directory or *_input.ndjson file path.
                       If omitted, uses latest /tmp/codex-prompt-debug.*.

Options:
  --port <n>           Server port (default: 8788)
  --query-id <id>      Preselect query id
  --no-open            Do not launch browser; print URL only
  --help, -h           Show help
`);
}

function parseArgs(argv) {
  const args = {
    target: null,
    port: DEFAULT_PORT,
    queryId: null,
    openBrowser: true,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      usage();
      process.exit(0);
    }
    if (arg === "--port") {
      args.port = Number.parseInt(argv[i + 1] ?? "", 10);
      i += 1;
      continue;
    }
    if (arg === "--query-id") {
      args.queryId = argv[i + 1] ?? "";
      i += 1;
      continue;
    }
    if (arg === "--no-open") {
      args.openBrowser = false;
      continue;
    }
    if (!args.target) {
      args.target = path.resolve(arg);
      continue;
    }
    throw new Error(`unexpected argument: ${arg}`);
  }

  if (!Number.isFinite(args.port) || args.port <= 0) {
    throw new Error(`invalid --port value: ${args.port}`);
  }
  return args;
}

function openBrowser(url) {
  const commands =
    process.platform === "darwin"
      ? [["open", [url]]]
      : process.platform === "win32"
        ? [["cmd", ["/c", "start", "", url]]]
        : [["xdg-open", [url]]];

  for (const [cmd, cmdArgs] of commands) {
    try {
      const child = spawn(cmd, cmdArgs, {
        stdio: "ignore",
        detached: true,
      });
      child.unref();
      return true;
    } catch (_error) {
      // Try next opener.
    }
  }
  return false;
}

function waitForHealthy(port) {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + HEALTH_TIMEOUT_MS;

    function attempt() {
      const req = http.get(
        {
          hostname: "127.0.0.1",
          port,
          path: "/api/health",
          timeout: HEALTH_RETRY_MS,
        },
        (res) => {
          res.resume();
          if (res.statusCode === 200) {
            resolve();
            return;
          }
          if (Date.now() > deadline) {
            reject(new Error("timed out waiting for prompt-debug-inspector to start"));
            return;
          }
          setTimeout(attempt, HEALTH_RETRY_MS);
        },
      );

      req.on("error", () => {
        if (Date.now() > deadline) {
          reject(new Error("timed out waiting for prompt-debug-inspector to start"));
          return;
        }
        setTimeout(attempt, HEALTH_RETRY_MS);
      });

      req.on("timeout", () => {
        req.destroy();
      });
    }

    attempt();
  });
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const serverPath = path.join(__dirname, "server.js");
  const serverArgs = [serverPath, "--port", String(args.port)];
  if (args.target) {
    serverArgs.push("--target", args.target);
  }

  const server = spawn(process.execPath, serverArgs, { stdio: "inherit" });
  let exited = false;
  server.on("exit", () => {
    exited = true;
  });

  process.on("SIGINT", () => {
    if (!exited) {
      server.kill("SIGINT");
    }
    process.exit(130);
  });
  process.on("SIGTERM", () => {
    if (!exited) {
      server.kill("SIGTERM");
    }
    process.exit(143);
  });

  await waitForHealthy(args.port);

  const pageUrl = new URL(`http://127.0.0.1:${args.port}/`);
  if (args.target) {
    pageUrl.searchParams.set("target", args.target);
  }
  if (args.queryId) {
    pageUrl.searchParams.set("queryId", args.queryId);
  }

  if (args.openBrowser) {
    const opened = openBrowser(pageUrl.toString());
    if (!opened) {
      console.error("Could not launch browser automatically.");
      console.error(`Open this URL manually:\n${pageUrl.toString()}`);
    }
  } else {
    console.log(pageUrl.toString());
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
