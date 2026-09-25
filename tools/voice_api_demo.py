#!/usr/bin/env python3
"""Demonstrate OpenAI realtime and ordinary audio APIs without exposing the key.

The four modes are:

  realtime-stt  Realtime transcription session -> text
  realtime-tts  Realtime conversation session -> 24 kHz PCM WAV
  audio-stt     POST /v1/audio/transcriptions -> text
  audio-tts     POST /v1/audio/speech -> an audio file

The default key source is ~/.codex/auth.json.d/API_KEY, whose JSON value is
expected to be under OPENAI_API_KEY. The key is never accepted as a command
line argument and is never printed.

The realtime modes use the public OpenAI Realtime WebSocket API directly. They
are deliberately separate from Codex app-server's thread/realtime/* protocol,
which additionally owns a Codex thread and handoff state.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import secrets
import socket
import ssl
import struct
import urllib.error
import urllib.parse
import urllib.request
import wave
from pathlib import Path


DEFAULT_KEY_FILE = Path.home() / ".codex" / "auth.json.d" / "API_KEY"
REALTIME_HOST = "api.openai.com"
REALTIME_PATH = "/v1/realtime"


def load_api_key(path: Path) -> str:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
        key = document["OPENAI_API_KEY"]
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise SystemExit(f"cannot read OPENAI_API_KEY from {path}: {exc}") from exc
    if not isinstance(key, str) or not key:
        raise SystemExit(f"OPENAI_API_KEY in {path} is not a non-empty string")
    return key


def json_request(url: str, key: str, body: bytes, content_type: str) -> bytes:
    request = urllib.request.Request(
        url,
        data=body,
        headers={
            "Authorization": f"Bearer {key}",
            "Content-Type": content_type,
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return response.read()
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise SystemExit(f"HTTP {exc.code} from {url}: {detail}") from exc
    except urllib.error.URLError as exc:
        raise SystemExit(f"request to {url} failed: {exc.reason}") from exc


class WebSocket:
    """Small client-side WebSocket implementation for this single API demo."""

    def __init__(self, key: str, model: str):
        self.socket = self._connect(key, model)

    @staticmethod
    def _connect(key: str, model: str) -> socket.socket:
        raw = socket.create_connection((REALTIME_HOST, 443), timeout=30)
        wrapped = ssl.create_default_context().wrap_socket(raw, server_hostname=REALTIME_HOST)
        nonce = base64.b64encode(secrets.token_bytes(16)).decode("ascii")
        target = f"{REALTIME_PATH}?{urllib.parse.urlencode({'model': model})}"
        request = (
            f"GET {target} HTTP/1.1\r\n"
            f"Host: {REALTIME_HOST}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {nonce}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            f"Authorization: Bearer {key}\r\n"
            "\r\n"
        ).encode("ascii")
        wrapped.sendall(request)
        response = read_until(wrapped, b"\r\n\r\n").decode("iso-8859-1")
        status = response.split("\r\n", 1)[0]
        if " 101 " not in status:
            wrapped.close()
            raise SystemExit(f"Realtime WebSocket handshake failed: {status}")
        headers = {
            name.strip().lower(): value.strip()
            for line in response.split("\r\n")[1:]
            if ":" in line
            for name, value in [line.split(":", 1)]
        }
        expected = base64.b64encode(
            hashlib.sha1((nonce + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        ).decode("ascii")
        if headers.get("sec-websocket-accept", "") != expected:
            wrapped.close()
            raise SystemExit("Realtime WebSocket handshake returned an invalid accept key")
        return wrapped

    def send_json(self, value: object) -> None:
        payload = json.dumps(value, separators=(",", ":")).encode("utf-8")
        self._send_frame(0x1, payload)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        length = len(payload)
        if length < 126:
            header = struct.pack("!BB", 0x80 | opcode, 0x80 | length)
        elif length < 65536:
            header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, 0x8000 | length)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, 0x8000000000000000 | length)
        mask = secrets.token_bytes(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.socket.sendall(header + mask + masked)

    def receive_json(self) -> dict:
        while True:
            first, second = read_exact(self.socket, 2)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", read_exact(self.socket, 2))[0]
            elif length == 127:
                length = struct.unpack("!Q", read_exact(self.socket, 8))[0]
            masked = second & 0x80
            mask = read_exact(self.socket, 4) if masked else b""
            payload = read_exact(self.socket, length)
            if masked:
                payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode == 0x8:
                raise SystemExit("Realtime WebSocket closed by server")
            if opcode != 0x1:
                continue
            event = json.loads(payload.decode("utf-8"))
            if event.get("type") == "error":
                raise SystemExit(f"Realtime API error: {json.dumps(event.get('error', event))}")
            return event

    def close(self) -> None:
        try:
            self._send_frame(0x8, b"")
        finally:
            self.socket.close()


def read_exact(sock: socket.socket, length: int) -> bytes:
    chunks = bytearray()
    while len(chunks) < length:
        chunk = sock.recv(length - len(chunks))
        if not chunk:
            raise SystemExit("connection closed while reading a WebSocket frame")
        chunks.extend(chunk)
    return bytes(chunks)


def read_until(sock: socket.socket, marker: bytes) -> bytes:
    data = bytearray()
    while marker not in data:
        chunk = sock.recv(4096)
        if not chunk:
            raise SystemExit("connection closed during WebSocket handshake")
        data.extend(chunk)
    return bytes(data)


def read_pcm16_wav(path: Path) -> bytes:
    try:
        with wave.open(str(path), "rb") as source:
            if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, 24000):
                raise SystemExit(f"{path} must be mono, 16-bit, 24 kHz PCM WAV")
            return source.readframes(source.getnframes())
    except (OSError, wave.Error) as exc:
        raise SystemExit(f"cannot read PCM WAV {path}: {exc}") from exc


def write_pcm16_wav(path: Path, audio: bytes) -> None:
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(24000)
        output.writeframes(audio)


def realtime_stt(key: str, audio_path: Path, model: str) -> None:
    audio = read_pcm16_wav(audio_path)
    websocket = WebSocket(key, model)
    try:
        websocket.send_json(
            {
                "type": "session.update",
                "session": {
                    "type": "transcription",
                    "audio": {
                        "input": {
                            "format": {"type": "audio/pcm", "rate": 24000},
                            "transcription": {"model": "gpt-live-transcribe"},
                            "turn_detection": None,
                        }
                    },
                },
            }
        )
        wait_for_event(websocket, "session.updated")
        for start in range(0, len(audio), 9600):
            websocket.send_json(
                {
                    "type": "input_audio_buffer.append",
                    "audio": base64.b64encode(audio[start : start + 9600]).decode("ascii"),
                }
            )
        websocket.send_json({"type": "input_audio_buffer.commit"})
        event = wait_for_event(websocket, "conversation.item.input_audio_transcription.completed")
        print(event["transcript"])
    finally:
        websocket.close()


def realtime_tts(key: str, text: str, output_path: Path, model: str, voice: str) -> None:
    websocket = WebSocket(key, model)
    audio = bytearray()
    transcript = []
    try:
        websocket.send_json(
            {
                "type": "session.update",
                "session": {
                    "type": "realtime",
                    "model": model,
                    "output_modalities": ["audio"],
                    "audio": {
                        "output": {"format": {"type": "audio/pcm", "rate": 24000}, "voice": voice}
                    },
                },
            }
        )
        wait_for_event(websocket, "session.updated")
        websocket.send_json(
            {
                "type": "conversation.item.create",
                "item": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}],
                },
            }
        )
        websocket.send_json({"type": "response.create"})
        while True:
            event = websocket.receive_json()
            event_type = event.get("type")
            if event_type == "response.output_audio.delta":
                audio.extend(base64.b64decode(event["delta"]))
            elif event_type == "response.output_audio_transcript.delta":
                transcript.append(event.get("delta", ""))
            elif event_type == "response.done":
                break
        write_pcm16_wav(output_path, bytes(audio))
        print(f"wrote {len(audio)} PCM bytes to {output_path}")
        if transcript:
            print("transcript:", "".join(transcript))
    finally:
        websocket.close()


def wait_for_event(websocket: WebSocket, expected_type: str) -> dict:
    while True:
        event = websocket.receive_json()
        if event.get("type") == expected_type:
            return event


def multipart_form(file_path: Path, model: str) -> tuple[bytes, str]:
    boundary = f"----codex-voice-{secrets.token_hex(12)}".encode("ascii")
    data = file_path.read_bytes()
    filename = file_path.name.encode("utf-8")
    body = b"".join(
        [
            b"--" + boundary + b"\r\n",
            b'Content-Disposition: form-data; name="model"\r\n\r\n',
            model.encode("ascii") + b"\r\n",
            b"--" + boundary + b"\r\n",
            b'Content-Disposition: form-data; name="file"; filename="' + filename + b'"\r\n',
            b"Content-Type: application/octet-stream\r\n\r\n",
            data,
            b"\r\n--" + boundary + b"--\r\n",
        ]
    )
    return body, f"multipart/form-data; boundary={boundary.decode('ascii')}"


def audio_stt(key: str, audio_path: Path, model: str) -> None:
    body, content_type = multipart_form(audio_path, model)
    result = json.loads(json_request("https://api.openai.com/v1/audio/transcriptions", key, body, content_type))
    print(result.get("text", result))
    if "usage" in result:
        print("usage:", json.dumps(result["usage"], sort_keys=True))


def audio_tts(key: str, text: str, output_path: Path, model: str, voice: str, response_format: str) -> None:
    body = json.dumps(
        {"model": model, "input": text, "voice": voice, "response_format": response_format}
    ).encode("utf-8")
    output_path.write_bytes(
        json_request("https://api.openai.com/v1/audio/speech", key, body, "application/json")
    )
    print(f"wrote {output_path.stat().st_size} bytes to {output_path}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("realtime-stt", "realtime-tts", "audio-stt", "audio-tts"))
    parser.add_argument("--key-file", type=Path, default=DEFAULT_KEY_FILE)
    parser.add_argument("--audio", type=Path, help="input audio file; realtime-stt requires 24 kHz mono PCM WAV")
    parser.add_argument("--text", help="input text for TTS modes")
    parser.add_argument("--output", type=Path, help="output audio file for TTS modes")
    parser.add_argument("--realtime-model", default="gpt-realtime-2.1")
    parser.add_argument("--audio-model", default=None)
    parser.add_argument("--voice", default="marin")
    parser.add_argument("--format", dest="response_format", default="mp3", choices=("mp3", "opus", "aac", "flac", "wav", "pcm"))
    args = parser.parse_args()
    key = load_api_key(args.key_file)

    if args.mode in ("realtime-stt", "audio-stt") and not args.audio:
        parser.error("--audio is required for speech-to-text modes")
    if args.mode in ("realtime-tts", "audio-tts") and not args.text:
        parser.error("--text is required for text-to-speech modes")
    if args.mode in ("realtime-tts", "audio-tts") and not args.output:
        parser.error("--output is required for text-to-speech modes")

    if args.mode == "realtime-stt":
        realtime_stt(key, args.audio, args.realtime_model)
    elif args.mode == "realtime-tts":
        realtime_tts(key, args.text, args.output, args.realtime_model, args.voice)
    elif args.mode == "audio-stt":
        audio_stt(key, args.audio, args.audio_model or "gpt-transcribe")
    else:
        audio_tts(key, args.text, args.output, args.audio_model or "gpt-4o-mini-tts", args.voice, args.response_format)


if __name__ == "__main__":
    main()
