#!/usr/bin/env python3
"""Small GUI demo for a Realtime microphone/speaker session.

Audio is raw signed 16-bit little-endian PCM, mono, 24 kHz.  By default the
GUI uses PipeWire's ``pw-record`` and ``pw-play`` commands.  The command
fields can be replaced with configured-device commands, or with a command
that reads/writes a named pipe.
"""

from __future__ import annotations

import argparse
import base64
import json
import queue
import shlex
import subprocess
import threading
import tkinter as tk
from pathlib import Path
from tkinter import messagebox, ttk

from voice_api_demo import WebSocket, load_api_key


RATE = 24_000
CHUNK_BYTES = 9_600  # 200 ms of mono s16 PCM
DEFAULT_INPUT = "pw-record --raw --format s16 --rate 24000 --channels 1 -"
DEFAULT_OUTPUT = "pw-play --raw --format s16 --rate 24000 --channels 1 -"
GUIDED_PHRASE = "Codex voice test. Please say the number forty two."

# USD per million tokens.  The audio rates are deliberately kept separate
# from text rates because Realtime usage reports them separately when it can.
RATES = {
    "gpt-realtime-1.5": (4.0, 16.0, 32.0, 64.0),
    "gpt-realtime-2": (4.0, 24.0, 32.0, 64.0),
    "gpt-realtime-mini": (0.60, 2.40, 10.0, 20.0),
}


class VoiceGui:
    def __init__(
        self,
        root: tk.Tk,
        model: str,
        key_path: Path,
        input_command: str = DEFAULT_INPUT,
        output_command: str = DEFAULT_OUTPUT,
        auto_start: bool = False,
        duration: float | None = None,
    ):
        self.root = root
        self.model = model
        self.key_path = key_path
        self.events: queue.Queue[tuple[str, object]] = queue.Queue()
        self.stop_event = threading.Event()
        self.worker: threading.Thread | None = None
        self.ws: WebSocket | None = None
        self.input_process: subprocess.Popen[bytes] | None = None
        self.output_process: subprocess.Popen[bytes] | None = None

        root.title("Codex Realtime Voice Demo")
        root.protocol("WM_DELETE_WINDOW", self.close)
        root.geometry("850x620")

        outer = ttk.Frame(root, padding=12)
        outer.pack(fill="both", expand=True)
        ttk.Label(outer, text="Realtime microphone ↔ speaker demo", font=(None, 16, "bold")).pack(anchor="w")
        ttk.Label(
            outer,
            text=(f'When Ready, say exactly: “{GUIDED_PHRASE}”  '
                  "The transcript and model audio will appear below."),
            wraplength=800,
        ).pack(anchor="w", pady=(6, 10))

        devices = ttk.LabelFrame(outer, text="Audio devices / pipes", padding=8)
        devices.pack(fill="x")
        self.input_command = tk.StringVar(value=input_command)
        self.output_command = tk.StringVar(value=output_command)
        ttk.Label(devices, text="Input command:").grid(row=0, column=0, sticky="w")
        ttk.Entry(devices, textvariable=self.input_command, width=92).grid(row=0, column=1, sticky="ew", padx=6)
        ttk.Label(devices, text="Output command:").grid(row=1, column=0, sticky="w", pady=(6, 0))
        ttk.Entry(devices, textvariable=self.output_command, width=92).grid(row=1, column=1, sticky="ew", padx=6, pady=(6, 0))
        ttk.Label(
            devices,
            text="Use --target DEVICE in either command, or replace a command with a reader/writer for a raw PCM pipe.",
            wraplength=760,
        ).grid(row=2, column=0, columnspan=2, sticky="w", pady=(6, 0))
        devices.columnconfigure(1, weight=1)

        controls = ttk.Frame(outer)
        controls.pack(fill="x", pady=10)
        self.start_button = ttk.Button(controls, text="Start realtime session", command=self.start)
        self.start_button.pack(side="left")
        self.stop_button = ttk.Button(controls, text="Stop", command=self.stop, state="disabled")
        self.stop_button.pack(side="left", padx=6)
        self.status = tk.StringVar(value="Stopped")
        ttk.Label(controls, textvariable=self.status).pack(side="left", padx=8)

        usage = ttk.LabelFrame(outer, text="API usage / cost", padding=8)
        usage.pack(fill="x")
        self.usage = tk.StringVar(value="No API usage yet. Local microphone/speaker time is not API usage.")
        ttk.Label(usage, textvariable=self.usage, wraplength=800, justify="left").pack(anchor="w")

        log_frame = ttk.LabelFrame(outer, text="Transcript and events", padding=8)
        log_frame.pack(fill="both", expand=True, pady=(10, 0))
        self.log = tk.Text(log_frame, height=18, width=100, state="disabled", wrap="word")
        self.log.pack(side="left", fill="both", expand=True)
        scrollbar = ttk.Scrollbar(log_frame, command=self.log.yview)
        scrollbar.pack(side="right", fill="y")
        self.log.configure(yscrollcommand=scrollbar.set)

        root.after(50, self.poll_events)
        if auto_start:
            root.after(150, self.start)
        if duration is not None:
            root.after(max(1, int(duration * 1000)), self.stop)

    def append_log(self, text: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", text.rstrip() + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def start(self) -> None:
        if self.worker and self.worker.is_alive():
            return
        try:
            load_api_key(self.key_path)
        except Exception as exc:
            messagebox.showerror("API key", str(exc))
            return
        try:
            input_command = shlex.split(self.input_command.get())
            output_command = shlex.split(self.output_command.get())
        except ValueError as exc:
            messagebox.showerror("Audio command", str(exc))
            return
        if not input_command or not output_command:
            messagebox.showerror("Audio command", "Both audio commands are required.")
            return

        self.stop_event.clear()
        self.start_button.configure(state="disabled")
        self.stop_button.configure(state="normal")
        self.status.set("Connecting…")
        self.append_log(f"Starting {self.model}; audio is 24 kHz mono PCM.")
        self.append_log(f"Test phrase: {GUIDED_PHRASE}")
        self.worker = threading.Thread(
            target=self.run_session,
            args=(input_command, output_command),
            daemon=True,
        )
        self.worker.start()

    def run_session(self, input_command: list[str], output_command: list[str]) -> None:
        try:
            key = load_api_key(self.key_path)
            self.ws = WebSocket(key, self.model)
            self.ws.send_json({
                "type": "session.update",
                "session": {
                    "type": "realtime",
                    "model": self.model,
                    "output_modalities": ["audio"],
                    "instructions": (
                        "You are a concise voice-test assistant. Repeat the user's words, then confirm "
                        "whether the number forty two was heard."
                    ),
                    "audio": {
                        "input": {
                            "format": {"type": "audio/pcm", "rate": RATE},
                            "transcription": {"model": "gpt-live-transcribe"},
                            "turn_detection": {"type": "semantic_vad"},
                        },
                        "output": {"format": {"type": "audio/pcm", "rate": RATE}, "voice": "marin"},
                    },
                },
            })
            self.events.put(("status", "Ready — speak the phrase shown above"))
            self.input_process = subprocess.Popen(
                input_command,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            self.output_process = subprocess.Popen(
                output_command,
                stdin=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            threading.Thread(target=self.capture_audio, daemon=True).start()
            while not self.stop_event.is_set():
                event = self.ws.receive_json()
                self.handle_event(event)
        except Exception as exc:
            self.events.put(("error", str(exc)))
        finally:
            self.stop_event.set()
            self.close_audio_processes()
            if self.ws:
                self.ws.close()
            self.ws = None
            self.events.put(("stopped", None))

    def capture_audio(self) -> None:
        process = self.input_process
        if not process or not process.stdout or not self.ws:
            return
        while not self.stop_event.is_set():
            chunk = process.stdout.read(CHUNK_BYTES)
            if not chunk:
                break
            try:
                self.ws.send_json({"type": "input_audio_buffer.append", "audio": base64.b64encode(chunk).decode()})
            except Exception as exc:
                self.events.put(("error", f"audio upload: {exc}"))
                break

    def handle_event(self, event: dict) -> None:
        event_type = event.get("type", "")
        if event_type in {"conversation.item.input_audio_transcription.delta", "conversation.item.input_audio_transcription.completed"}:
            text = event.get("delta") or event.get("transcript") or ""
            if text:
                self.events.put(("log", f"You: {text}" if event_type.endswith("completed") else f"You… {text}"))
        elif event_type == "response.output_audio.delta":
            audio = base64.b64decode(event.get("delta", ""))
            if self.output_process and self.output_process.stdin:
                self.output_process.stdin.write(audio)
                self.output_process.stdin.flush()
        elif event_type == "response.output_audio_transcript.delta":
            if event.get("delta"):
                self.events.put(("log", f"Assistant… {event['delta']}"))
        elif event_type == "response.output_audio_transcript.done":
            if event.get("transcript"):
                self.events.put(("log", f"Assistant: {event['transcript']}"))
        elif event_type == "response.done":
            self.update_usage(event.get("response", {}).get("usage"))
        elif event_type == "error":
            self.events.put(("error", json.dumps(event.get("error", event))))

    def update_usage(self, usage: object) -> None:
        if not isinstance(usage, dict):
            self.events.put(("usage", "Response completed; this response did not include token usage."))
            return
        input_tokens = int(usage.get("input_tokens", 0) or 0)
        output_tokens = int(usage.get("output_tokens", 0) or 0)
        input_details = usage.get("input_token_details") or {}
        output_details = usage.get("output_token_details") or {}
        text_in = int(input_details.get("text_tokens", 0) or 0)
        audio_in = int(input_details.get("audio_tokens", 0) or 0)
        text_out = int(output_details.get("text_tokens", 0) or 0)
        audio_out = int(output_details.get("audio_tokens", 0) or 0)
        # If detail fields are absent, retain the total but do not pretend the
        # total can be priced accurately across text and audio rates.
        rates = RATES.get(self.model)
        if rates and (text_in or audio_in or text_out or audio_out):
            cost = (text_in * rates[0] + text_out * rates[1] + audio_in * rates[2] + audio_out * rates[3]) / 1_000_000
            text = (f"Reported usage: {input_tokens:,} input + {output_tokens:,} output tokens "
                    f"(text in/out {text_in:,}/{text_out:,}; audio in/out {audio_in:,}/{audio_out:,}). "
                    f"Estimated API cost for this response: ${cost:.6f} using {self.model} rates. "
                    "Final billing may differ; local audio I/O is not included.")
        else:
            text = (f"Reported usage: {input_tokens:,} input + {output_tokens:,} output tokens. "
                    "Exact audio/text cost is unavailable because the response omitted token details.")
        self.events.put(("usage", text))

    def poll_events(self) -> None:
        try:
            while True:
                kind, value = self.events.get_nowait()
                if kind == "status":
                    self.status.set(str(value))
                elif kind == "log":
                    self.append_log(str(value))
                elif kind == "usage":
                    self.usage.set(str(value))
                elif kind == "error":
                    self.status.set("Error")
                    self.append_log(f"ERROR: {value}")
                elif kind == "stopped":
                    self.status.set("Stopped")
                    self.start_button.configure(state="normal")
                    self.stop_button.configure(state="disabled")
        except queue.Empty:
            pass
        self.root.after(50, self.poll_events)

    def stop(self) -> None:
        self.stop_event.set()
        if self.ws:
            self.ws.close()
        self.close_audio_processes()

    def close_audio_processes(self) -> None:
        for process in (self.input_process, self.output_process):
            if process and process.poll() is None:
                process.terminate()
        self.input_process = None
        self.output_process = None

    def close(self) -> None:
        self.stop()
        self.root.destroy()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", default="gpt-realtime-1.5")
    parser.add_argument("--key-file", type=Path, default=Path.home() / ".codex/auth.json.d/API_KEY")
    parser.add_argument("--input-command", default=DEFAULT_INPUT, help="raw PCM capture command")
    parser.add_argument("--output-command", default=DEFAULT_OUTPUT, help="raw PCM playback command")
    parser.add_argument("--auto-start", action="store_true", help="start immediately; useful for smoke tests")
    parser.add_argument("--duration", type=float, help="stop after this many seconds")
    args = parser.parse_args()
    root = tk.Tk()
    VoiceGui(root, args.model, args.key_file, args.input_command, args.output_command, args.auto_start, args.duration)
    root.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
