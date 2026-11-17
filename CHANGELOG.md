# Changelog

## Unreleased

## 0.1.1 - 2026-11-17

- README: a header image, in `docs/assets/`. No code changes.

## 0.1.0 - 2026-11-13

First open-source release.

- `lumbridge-speak`, an OpenAI-style `/v1/audio/speech` front for NVIDIA Magpie TTS on NeMo-Speech.cpp,
  with `wav`, `pcm` and (through `ffmpeg`) `mp3`, `opus`, `aac` and `flac`.
- Numbers as words before the model sees the text: money, percentages, times, durations (`3m 12s`), dates,
  phone numbers, versions, units, ordinals, ranges, web addresses, file paths and symbols.
- The `speak` command: speak here or on another machine, write a WAV, list and demo the voices, measure
  how well each voice is understood (`speak check`), print the words that would be spoken.
- A macOS installer with a LaunchAgent, and a systemd user unit for Linux.
