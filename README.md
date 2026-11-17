![Lumbridge Speak: Text to speech you host yourself.](docs/assets/banner.jpg)

# Lumbridge Speak

Text to speech: one small Rust program that is both an OpenAI-style `/v1/audio/speech`
server and the `speak` command. It is for people who run their own speech model and want
a plain command and API in front of it. It writes numbers, money, times, versions and
paths as words before the model sees them, so "$0.19" is said as "nineteen cents".

Part of [Lumbridge](https://lumbridgecorp.com) ([all repos](https://github.com/lumbridge-corp)). In daily use.

```console
$ speak text "PR #73 merged at 4:05 PM, took 3m 12s and cost \$0.19"
PR number seventy-three merged at four oh five P M, took three minutes twelve seconds and cost nineteen cents
$ speak Hello from Lumbridge Speak.
$ speak voices
```

## Status

- **Version:** 0.1.0. The command line and settings can change until 1.0.
- **Runs on:** macOS and Linux. Playback with `afplay` on a Mac and `pw-play`, `paplay`
  or `aplay` on Linux.
- **Not yet:**
  - One model adapter: NVIDIA Magpie TTS on NeMo-Speech.cpp. Other models need a new
    adapter (`src/adapter.rs`).
  - No voice cloning; Magpie's five built-in voices only.
  - No download: you build it from source. The installer is for macOS; on Linux you copy
    the binary and an optional user unit by hand.
  - Numbers as words is English only.

## Install

You need Rust 1.85 or newer. `ffmpeg` is needed for every audio format except `wav`.

On Ubuntu 24.04:

```sh
sudo apt install build-essential pkg-config ffmpeg pipewire-bin
git clone https://github.com/lumbridge-corp/lumbridge-speak.git
cd lumbridge-speak
cargo build --release --locked
install -Dm755 target/release/lumbridge-speak ~/.local/bin/lumbridge-speak
ln -sf lumbridge-speak ~/.local/bin/speak
# optional: run the front as a user service
install -Dm644 deploy/linux/lumbridge-speak.service ~/.config/systemd/user/lumbridge-speak.service
systemctl --user daemon-reload && systemctl --user enable --now lumbridge-speak
```

To remove it: `systemctl --user disable --now lumbridge-speak`, then delete
`~/.local/bin/lumbridge-speak`, `~/.local/bin/speak`, the unit file and
`~/.config/lumbridge-speak/`.

On macOS (with Rust and, if you want other formats, `ffmpeg` from Homebrew):

```sh
deploy/mac/install.sh            # build, install, start; safe to run again
deploy/mac/install.sh --remove   # the settings file stays
```

The Mac installer puts `lumbridge-speak` and the `speak` link in `~/.local/bin`, writes
`~/.config/lumbridge-speak/config.toml` from the example if there is none (an existing
one is never changed), refuses to overwrite a `speak` that is not this one, and runs the
front as the LaunchAgent `com.lumbridgecorp.lumbridge-speak` (log
`~/Library/Logs/lumbridge-speak.log`).

## Quick start

No model needed for these:

```sh
speak text "v0.3.1 is out, 32GB free, call 415-555-0142"   # the words that would be spoken
LUMBRIDGE_SPEAK_ADAPTER=mock lumbridge-speak &               # the front with a stand-in model
curl -s http://127.0.0.1:8813/health
```

To hear speech you need a Magpie TTS server. [docs/MODEL-SERVER.md](docs/MODEL-SERVER.md)
stands one up on the same machine in four steps. Then, with the front running:

```sh
speak Hello from Lumbridge Speak.          # plays here, sentence by sentence
echo "The build is done." | speak          # text from stdin
speak --voice Jason --rate 1.1 Good morning.
speak --out hello.wav Hello.               # a file instead of sound
speak demo                                 # hear every voice, one after another
speak stop                                 # stop what is playing
```

One `speak` plays at a time on a machine; a second one waits its turn.

## Configuration

Settings come from built-in defaults, then a TOML file, then `LUMBRIDGE_SPEAK_*`
environment variables. Every setting is in
[lumbridge-speak.example.toml](lumbridge-speak.example.toml); `lumbridge-speak --print-config`
shows what is in effect.

| What | Path |
|---|---|
| Config | `~/.config/lumbridge-speak/config.toml` (or `--config FILE`, `$LUMBRIDGE_SPEAK_CONFIG`) |

The settings most people change:

- `[magpie] url`: where the model server is (default `http://127.0.0.1:8808`).
- `listen`: where the front listens (default `127.0.0.1:8813`).
- `default_voice`: `John`, `Sofia`, `Aria`, `Jason` or `Leo` (default `Leo`).
- `level`: scale each answer to a fixed peak (0 to 1). Magpie's own output is quiet and
  uneven.
- `[say]`: how your own names are said (`"PostgreSQL" = "post gress Q L"`), and `drop`:
  patterns never read aloud.
- `[client]`: the front the `speak` command asks, its voice and rate, and
  `[client.targets.NAME]` for `speak --on NAME` (another machine that speaks for you).

## Numbers as words

Magpie has no text normalizer. Plain prose comes back word for word when spoken and
transcribed again, but written forms come back as noise: "PR 73 merged at 4:05 PM and
cost $0.19" was heard as "PR merged Adad and Costin". So the front writes them as words
first (`src/speakable.rs`): money, percentages, clock times, durations, dates, phone
numbers, versions, units (`32GB`, `48 tok/s`, `645 ms`), ordinals, ranges, web
addresses, file paths (the last part is said), commit hashes (left out), abbreviations
(`e.g.`, `w/o`, `Dr.`) and symbols. Phone numbers are said in groups with "zero":
"eight zero zero, five five five, zero one nine nine".

`normalize = false` in the settings turns it off; one request can with
`"normalize": false`.

`speak check` measures the result: each voice says thirteen sentences (three of prose,
ten full of numbers), the speech-to-text servers named by `[client] asr` (any
OpenAI-style `/v1/audio/transcriptions`) write down what they heard, and that is compared
word by word with what was said. With several listeners a sentence counts as its best
listener heard it. In our runs every voice reached 99% to 100% of words understood.

## How it fits the system

Lumbridge Speak works alone with any Magpie TTS server. Other programs use it through
its HTTP API.

| Works with | What passes between them |
|---|---|
| [lumbridge-voice](https://github.com/lumbridge-corp/lumbridge-voice) | Speech to text, the other direction; `speak check` can use it as a listener |
| Any OpenAI TTS client | `POST /v1/audio/speech` with `input`, `voice`, `response_format` |

## What it sends and where

The front listens on loopback (`127.0.0.1:8813`) with no authentication, and sends the
text it is given to the model server in `[magpie] url` (loopback by default). The
`speak` command talks to the front in `[client] front`; `speak check` also sends the
audio to the speech-to-text servers in `[client] asr`; `speak --on NAME` sends text to the
machine you named, with its bearer token. Nothing else leaves the machine. It stores
nothing: no audio, no text, no logs beyond what the service manager keeps. If you make
the front listen on a network, anyone who can reach it can use it.

## Docs

- [HTTP API](docs/API.md) — routes, fields, formats and errors
- [Run your own model server](docs/MODEL-SERVER.md) — Magpie on NeMo-Speech.cpp
- [Example settings](lumbridge-speak.example.toml)
- [Change log](CHANGELOG.md)

## Contributing

Bug reports and small fixes are welcome. Talk to us in an issue before a large change.
See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and send a pull request.

```sh
scripts/ci.sh   # scrub check, clippy, tests (a stand-in model; no server, no sound)
```

## Security

Do not open a public issue for a security problem.
Email security@lumbridgecorp.com or use "Report a vulnerability" on this repo's Security tab.
See [SECURITY.md](SECURITY.md).

## Licence

Apache-2.0. See [LICENSE](LICENSE).

Model weights are not part of this repo and have their own licences: see the
[MagpieTTS](https://huggingface.co/nvidia/magpie_tts_multilingual_357m) and
[NeMo NanoCodec](https://huggingface.co/nvidia/nemo-nano-codec-22khz-1.89kbps-21.5fps)
model cards. No audio is in this repo.
