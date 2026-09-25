# ChatGPT Android realtime voice bridge

`chatgpt_android_voice_bridge.py` is a Linux control panel for an already
authenticated ChatGPT Android installation. It deliberately keeps the
ChatGPT session, WebRTC media, microphone, and speaker on the phone. Linux
uses ADB for control and can optionally ask `scrcpy` to forward phone playback
to the Linux audio output.

Run it with the authorized phone connected:

```sh
python3 tools/chatgpt_android_voice_bridge.py \
  --serial 10.16.77.170:42293
```

The default coordinates are for the tested 1080x2520 Flip7 portrait layout.
Use `--voice-xy`, `--compose-xy`, and `--send-xy` for another layout. The GUI
instructs the user to say “Please reply with the word OK”. The phone's own
microphone and speaker are the reliable audio path; `scrcpy --audio` is an
optional phone-speaker-to-Linux path when scrcpy is installed. ADB does not
inject the Linux microphone into an unrooted Android app.

## Backend observations

The decompiled Android build uses a first-party authenticated realtime/app-
server channel rather than the public OpenAI API. The observed RPC envelope is
JSON-RPC 2.0 with an integer request id, method, and params. The realtime RPC
methods include:

- `thread/realtime/start`
- `thread/realtime/appendSpeech`
- `thread/realtime/appendText`
- `thread/realtime/stop`
- `thread/realtime/listVoices`

Notifications include `thread/realtime/started`, `itemAdded`, `closed`,
`transcript/done`, `sdp`, `transcript/delta`, and `error`. `appendText` carries
`threadId`, `text`, and a role such as `user`; `appendSpeech` carries a
`threadId` and text. The actual live audio is negotiated separately through
the Android WebRTC service, not sent as ordinary `/v1/audio` requests.

The app contains first-party bases including
`https://android.chat.openai.com/realtime/` and performs Play Integrity work
through `/backend-api/sentinel/chat-requirements` and
`/backend-api/playintegrity`. These endpoints are not a supported standalone
Linux ChatGPT API and the bridge does not extract or replay cookies, bearer
tokens, attestation results, or raw audio.

## S6 diagnosis

The S6's `test-keys`/`userdebug` state and logcat
`TRUST_KEYMASTER ... Device is compromized` evidence explain why it cannot
authenticate the same way as the locked, `release-keys`, green-state Flip7.
Patching the APK to skip its local check would not make the server accept an
invalid attestation. The dependable remedy is a stock, certified, locked
device; otherwise use the authenticated Flip7 as this bridge's dependency.

Usage and cost are governed by the ChatGPT account on the phone. No OpenAI API
key is read, no API token is consumed, and the API dollar cost is therefore
not measurable by this tool.
