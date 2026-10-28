#!/usr/bin/env bash
#
# A development build whose identity survives being rebuilt.
#
# The problem this solves has nothing to do with distributing anything. macOS keeps a secret behind a
# rule naming the application allowed to read it, and for an unsigned build that rule is the hash of
# the binary. Rebuild, and the hash is different, and the application is a stranger to its own
# keychain — which is why a development build asks about the keychain over and over, and why it can
# end up unable to read the key its own databases are encrypted under.
#
# Signing with a real certificate replaces the hash with a rule naming the bundle and the
# certificate. That rule is the same on every build, for ever. `scripts/holds-still.sh` checks it.
#
# Notarization is not done here and is not needed: it is what lets *other* machines open the
# application, and this one is for this machine. No App Store Connect key, no network, no waiting on
# Apple.

set -euo pipefail

cd "$(dirname "$0")/.."

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

say "This machine"
node scripts/check-machine.mjs --build

say "The signing identity"
# shellcheck disable=SC1091
. scripts/signing-identity.sh signing
echo "certificate in the keychain"

say "Building"
# `--debug` because this is for working on, not for shipping: it keeps the assertions and the symbols
# and does not spend the optimizer's time. Signed all the same — the signature is the whole point.
#
# Only the application bundle. A disk image is a thing to send somebody, and nobody is sending this.
npx tauri build --debug --bundles app

app="target/debug/bundle/macos/Lyonoyl Mail.app"

say "Whether this build will still be itself tomorrow"
scripts/holds-still.sh "$app"

say "Done"
echo "$app"
echo
echo "The first run of a signed build asks once for each secret an unsigned build wrote, since"
echo "their rule names that build: answer each question with Always Allow. Never remove this"
echo "application's items from Keychain Access: one of them is the only key its databases open with."
