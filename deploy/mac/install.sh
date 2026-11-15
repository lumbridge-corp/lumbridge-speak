#!/bin/bash
# Installs Lumbridge Speak on this Mac: the front as a background service on loopback,
# and the `speak` command. Run it on the Mac, as yourself. Running it again is safe: it
# rebuilds, swaps the program and restarts the service.
#
#   1. Builds the release program (cargo).
#   2. Stops if `speak` or `lumbridge-speak` in ~/.local/bin belongs to something else.
#   3. Writes ~/.config/lumbridge-speak/config.toml from the example if there is none.
#      An existing file is never changed.
#   4. Installs ~/.local/bin/lumbridge-speak and the `speak` link to it.
#   5. Installs and (re)starts the LaunchAgent com.lumbridgecorp.lumbridge-speak.
#   6. Waits for the front to answer and prints what it says.
#
# Remove:  deploy/mac/install.sh --remove   (the settings file stays)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BIN="$HOME/.local/bin"
CONF="$HOME/.config/lumbridge-speak/config.toml"
LABEL="com.lumbridgecorp.lumbridge-speak"
AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

ours() { # a path is free, or is ours
  [ ! -e "$1" ] && [ ! -L "$1" ] && return 0
  [ -L "$1" ] && [ "$(readlink "$1")" = "lumbridge-speak" ] && return 0
  [ -f "$1" ] && "$1" --help 2>/dev/null | grep -q '^lumbridge-speak: text to speech' && return 0
  return 1
}
ours "$BIN/lumbridge-speak" || die "$BIN/lumbridge-speak exists and is not Lumbridge Speak"
ours "$BIN/speak" || die "$BIN/speak exists and is not Lumbridge Speak"

if [ "${1:-}" = "--remove" ]; then
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  rm -f "$AGENT" "$BIN/speak" "$BIN/lumbridge-speak"
  say "Removed. Settings kept: $CONF"
  exit 0
fi

command -v cargo >/dev/null || die "cargo is not on the PATH"
(cd "$REPO" && cargo build --release)
BUILT="$REPO/target/release/lumbridge-speak"
[ -x "$BUILT" ] || die "the build did not produce $BUILT"

mkdir -p "$BIN" "$(dirname "$CONF")" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
if [ ! -f "$CONF" ]; then
  cp "$REPO/lumbridge-speak.example.toml" "$CONF"
  say "Wrote $CONF from the example. Set [magpie] url to your model server."
fi
"$BUILT" --config "$CONF" --print-config >/dev/null || die "$CONF has a mistake (see above); nothing was installed"

# A new file moved into place, never written over the running one.
cp "$BUILT" "$BIN/.lumbridge-speak.new"
mv -f "$BIN/.lumbridge-speak.new" "$BIN/lumbridge-speak"
ln -sfn lumbridge-speak "$BIN/speak"

sed "s|__HOME__|$HOME|g" "$HERE/$LABEL.plist" > "$AGENT"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
for _ in 1 2 3 4 5; do launchctl bootstrap "gui/$(id -u)" "$AGENT" 2>/dev/null && break; sleep 1; done

LISTEN="$("$BIN/lumbridge-speak" --config "$CONF" --print-config | sed -n 's/^ *listen: "\(.*\)",$/\1/p' | head -1)"
for _ in $(seq 1 20); do
  if HEALTH="$(curl -fsS -m 3 "http://$LISTEN/health" 2>/dev/null)"; then
    say "Lumbridge Speak is running on $LISTEN: $HEALTH"
    say "Try:  speak Hello from Lumbridge Speak.     speak voices     speak demo"
    exit 0
  fi
  sleep 0.5
done
say "Installed, but the front on $LISTEN does not report healthy. It answers once the model server"
say "can be reached. Log: $HOME/Library/Logs/lumbridge-speak.log"
curl -sS -m 3 "http://$LISTEN/health" || true
exit 1
