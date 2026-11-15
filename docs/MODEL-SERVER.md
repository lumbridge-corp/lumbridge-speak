# Run your own model server

`lumbridge-speak` holds no model. Its `magpie` adapter sends each sentence to a
[NVIDIA Magpie TTS](https://huggingface.co/nvidia/magpie_tts_multilingual_357m) server
and gets raw audio back. This page stands one up on the same machine.

You can try the front and the `speak` command without any of this:
`speak text "..."` prints the words that would be spoken, and
`LUMBRIDGE_SPEAK_ADAPTER=mock lumbridge-speak` serves placeholder audio with no model.

## 1. Install NeMo-Speech.cpp

Magpie runs in [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp), NVIDIA's
C++ speech runtime (Apache-2.0). Follow its README to install the `nemo-speech` command,
or build it from source for your GPU (its build guide has the CUDA server preset). A
GPU is not required, but synthesis is much faster with one.

## 2. Get the model

```sh
nemo-speech pull magpie
```

This downloads MagpieTTS Multilingual 357M, its tokenizer and the NeMo NanoCodec into
`~/.cache/nemo-speech/models/nvidia/`. The model weights are not part of this repo and
have their own licence terms: read the model cards for
[MagpieTTS Multilingual 357M](https://huggingface.co/nvidia/magpie_tts_multilingual_357m) and
[NeMo NanoCodec](https://huggingface.co/nvidia/nemo-nano-codec-22khz-1.89kbps-21.5fps)
before you use them.

## 3. Start the server

```sh
runtime/magpie/serve-magpie.sh
```

It listens on `127.0.0.1:8808`, the address `lumbridge-speak` asks by default.
`NEMO_SPEECH` names the binary if it is not on `PATH`, `MAGPIE_DIR` the model folder,
`HOST` and `PORT` the address. Check it:

```sh
curl -s http://127.0.0.1:8808/v1/models      # lists magpietts and its five voices
```

## 4. Point the front at it

Nothing to do if both run on one machine. Otherwise set the server's address in
`~/.config/lumbridge-speak/config.toml`:

```toml
[magpie]
url = "http://127.0.0.1:8808"
model = "magpietts"
rate = 22050
```

or `LUMBRIDGE_SPEAK_MAGPIE_URL`. Then `speak Hello.` should play.

The server has no authentication. Keep it on loopback, or on a network you trust.

## Voices

Magpie has five built-in voices: `John`, `Sofia`, `Aria`, `Jason`, `Leo`. It cannot clone
a voice. In the build we measured, the names do not match how the voices sound: Leo has
the highest pitch (212 Hz median), Aria and Sofia the lowest. Try them with `speak demo`.
