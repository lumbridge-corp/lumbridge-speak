# HTTP API

The HTTP API of `lumbridge-speak` (the front). It is an OpenAI-style `/v1/audio/speech`
in front of a Magpie TTS server, and it answers byte for byte like the earlier Python
front it replaced, so existing callers keep working. Everything marked **observed** was
checked against a running service on 2026-11-09.

The front holds no model. It sends each piece of text to Magpie
(`[magpie] url`, default `http://127.0.0.1:8808/v1/audio/speech`) and gets raw samples back.

No authentication: the front listens on loopback (`127.0.0.1:8813`) unless the settings
say otherwise. JSON responses are written the way Python's `json.dumps` writes them:
**a space after every `:` and `,`**. Health checks can look for the exact text
`"ok": true`, with that space.

## `GET /health`

The front asks the model server `GET /v1/models` (5-second limit).

Up, status 200 (observed):

```json
{"ok": true, "voices": 5, "default": "default", "upstream": "http://127.0.0.1:8808", "engine": "magpietts", "rate": 22050}
```

Model server not reachable: status 503, `{"ok": false, "error": "<reason>"}`.

## `GET /v1/models`

Fixed, and kept as the earlier front answered it (it names two earlier models, not
Magpie). Observed:

```json
{"object": "list", "data": [{"id": "voxcpm2", "object": "model"}, {"id": "qwen3-tts", "object": "model"}]}
```

## `GET /v1/audio/voices` and `GET /voices`

Observed:

```json
{"voices": [{"id": "John", "label": "John"}, {"id": "Sofia", "label": "Sofia"}, {"id": "Aria", "label": "Aria"}, {"id": "Jason", "label": "Jason"}, {"id": "Leo", "label": "Leo"}], "default": "default"}
```

`default` is the setting `reported_default_voice` and selects nothing. What an unknown
voice really gets is `default_voice` (Leo).

## `POST /v1/audio/speech`

JSON body.

| Field | Default | Meaning |
|---|---|---|
| `input` | required | The text. Up to 4096 characters after cleaning. |
| `voice` | default voice | One of the five names, any letter case. Anything else, or nothing: the default voice. |
| `response_format` | `mp3` | `mp3`, `wav`, `pcm`, `opus`, `aac`, `flac`, any letter case |
| `instructions` | none | A style cue. Must be text or absent. Checked, then **ignored** with Magpie. |
| `normalize` | `true` | `false`: the text goes to the model as written. Must be `true`, `false` or absent, else 400 `normalize must be true or false`. |
| `model`, `speed`, anything else | | Ignored |

### What happens to the text

1. **Cleaning.** Fenced code blocks are removed; `[label](url)` becomes `label`; the
   characters `* _ ` # > | ~` become spaces; runs of whitespace become one space; the
   ends are trimmed.
2. Empty after cleaning → 400. Longer than 4096 characters → 400.
3. **Style cue.** If the text starts with a cue in round brackets, `(softly) Hello`, the
   cue is cut off the text. The cue (from `instructions`, else from the brackets) must be
   at most 300 characters and contain no brackets, or → 400. With Magpie the cue is then
   dropped.
4. **Pieces.** The text is split after `.`, `!` or `?` followed by whitespace, and the
   sentences are packed into pieces of at most 180 characters. A piece that is still
   longer is cut at the last space before 180 (or at 180 if there is none).
5. Each piece goes to the model, one after another:
   `{"model": "magpietts", "input": "<piece>", "voice": "<voice>", "language": "en-US", "response_format": "pcm"}`
   (120-second limit each). The answers are joined with 0.15 s of silence between pieces.

Before step 4, numbers, money, times, versions, paths and the like are written as words
(`src/speakable.rs`, README "Numbers as words"), unless the settings say
`normalize = false` or the request says `"normalize": false`. Magpie reads the written
forms as noise. After joining, the audio is scaled to the settings' `level`, if one is set.

### Response

Status 200, the audio, with `Content-Type` and `Content-Length`. Mono, 16-bit.

| `response_format` | Content type | Audio |
|---|---|---|
| `wav` | `audio/wav` | 22050 Hz WAV with a 44-byte header (observed) |
| `pcm` | `audio/pcm` | Raw samples **resampled to 24000 Hz**, the OpenAI rule for raw audio (observed: 75,790 bytes for the clip that is 69,632 bytes at 22050 Hz) |
| `mp3` | `audio/mpeg` | from `ffmpeg` |
| `opus` | `audio/ogg` | Opus in Ogg, 48 kbit/s, from `ffmpeg` |
| `aac` | `audio/aac` | ADTS, from `ffmpeg` |
| `flac` | `audio/flac` | from `ffmpeg` |

### Errors

All errors have this form, with the space after each `:` and `,`:

```json
{"error": {"message": "<message>", "type": "invalid_request_error"}}
```

`type` is `invalid_request_error` below status 500 and `server_error` from 500 up.

| Status | Message | When | Observed |
|---|---|---|---|
| 400 | `body must be JSON` | Body is not JSON | yes |
| 400 | `input is empty` | Nothing left after cleaning | yes |
| 400 | `input longer than 4096 chars` | | no |
| 400 | `instructions must be a string` | | yes |
| 400 | `instructions must be at most 300 chars without parentheses` | | yes |
| 400 | `unsupported response_format <name>` | | yes |
| 502 | `magpietts <status>: <first 200 characters>` | The model server answered with an error | no |
| 502 | `magpietts unreachable: <reason>` | The model server could not be reached | no |
| 500 | `ffmpeg: <first 200 characters>` or another reason | Conversion failed | no |
| 404 | `not found` | Any other path, `GET` or `POST` | yes |

The checks run in the order of the first six rows; the first that fails answers.

A JSON body that is not an object (a list, a number) is 400 `body must be JSON`. A
request body over 2 MB is 413. A query string on a known path is ignored.

## Magpie's own contract

For reference, since the front speaks it. Observed on `:8808`:

- `GET /ready` → `{"capabilities":["tts"],"device":"auto","ready":true}`
- `GET /v1/models` →
  `{"data":[{"capability":"speech","device":"auto","id":"magpietts","languages":["en-US","es-ES","de-DE","fr-FR","it-IT","vi-VN","hi-IN","ar-AE","ar-SA","ar-MSA","ko-KR","pt-BR"],"object":"model","owned_by":"local","voices":["John","Sofia","Aria","Jason","Leo"]}],"object":"list"}`
- `POST /v1/audio/speech` with `{"input", "voice", "language", "response_format"}`:
  `wav` → `audio/wav`, 22050 Hz mono 16-bit; `pcm` → `audio/pcm`, the same without a
  header. `model` is optional.
- Errors, status 400:
  `{"error":{"code":null,"message":"input is required","param":null,"type":"invalid_request_error"}}`,
  and `"unknown voice_name 'Nobody'"` for a voice it does not have.
- From earlier measurements (2026-11-06, not repeated here): it has no text normalizer,
  and speed must stay 1.0.
