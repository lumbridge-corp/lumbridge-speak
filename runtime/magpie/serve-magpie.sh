#!/usr/bin/env bash
# Start NVIDIA Magpie TTS 357M on NeMo-Speech.cpp's server, the model lumbridge-speak's `magpie` adapter talks to.
# See docs/MODEL-SERVER.md for how to build the server and get the model files.
#
#   NEMO_SPEECH   the nemo-speech binary         (default: nemo-speech on PATH)
#   MAGPIE_DIR    folder with the Magpie files   (default: ~/.cache/nemo-speech/models/nvidia)
#   HOST, PORT    where to listen                (default: 127.0.0.1:8808)
#
# Extra arguments go to `nemo-speech serve`.
set -euo pipefail
bin=${NEMO_SPEECH:-nemo-speech}
M=${MAGPIE_DIR:-$HOME/.cache/nemo-speech/models/nvidia}
# `nemo-speech pull magpie` puts each model in <name>/<revision>/; the last revision found is used.
last() { local d; d=$(ls -d "$1"/*/ 2>/dev/null | tail -n 1); [ -n "$d" ] || { echo "serve-magpie: nothing in $1 (run: nemo-speech pull magpie)" >&2; exit 1; }; echo "${d%/}"; }
tts=$(last "$M/magpie_tts_multilingual_357m")
codec=$(last "$M/nemo-nano-codec-22khz-1.89kbps-21.5fps")
exec "$bin" serve \
  --host "${HOST:-127.0.0.1}" --port "${PORT:-8808}" \
  --tts-model "$tts/magpie_tts_multilingual_357m.v2607.f16.gguf" \
  --codec-model "$codec/nemo_nano_codec_22khz_1.89kbps_21.5fps.decoder.f16.gguf" \
  --tokenizer-dir "$tts/tokenizer" "$@"
