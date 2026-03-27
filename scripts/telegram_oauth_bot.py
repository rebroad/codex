#!/usr/bin/env python3
"""
Telegram bot service for Codex device-auth login.

Flow:
1. User sends any message.
2. If per-user auth is valid, bot forwards the message to `codex exec --bare-prompt`.
3. If auth is missing and no login started in last hour, bot runs:
   codex --auth-file <file> tlogin start --user-id <chat_id> --json
   and returns verification URL + one-time code.
4. When user sends any message again, bot attempts completion in background:
   codex --auth-file <file> tlogin complete --user-id <chat_id>
"""

from __future__ import annotations

import argparse
import json
import tempfile
import re
import os
import tomllib
import subprocess
import threading
import time
import traceback
import urllib.error
import urllib.request
from pathlib import Path

LOGIN_COOLDOWN_SECONDS = 3600
COMPLETE_ATTEMPT_TIMEOUT_SECONDS = 8
EXEC_TIMEOUT_SECONDS = 300
POLL_TIMEOUT_SECONDS = 20
POLL_HTTP_TIMEOUT_SECONDS = 35
TRANSIENT_LOG_INTERVAL_SECONDS = 60


def telegram_request(
    token: str,
    method: str,
    payload: dict,
    timeout_seconds: int = POLL_HTTP_TIMEOUT_SECONDS,
) -> dict:
    url = f"https://api.telegram.org/bot{token}/{method}"
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout_seconds) as resp:
        parsed = json.loads(resp.read().decode("utf-8"))
        if not parsed.get("ok"):
            raise RuntimeError(f"telegram api error for {method}: {parsed}")
        return parsed


def send_message(token: str, chat_id: int, text: str) -> None:
    telegram_request(
        token,
        "sendMessage",
        {
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": True,
        },
    )


def is_transient_telegram_error(exc: Exception) -> bool:
    if isinstance(exc, TimeoutError):
        return True
    if isinstance(exc, urllib.error.URLError):
        return True
    if isinstance(exc, OSError):
        return True
    return False


class PendingStore:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self.pending_by_chat: dict[int, dict] = {}
        self.last_started_at: dict[int, int] = {}
        self.completing_chats: set[int] = set()

    def put(self, chat_id: int, payload: dict) -> None:
        with self._lock:
            self.pending_by_chat[chat_id] = payload
            self.last_started_at[chat_id] = int(time.time())

    def get(self, chat_id: int) -> dict | None:
        with self._lock:
            return self.pending_by_chat.get(chat_id)

    def pop(self, chat_id: int) -> dict | None:
        with self._lock:
            return self.pending_by_chat.pop(chat_id, None)

    def seconds_until_login_allowed(self, chat_id: int) -> int:
        with self._lock:
            last_started = self.last_started_at.get(chat_id)
        if last_started is None:
            return 0
        elapsed = int(time.time()) - last_started
        remaining = LOGIN_COOLDOWN_SECONDS - elapsed
        return remaining if remaining > 0 else 0

    def mark_completing(self, chat_id: int) -> bool:
        with self._lock:
            if chat_id in self.completing_chats:
                return False
            self.completing_chats.add(chat_id)
            return True

    def unmark_completing(self, chat_id: int) -> None:
        with self._lock:
            self.completing_chats.discard(chat_id)

    def is_completing(self, chat_id: int) -> bool:
        with self._lock:
            return chat_id in self.completing_chats


def run_codex(
    cmd: list[str],
    timeout_seconds: int | None = None,
    env_overrides: dict[str, str] | None = None,
    cwd: str | None = None,
) -> str:
    env = os.environ.copy()
    if env_overrides:
        env.update(env_overrides)
    completed = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=timeout_seconds,
        env=env,
        cwd=cwd,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(cmd)}\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )
    return completed.stdout


def sanitize_user_key(raw: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]", "_", raw).strip("._-") or "unknown"


def auth_file_for_user(auth_root: Path, user_key: str) -> Path:
    return auth_root / f"{sanitize_user_key(user_key)}.auth.json"


def codex_home_for_user(auth_root: Path, user_key: str) -> Path:
    return auth_root / "homes" / sanitize_user_key(user_key)


def codex_workspace_for_user(auth_root: Path, user_key: str) -> Path:
    return codex_home_for_user(auth_root, user_key) / "workspace"


def codex_env_for_user(auth_root: Path, user_key: str) -> dict[str, str]:
    return {"CODEX_HOME": str(codex_home_for_user(auth_root, user_key))}


def find_parent_config_for_user(user_home: Path) -> Path | None:
    boundary = (Path.home() / ".codex").resolve()
    current = user_home.resolve()
    if not current.is_relative_to(boundary):
        return None
    while True:
        candidate = current / "config.toml"
        if candidate.exists():
            return candidate
        if current == boundary:
            return None
        parent = current.parent.resolve()
        if parent == current:
            return None
        if not parent.is_relative_to(boundary):
            return None
        current = parent


def resolve_user_runtime_context(auth_root: Path, user_key: str) -> tuple[Path | None, dict[str, str]]:
    user_home = codex_home_for_user(auth_root, user_key)
    user_home.mkdir(parents=True, exist_ok=True)
    source_config = find_parent_config_for_user(user_home)
    env_overrides: dict[str, str] = {}
    if source_config is None:
        return None, env_overrides

    try:
        config_data = tomllib.loads(source_config.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"[telegram] warning: failed to parse config {source_config}: {exc}")
        return source_config.parent, env_overrides

    prompt_debug = config_data.get("prompt_debug_http")
    if isinstance(prompt_debug, dict):
        capture_dir = prompt_debug.get("capture_dir")
        if isinstance(capture_dir, str) and "$user" in capture_dir:
            env_overrides["CODEX_BACKEND_CAPTURE_DIR"] = capture_dir.replace(
                "$user", f"tg-{sanitize_user_key(user_key)}"
            )

    return source_config.parent, env_overrides


def user_key_from_message(message: dict) -> str:
    from_user = message.get("from") or {}
    username = (from_user.get("username") or "").strip()
    if username:
        return username
    user_id = from_user.get("id")
    if user_id is not None:
        return f"id-{user_id}"
    return "unknown"


def tlogin_user_id(bot_id: int, chat_id: int) -> str:
    return f"{bot_id}:{chat_id}"


def has_valid_auth(codex_bin: str, auth_file: Path, auth_root: Path, user_key: str) -> bool:
    runtime_cwd, runtime_env = resolve_user_runtime_context(auth_root, user_key)
    env = codex_env_for_user(auth_root, user_key)
    env.update(runtime_env)
    cmd = [codex_bin, "--auth-file", str(auth_file), "login", "status"]
    completed = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        env={**os.environ, **env},
        cwd=str(runtime_cwd) if runtime_cwd else None,
    )
    return completed.returncode == 0


def should_force_new_code(text: str) -> bool:
    lowered = text.strip().lower()
    return lowered in {
        "/start",
        "start",
        "retry",
        "new code",
        "newcode",
        "again",
    }


def start_login_for_chat(
    token: str,
    bot_id: int,
    chat_id: int,
    user_key: str,
    codex_bin: str,
    auth_root: Path,
    store: PendingStore,
) -> None:
    auth_root.mkdir(parents=True, exist_ok=True)
    runtime_cwd, runtime_env = resolve_user_runtime_context(auth_root, user_key)
    env = codex_env_for_user(auth_root, user_key)
    env.update(runtime_env)
    auth_file = auth_file_for_user(auth_root, user_key)
    cmd = [
        codex_bin,
        "--auth-file",
        str(auth_file),
        "tlogin",
        "start",
        "--user-id",
        tlogin_user_id(bot_id, chat_id),
        "--json",
    ]
    try:
        output = run_codex(
            cmd,
            env_overrides=env,
            cwd=str(runtime_cwd) if runtime_cwd else None,
        )
        parsed = json.loads(output)
    except Exception as exc:
        print(f"[telegram] failed to start login for chat {chat_id}: {exc}")
        send_message(
            token,
            chat_id,
            "Failed to start login right now. Please try again in a moment.",
        )
        return

    store.put(chat_id, {"auth_file": str(auth_file), "user_key": user_key})
    verification_url = parsed.get("verificationUrl", "")
    user_code = parsed.get("userCode", "")
    send_message(
        token,
        chat_id,
        "Please complete Codex sign-in:\n\n"
        f"1) Open: {verification_url}\n"
        f"2) Enter code: {user_code}\n\n"
        "When done, send any message here and I will finish login.",
    )


def split_telegram_message(text: str, limit: int = 3900) -> list[str]:
    if len(text) <= limit:
        return [text]
    chunks = []
    remaining = text
    while len(remaining) > limit:
        split_at = remaining.rfind("\n", 0, limit)
        if split_at <= 0:
            split_at = limit
        chunks.append(remaining[:split_at])
        remaining = remaining[split_at:].lstrip("\n")
    if remaining:
        chunks.append(remaining)
    return chunks


def send_long_message(token: str, chat_id: int, text: str) -> None:
    for chunk in split_telegram_message(text):
        send_message(token, chat_id, chunk)


def run_exec_prompt(
    codex_bin: str,
    auth_file: Path,
    auth_root: Path,
    user_key: str,
    prompt: str,
    system_prompt: str | None,
) -> str:
    runtime_cwd, runtime_env = resolve_user_runtime_context(auth_root, user_key)
    workspace = codex_workspace_for_user(auth_root, user_key)
    workspace.mkdir(parents=True, exist_ok=True)
    env = codex_env_for_user(auth_root, user_key)
    env.update(runtime_env)
    with tempfile.NamedTemporaryFile(mode="w+", encoding="utf-8", delete=True) as out_file:
        resume_cmd = [
            codex_bin,
            "--auth-file",
            str(auth_file),
            "exec",
            "--bare-prompt",
            "--skip-git-repo-check",
            "--cd",
            str(workspace),
            "--output-last-message",
            out_file.name,
        ]
        if system_prompt:
            resume_cmd.extend(["--system", system_prompt])
        resume_cmd.extend([
            "resume",
            "--last",
            "--all",
            prompt,
        ])
        try:
            run_codex(
                resume_cmd,
                timeout_seconds=EXEC_TIMEOUT_SECONDS,
                env_overrides=env,
                cwd=str(runtime_cwd) if runtime_cwd else None,
            )
        except Exception:
            fresh_cmd = [
                codex_bin,
                "--auth-file",
                str(auth_file),
                "exec",
                "--bare-prompt",
                "--skip-git-repo-check",
                "--cd",
                str(workspace),
                "--output-last-message",
                out_file.name,
            ]
            if system_prompt:
                fresh_cmd.extend(["--system", system_prompt])
            fresh_cmd.append(prompt)
            run_codex(
                fresh_cmd,
                timeout_seconds=EXEC_TIMEOUT_SECONDS,
                env_overrides=env,
                cwd=str(runtime_cwd) if runtime_cwd else None,
            )
        out_file.seek(0)
        result = out_file.read().strip()
        if not result:
            return "No response was returned."
        return result


def complete_login_for_chat(
    token: str,
    bot_id: int,
    chat_id: int,
    codex_bin: str,
    pending: dict,
    auth_root: Path,
    store: PendingStore,
) -> None:
    if not store.mark_completing(chat_id):
        return

    def _worker() -> None:
        try:
            auth_file = pending["auth_file"]
            user_key = pending["user_key"]
            runtime_cwd, runtime_env = resolve_user_runtime_context(auth_root, user_key)
            env = codex_env_for_user(auth_root, user_key)
            env.update(runtime_env)
            cmd = [
                codex_bin,
                "--auth-file",
                auth_file,
                "tlogin",
                "complete",
                "--user-id",
                tlogin_user_id(bot_id, chat_id),
            ]
            run_codex(
                cmd,
                timeout_seconds=COMPLETE_ATTEMPT_TIMEOUT_SECONDS,
                env_overrides=env,
                cwd=str(runtime_cwd) if runtime_cwd else None,
            )
            store.pop(chat_id)
            send_message(
                token,
                chat_id,
                "Login complete.",
            )
        except subprocess.TimeoutExpired:
            send_message(
                token,
                chat_id,
                "Login not ready yet. Please complete browser approval, then send any message again.",
            )
        except Exception as exc:
            print(f"[telegram] login completion failed for chat {chat_id}: {exc}")
            send_message(
                token,
                chat_id,
                "Login is not complete yet (or failed). "
                "Please finish verification and send another message. "
                "If needed, send /start to request a fresh code.",
            )
        finally:
            store.unmark_completing(chat_id)

    threading.Thread(target=_worker, daemon=True).start()


def run_bot_loop(
    token: str,
    bot_id: int,
    bot_username: str,
    codex_bin: str,
    auth_root: Path,
    store: PendingStore,
    stop_event: threading.Event,
    system_prompt: str | None,
) -> None:
    offset = 0
    next_transient_log_at = 0.0
    while not stop_event.is_set():
        try:
            updates = telegram_request(
                token,
                "getUpdates",
                {"timeout": POLL_TIMEOUT_SECONDS, "offset": offset},
            )["result"]
        except Exception as exc:
            if stop_event.is_set():
                return
            if is_transient_telegram_error(exc):
                now = time.time()
                if now >= next_transient_log_at:
                    print(
                        f"[telegram:@{bot_username}] transient getUpdates error: {exc}; retrying"
                    )
                    next_transient_log_at = now + TRANSIENT_LOG_INTERVAL_SECONDS
                time.sleep(3)
                continue
            print(f"[telegram:@{bot_username}] non-transient getUpdates error, retrying in 10s")
            traceback.print_exc()
            time.sleep(10)
            continue

        for update in updates:
            offset = max(offset, update["update_id"] + 1)
            message = update.get("message")
            if not message:
                continue
            chat_id = message.get("chat", {}).get("id")
            if chat_id is None:
                continue
            user_key = user_key_from_message(message)
            text = (message.get("text") or "").strip()
            if not text:
                continue

            if text == "/help":
                send_message(
                    token,
                    chat_id,
                    "Codex login bot. Send any message to start or complete login.",
                )
                continue

            auth_file = auth_file_for_user(auth_root, user_key)
            if has_valid_auth(codex_bin, auth_file, auth_root, user_key):
                try:
                    reply = run_exec_prompt(
                        codex_bin, auth_file, auth_root, user_key, text, system_prompt
                    )
                    send_long_message(token, chat_id, reply)
                except subprocess.TimeoutExpired:
                    send_message(
                        token,
                        chat_id,
                        "Request timed out. Please try a shorter prompt.",
                    )
                except Exception as exc:
                    print(
                        f"[telegram:@{bot_username}] exec failed for user {user_key} chat {chat_id}: {exc}"
                    )
                    send_message(
                        token,
                        chat_id,
                        "Sorry, I could not process that just now. Please try again.",
                    )
                continue

            if should_force_new_code(text):
                start_login_for_chat(
                    token, bot_id, chat_id, user_key, codex_bin, auth_root, store
                )
                continue

            pending = store.get(chat_id)
            if pending is None:
                cooldown = store.seconds_until_login_allowed(chat_id)
                if cooldown > 0:
                    send_message(
                        token,
                        chat_id,
                        f"A login was started recently. "
                        f"Please complete it or wait about {cooldown} seconds before retry.",
                    )
                    continue
                start_login_for_chat(
                    token, bot_id, chat_id, user_key, codex_bin, auth_root, store
                )
                continue

            if store.is_completing(chat_id):
                send_message(
                    token,
                    chat_id,
                    "Still checking login completion. Please wait a moment.",
                )
                continue

            send_message(
                token,
                chat_id,
                "Checking login completion now...",
            )
            complete_login_for_chat(
                token, bot_id, chat_id, codex_bin, pending, auth_root, store
            )


def load_tokens(token_file: Path) -> list[str]:
    if not token_file.exists():
        raise SystemExit(f"token file not found: {token_file}")
    raw = token_file.read_text(encoding="utf-8")
    tokens = []
    for line in raw.splitlines():
        token = line.strip()
        if not token or token.startswith("#"):
            continue
        tokens.append(token)
    if not tokens:
        raise SystemExit(f"token file has no tokens: {token_file}")
    return tokens


def reset_webhook(token: str, bot_username: str) -> None:
    try:
        telegram_request(
            token,
            "deleteWebhook",
            {"drop_pending_updates": True},
        )
        print(f"Reset Telegram webhook for @{bot_username} (polling mode).")
    except Exception as exc:
        raise RuntimeError(f"failed to reset webhook for @{bot_username}: {exc}") from exc


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--token-file",
        default=str(Path.home() / ".codex" / "telegram.token"),
    )
    parser.add_argument("--codex-bin", default="codex")
    parser.add_argument(
        "--auth-root",
        default=str(Path.home() / ".codex" / "telegram-auth"),
    )
    parser.add_argument("--system-prompt", default=None)
    parser.add_argument("--system-prompt-file", default=None)
    args = parser.parse_args()

    system_prompt = args.system_prompt
    if args.system_prompt_file:
        system_prompt = Path(args.system_prompt_file).read_text(encoding="utf-8").strip()

    token_file = Path(args.token_file)
    tokens = load_tokens(token_file)

    auth_root = Path(args.auth_root)
    workers = []
    stop_event = threading.Event()
    for token in tokens:
        me = telegram_request(token, "getMe", {})["result"]
        bot_id = int(me.get("id"))
        bot_username = me.get("username") or f"bot-{bot_id}"
        reset_webhook(token, bot_username)
        print(f"Telegram bot @{bot_username} running.")

        store = PendingStore()
        worker = threading.Thread(
            target=run_bot_loop,
            kwargs={
                "token": token,
                "bot_id": bot_id,
                "bot_username": bot_username,
                "codex_bin": args.codex_bin,
                "auth_root": auth_root,
                "store": store,
                "stop_event": stop_event,
                "system_prompt": system_prompt,
            },
            daemon=True,
        )
        worker.start()
        workers.append(worker)

    try:
        while any(worker.is_alive() for worker in workers):
            for worker in workers:
                worker.join(timeout=0.5)
    except KeyboardInterrupt:
        print("\nShutting down Telegram bot worker...")
        stop_event.set()
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
