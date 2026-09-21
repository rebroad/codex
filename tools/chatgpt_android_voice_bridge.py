#!/usr/bin/env python3
"""Small Linux control panel for ChatGPT Android realtime voice.

This is intentionally a phone-dependent bridge.  The authenticated ChatGPT
session and WebRTC microphone/speaker remain in the Android app; ADB is used
only to launch it, press the voice control, and submit optional text guidance.
No ChatGPT cookie, API key, bearer token, or audio is copied to Linux.

The coordinate defaults match a 1080x2520 Flip7 portrait display.  Use the
coordinate options when controlling another device or a different layout.
"""

from __future__ import annotations

import argparse
import queue
import shutil
import subprocess
import threading
import time
import tkinter as tk
from tkinter import ttk


DEFAULT_SERIAL = "10.16.77.170:42293"


class Bridge:
    def __init__(self, serial: str, start_xy: tuple[int, int], compose_xy: tuple[int, int], send_xy: tuple[int, int]):
        self.serial = serial
        self.start_xy = start_xy
        self.compose_xy = compose_xy
        self.send_xy = send_xy
        self.events: queue.Queue[str] = queue.Queue()
        self.scrcpy: subprocess.Popen[bytes] | None = None

    def adb(self, *args: str, timeout: float = 15) -> str:
        result = subprocess.run(
            ["adb", "-s", self.serial, *args],
            check=True,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return result.stdout.strip()

    def launch(self) -> None:
        self.adb("shell", "monkey", "-p", "com.openai.chatgpt", "1")
        self.events.put("ChatGPT launched on the phone.")

    def tap(self, xy: tuple[int, int]) -> None:
        self.adb("shell", "input", "tap", str(xy[0]), str(xy[1]))

    def start_or_stop_voice(self) -> None:
        self.tap(self.start_xy)
        self.events.put("Voice control tapped; speak into the phone microphone or tap again to stop.")

    def send_text(self, prompt: str) -> None:
        prompt = prompt.strip()
        if not prompt:
            raise ValueError("Enter text guidance first.")
        self.tap(self.compose_xy)
        time.sleep(0.3)
        # Android's input command uses %s for a literal space.
        encoded = prompt.replace("%", "%25").replace(" ", "%s")
        self.adb("shell", "input", "text", encoded)
        time.sleep(0.3)
        self.tap(self.send_xy)
        self.events.put("Text guidance submitted to the active phone conversation.")

    def start_audio_forwarding(self) -> None:
        if not shutil.which("scrcpy"):
            raise RuntimeError("scrcpy is not installed; the phone will still handle its own audio.")
        if self.scrcpy and self.scrcpy.poll() is None:
            self.events.put("Audio forwarding is already running.")
            return
        self.scrcpy = subprocess.Popen(
            ["scrcpy", "--serial", self.serial, "--no-video", "--audio"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        self.events.put("scrcpy audio forwarding started (phone speaker to Linux output).")

    def close(self) -> None:
        if self.scrcpy and self.scrcpy.poll() is None:
            self.scrcpy.terminate()


class App:
    def __init__(self, root: tk.Tk, bridge: Bridge):
        self.root = root
        self.bridge = bridge
        self.root.title("ChatGPT Android realtime voice bridge")
        self.root.geometry("650x430")
        self.prompt = tk.StringVar()
        self.status = tk.StringVar(value="Ready. The phone owns authentication and audio.")

        frame = ttk.Frame(root, padding=16)
        frame.pack(fill="both", expand=True)
        ttk.Label(frame, text="ChatGPT Android realtime voice", font=(None, 16, "bold")).pack(anchor="w")
        ttk.Label(
            frame,
            text=("Phone-dependent mode: speak and listen on the Flip7. "
                  "Optional scrcpy forwarding sends phone playback to Linux."),
            wraplength=610,
        ).pack(anchor="w", pady=(6, 14))

        controls = ttk.Frame(frame)
        controls.pack(fill="x")
        ttk.Button(controls, text="Launch ChatGPT", command=lambda: self.run(self.bridge.launch)).pack(side="left")
        ttk.Button(controls, text="Start / stop voice", command=lambda: self.run(self.bridge.start_or_stop_voice)).pack(side="left", padx=8)
        ttk.Button(controls, text="Forward speaker to Linux", command=lambda: self.run(self.bridge.start_audio_forwarding)).pack(side="left")

        ttk.Label(frame, text="Guiding text for the active voice conversation:").pack(anchor="w", pady=(22, 4))
        entry = ttk.Entry(frame, textvariable=self.prompt)
        entry.pack(fill="x")
        entry.bind("<Return>", lambda _event: self.send())
        ttk.Button(frame, text="Send text guidance", command=self.send).pack(anchor="w", pady=8)

        ttk.Label(frame, textvariable=self.status, wraplength=610).pack(anchor="w", pady=(10, 6))
        ttk.Label(
            frame,
            text=("Allowance/cost: this path uses the ChatGPT Android account, not the OpenAI API key. "
                  "It therefore has no API token or API dollar meter; usage is subject to the ChatGPT plan."),
            wraplength=610,
        ).pack(anchor="w", pady=(8, 0))

        self.root.after(100, self.drain_events)
        self.root.protocol("WM_DELETE_WINDOW", self.close)

    def run(self, action) -> None:
        def worker() -> None:
            try:
                action()
            except Exception as error:  # UI boundary: report command/device failures.
                self.bridge.events.put(f"Error: {error}")

        threading.Thread(target=worker, daemon=True).start()

    def send(self) -> None:
        prompt = self.prompt.get()
        self.run(lambda: self.bridge.send_text(prompt))
        self.prompt.set("")

    def drain_events(self) -> None:
        try:
            while True:
                self.status.set(self.bridge.events.get_nowait())
        except queue.Empty:
            pass
        self.root.after(100, self.drain_events)

    def close(self) -> None:
        self.bridge.close()
        self.root.destroy()


def parse_xy(value: str) -> tuple[int, int]:
    try:
        x, y = (int(part) for part in value.split(",", 1))
    except ValueError as error:
        raise argparse.ArgumentTypeError("coordinates must be X,Y") from error
    return x, y


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", default=DEFAULT_SERIAL, help="ADB serial for the authenticated Android phone")
    parser.add_argument("--voice-xy", type=parse_xy, default=(960, 2200), help="voice button X,Y")
    parser.add_argument("--compose-xy", type=parse_xy, default=(300, 2200), help="voice text field X,Y")
    parser.add_argument("--send-xy", type=parse_xy, default=(960, 1385), help="send button X,Y after keyboard opens")
    args = parser.parse_args()

    root = tk.Tk()
    App(root, Bridge(args.serial, args.voice_xy, args.compose_xy, args.send_xy))
    root.mainloop()


if __name__ == "__main__":
    main()
