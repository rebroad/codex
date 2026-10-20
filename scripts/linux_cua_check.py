#!/usr/bin/env python3
"""Check the configured Linux Computer Use MCP backend.

This is a read-only diagnostic. It does not launch or modify desktop apps.
Pass --screenshot to capture an existing window when one is available.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import select
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


CONFIG_ERROR = 2
DISPLAY_ERROR = 3
MCP_ERROR = 4
WINDOW_ERROR = 5


def fail(code: int, message: str) -> int:
    print(json.dumps({"status": "error", "code": code, "message": message}))
    return code


def capability_error_code(message: str) -> int:
    lowered = message.lower()
    capability_markers = (
        "display",
        "x11",
        "wayland",
        "at-spi",
        "accessibility",
        "xdg_runtime",
        "dbus",
    )
    return DISPLAY_ERROR if any(marker in lowered for marker in capability_markers) else MCP_ERROR


def read_config(path: Path) -> tuple[str, list[str], dict[str, str]]:
    try:
        with path.open("rb") as stream:
            config = tomllib.load(stream)
    except OSError as error:
        raise ValueError(f"cannot read config {path}: {error}") from error
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"invalid TOML in {path}: {error}") from error

    server = config.get("mcp_servers", {}).get("linux_cua")
    if not isinstance(server, dict):
        raise ValueError(f"{path} has no [mcp_servers.linux_cua] entry")
    command = server.get("command")
    args = server.get("args", [])
    env = server.get("env", {})
    if not isinstance(command, str) or not command:
        raise ValueError("linux_cua.command is missing")
    if not isinstance(args, list) or not all(isinstance(item, str) for item in args):
        raise ValueError("linux_cua.args must be an array of strings")
    if not isinstance(env, dict) or not all(
        isinstance(key, str) and isinstance(value, str) for key, value in env.items()
    ):
        raise ValueError("linux_cua.env must contain string values")
    return command, args, env


def read_response(stream: Any, timeout: float) -> dict[str, Any]:
    ready, _, _ = select.select([stream], [], [], timeout)
    if not ready:
        raise TimeoutError("timed out waiting for the CUA MCP server")
    line = stream.readline()
    if not line:
        raise RuntimeError("the CUA MCP server closed its output")
    try:
        response = json.loads(line)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"CUA MCP server returned invalid JSON: {line!r}") from error
    if not isinstance(response, dict):
        raise RuntimeError("CUA MCP server returned a non-object response")
    return response


def send(process: subprocess.Popen[str], message: dict[str, Any]) -> None:
    assert process.stdin is not None
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()


def call_js(
    process: subprocess.Popen[str], request_id: int, code: str, timeout: float
) -> str:
    send(
        process,
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": "js",
                "arguments": {"code": code, "title": "Check Linux Computer Use"},
            },
        },
    )
    response = read_response(process.stdout, timeout)
    if "error" in response:
        raise RuntimeError(f"MCP request failed: {response['error']}")
    result = response.get("result", {})
    if result.get("isError"):
        text = " ".join(
            item.get("text", "")
            for item in result.get("content", [])
            if isinstance(item, dict)
        )
        raise RuntimeError(text or "the CUA JavaScript call failed")
    text = "\n".join(
        item.get("text", "")
        for item in result.get("content", [])
        if isinstance(item, dict) and isinstance(item.get("text"), str)
    )
    return text


def run(config_path: Path, timeout: float, screenshot: bool) -> int:
    if platform.system() != "Linux":
        return fail(CONFIG_ERROR, "linux_cua_check requires Linux")
    try:
        command, args, configured_env = read_config(config_path)
    except ValueError as error:
        return fail(CONFIG_ERROR, str(error))

    display = os.environ.get("DISPLAY")
    wayland = os.environ.get("WAYLAND_DISPLAY")
    if not display and not wayland:
        return fail(
            DISPLAY_ERROR,
            "no DISPLAY or WAYLAND_DISPLAY is set; run from a graphical session",
        )
    if wayland and not display:
        return fail(
            DISPLAY_ERROR,
            "Wayland session detected; generic Linux CUA Wayland support is not validated; "
            "use an X11 display for this backend",
        )
    if wayland and not os.environ.get("XDG_RUNTIME_DIR"):
        return fail(DISPLAY_ERROR, "WAYLAND_DISPLAY is set but XDG_RUNTIME_DIR is missing")
    if not Path(command).is_file() or not os.access(command, os.X_OK):
        return fail(CONFIG_ERROR, f"linux_cua.command is not executable: {command}")

    environment = os.environ.copy()
    environment.update(configured_env)
    try:
        process = subprocess.Popen(
            [command, *args],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=environment,
        )
    except OSError as error:
        return fail(CONFIG_ERROR, f"could not start linux_cua: {error}")

    try:
        send(
            process,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "linux_cua_check", "version": "1"},
                },
            },
        )
        initialized = read_response(process.stdout, timeout)
        if "error" in initialized:
            return fail(MCP_ERROR, f"MCP initialize failed: {initialized['error']}")
        send(process, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})

        # The CUA REPL requires getState as the first JavaScript operation.
        state_text = call_js(
            process,
            2,
            "nodeRepl.write(JSON.stringify(await cua.getState({emit:false})))",
            timeout,
        )
        state = json.loads(state_text.strip().splitlines()[-1])
        if state.get("errors"):
            return fail(DISPLAY_ERROR, "; ".join(str(error) for error in state["errors"]))
        expression = """
let apps = await cua.listApps({emit:false});
let windows = apps.flatMap((app) => app.windows || []);
let result = {
  api: {
    listApps: typeof cua.listApps,
    launch_app: typeof cua.computer?.launch_app,
    getApp: typeof cua.getApp,
  },
  displayBackend: DISPLAY_BACKEND,
  appCount: apps.length,
  windowCount: windows.length,
  screenshotBytes: null,
  accessibilityBytes: null,
};
if (SCREENSHOT && windows.length > 0) {
  let app = await cua.getApp({windowId: windows[0].id});
  result.screenshotBytes = (await app.getScreenshot({emit:false})).length;
  result.accessibilityBytes = (await app.getAXState({emit:false})).length;
}
nodeRepl.write(JSON.stringify(result));
""".replace("SCREENSHOT", json.dumps(screenshot)).replace(
            "DISPLAY_BACKEND", json.dumps("x11" if display else "wayland")
        )
        payload = call_js(process, 3, expression, timeout)
        result = json.loads(payload.strip().splitlines()[-1])
        if result["api"] != {"listApps": "function", "launch_app": "function", "getApp": "function"}:
            return fail(MCP_ERROR, f"unexpected Linux CUA API surface: {result['api']}")
        if screenshot and result["windowCount"] == 0:
            return fail(WINDOW_ERROR, "no open window is available for screenshot verification")
        if screenshot and not result["screenshotBytes"]:
            return fail(MCP_ERROR, "Linux CUA returned no screenshot bytes")
        print(json.dumps({"status": "ok", **result}, sort_keys=True))
        return 0
    except (OSError, RuntimeError, TimeoutError, json.JSONDecodeError) as error:
        message = str(error)
        return fail(capability_error_code(message), message)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config", type=Path, default=Path.home() / ".codex" / "config.toml"
    )
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument(
        "--screenshot",
        action="store_true",
        help="also capture an existing window and accessibility state",
    )
    args = parser.parse_args()
    return run(args.config.expanduser(), args.timeout, args.screenshot)


if __name__ == "__main__":
    sys.exit(main())
