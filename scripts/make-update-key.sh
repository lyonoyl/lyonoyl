#!/usr/bin/env bash
#
# Makes the key every update is sealed with. Once.
#
# The public half goes into every build through `LYONOYL_UPDATE_PUBKEY`, and a copy takes an update
# sealed by that key and by no other, for as long as that copy exists. **A second key is a release
# every copy already installed refuses**, and losing the first is the same thing: those copies can
# then only be replaced by hand, by somebody who knows they have to. So this runs once, and refuses
# to run again over a key that is there.
#
# Where it puts things, all of it outside this tree:
#
#     ~/.config/lyonoyl/update.key        the secret half, locked with a password, chmod 600
#     ~/.config/lyonoyl/update.key.pub    the public half
#     ~/.config/lyonoyl/signing.env       two lines added: LYONOYL_UPDATE_PUBKEY, the line a build
#                                         compiles in, and LYONOYL_UPDATE_KEY_PATH
#
# The password is asked for at the terminal by the tool that makes the key, so it never passes
# through this script or onto a command line. Nothing here prints a key (AGENTS.md invariant 21).

set -euo pipefail

cd "$(dirname "$0")/.."

no() { printf '\033[31m%s\033[0m\n' "$1" >&2; }
say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

folder="$HOME/.config/lyonoyl"
key="$folder/update.key"
identity="${LYONOYL_SIGNING_ENV:-$folder/signing.env}"

if [ -e "$key" ] || [ -e "$key.pub" ]; then
  no "There is an update key already: $key"
  no "Every copy built with it takes updates sealed by it and by nothing else. A new one would leave"
  no "all of them unable to take an update again, so this stops rather than replace it."
  exit 1
fi

# Read as text rather than sourced: all this asks is whether a key is named there, and sourcing a
# file to find out runs it.
if [ -f "$identity" ] && grep -qE '^[[:space:]]*(export[[:space:]]+)?LYONOYL_UPDATE_(PUBKEY|KEY_PATH)=' "$identity"; then
  no "The identity file already names an update key: $identity"
  no "A key named there and missing from $folder is a key that was lost or moved. Find it rather"
  no "than making another: a new one is a release every copy already installed refuses."
  exit 1
fi

mkdir -p "$folder"
chmod 700 "$folder"

say "The key"
echo "The password locks the secret half. Keep it with the key, somewhere that outlives this machine."

# `CI` set to anything is read by the tool as "ask nothing", which makes a key with no password.
env -u CI npx tauri signer generate -w "$key"
chmod 600 "$key"

# The tool takes an empty password as well. A key that opens without one is a key anybody who
# copies the file seals updates with, so it is tried: sealing a scratch file with no password must
# fail. The password is set, empty, in the tool's environment, which makes it try that and not ask.
scratch=$(mktemp)
if env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PATH TAURI_SIGNING_PRIVATE_KEY_PASSWORD= \
  npx tauri signer sign -f "$key" "$scratch" </dev/null >/dev/null 2>&1; then
  rm -f "$key" "$key.pub" "$scratch" "$scratch.sig"
  no "That key opens with no password, so nothing was kept. Run this again and give it one."
  exit 1
fi
rm -f "$scratch" "$scratch.sig"

# The tool writes minisign wrapped in one more layer of base64. Unwrapped, the second line is the
# key itself: two bytes of algorithm, eight of key identifier and thirty-two of key, which is
# fifty-six characters of base64 and starts `RW`. Anything else is not something to build in.
public=$(base64 --decode < "$key.pub" | sed -n 2p | tr -d '\r')
if [ "${#public}" -ne 56 ] || [ "${public:0:2}" != "RW" ]; then
  no "The public half is not a key this project's builds can read, so nothing was written down."
  no "The files are at $key and $key.pub; remove both before running this again."
  exit 1
fi

say "The identity file"
umask 077
[ -f "$identity" ] || : > "$identity"
{
  printf '\n# The update key, made by scripts/make-update-key.sh. The public half is compiled into\n'
  printf '# every build; the secret half seals what those builds will take.\n'
  printf "LYONOYL_UPDATE_PUBKEY='%s'\n" "$public"
  printf "LYONOYL_UPDATE_KEY_PATH='%s'\n" "$key"
} >> "$identity"
chmod 600 "$identity"
echo "$identity names the key now"

say "Two things, before anything else is built"
echo "  1. Copy $key and its password somewhere that survives this machine:"
echo "     a password manager, not a second folder on this disk. Lose either and every copy built"
echo "     from now on can never be updated again."
echo "  2. Nobody else gets it. An update still has to be signed by the Developer ID before a copy"
echo "     installs it, so the key alone puts nothing on anybody's Mac -- but it is half of what does."
echo
echo "The next npm run app:release builds the public half in, and says so: Updates: on."
