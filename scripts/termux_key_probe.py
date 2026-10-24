#!/usr/bin/env python3
"""Show the key sequences a terminal sends, useful for diagnosing Termux input."""

import os
import select
import sys
import termios
import tty


ARROWS = {
    ord("A"): "Up",
    ord("B"): "Down",
    ord("C"): "Right",
    ord("D"): "Left",
}
MODIFIERS = {
    2: ("Shift",),
    3: ("Alt",),
    4: ("Shift", "Alt"),
    5: ("Ctrl",),
    6: ("Shift", "Ctrl"),
    7: ("Alt", "Ctrl"),
    8: ("Shift", "Alt", "Ctrl"),
}
SEQUENCE_TIMEOUT = 0.25


def describe(data: bytes) -> str:
    """Name common xterm arrow encodings while preserving the raw bytes below."""
    if data in (b"\x1b[A", b"\x1bOA"):
        return "Up"
    if data in (b"\x1b[B", b"\x1bOB"):
        return "Down"
    if data in (b"\x1b[C", b"\x1bOC"):
        return "Right"
    if data in (b"\x1b[D", b"\x1bOD"):
        return "Left"

    if data.startswith(b"\x1b[1;") and len(data) == 6:
        try:
            modifier = int(chr(data[4]))
        except ValueError:
            return "unrecognized sequence"
        arrow = ARROWS.get(data[5])
        if arrow is not None and modifier in MODIFIERS:
            return "+".join((*MODIFIERS[modifier], arrow))

    names = {b"\x03": "Ctrl+C", b"\x04": "Ctrl+D", b"\r": "Enter", b"\t": "Tab", b"\x7f": "Backspace"}
    if data in names:
        return names[data]
    if len(data) == 1 and data[0] == 0x1B:
        return "Escape"
    return "unrecognized sequence"


def read_sequence(fd: int) -> bytes:
    """Read one key sequence, ending after a short pause in terminal input."""
    data = bytearray(os.read(fd, 1))
    while select.select([fd], [], [], SEQUENCE_TIMEOUT)[0]:
        data.extend(os.read(fd, 1))
    return bytes(data)


def main() -> int:
    if not sys.stdin.isatty():
        print("Run this utility directly in a terminal; stdin is not a TTY.", file=sys.stderr)
        return 2

    fd = sys.stdin.fileno()
    original = termios.tcgetattr(fd)
    print(
        "Press a key or chord, then pause briefly to see what Termux sent.\n"
        "For the extra-keys row, tap SHIFT and then LEFT to test Shift+Left.\n"
        "Press Escape alone to quit.",
        flush=True,
    )
    try:
        tty.setraw(fd)
        while True:
            if not select.select([fd], [], [fd])[0]:
                continue
            data = read_sequence(fd)
            label = describe(data)
            raw = repr(data)
            hex_bytes = " ".join(f"{byte:02x}" for byte in data)
            os.write(sys.stdout.fileno(), f"{label}: {raw}  [hex: {hex_bytes}]\r\n".encode())
            if data == b"\x1b":
                break
    except KeyboardInterrupt:
        pass
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, original)
        print("Terminal settings restored.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
