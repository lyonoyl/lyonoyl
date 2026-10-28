#!/usr/bin/env bash
#
# Stands in for `cargo` while `tauri dev` is running, so that what it launches is signed.
#
# `tauri dev` builds and runs in one command — it calls `cargo run` — and what that produces is a
# bare binary the linker signs ad-hoc. Its rule is then the hash of the binary, so every rebuild is
# a new application to the keychain and the system asks about the password again. Bundling is where
# Tauri does its signing, and `tauri dev` never bundles.
#
# So this takes the `run` apart: build, sign, then launch. Tauri is handed a process either way and
# does not know the difference.
#
# **The identifier is the part that is easy to get wrong.** A bare binary's identity is its file
# name unless it is told otherwise, so a signed `lyonoyl` would be a different application from
# `Lyonoyl Mail.app` and would need its own permission. Named with the bundle's identifier, the two
# carry the same rule byte for byte and one answer covers both, for ever.
#
# Not macOS, or no certificate set up: this is `cargo`, unchanged. A checkout nobody has configured
# still runs.

set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$(uname -s)" != "Darwin" ]; then
  exec cargo "$@"
fi

IDENTITY_FILE="${LYONOYL_SIGNING_ENV:-$HOME/.config/lyonoyl/signing.env}"
if [ ! -f "$IDENTITY_FILE" ] || ! bash -n "$IDENTITY_FILE" 2>/dev/null; then
  printf '\033[33m%s\033[0m\n' "No signing identity, so this build is signed with nothing --" >&2
  printf '\033[33m%s\033[0m\n' "the keychain will ask again after every rebuild. docs/development/guide.md says how." >&2
  exec cargo "$@"
fi

set -a
# shellcheck disable=SC1090
. "$IDENTITY_FILE"
set +a

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  exec cargo "$@"
fi

# What arrives is `run <flags…> -- <arguments for the application>`. The flags belong to the build,
# and everything past the first `--` belongs to what is being launched.
flags=()
app=()
seen=0
for one in "$@"; do
  case "$seen$one" in
    0run) ;;
    0--) seen=1 ;;
    0*) flags+=("$one") ;;
    *) app+=("$one") ;;
  esac
done

# Expanded the long way round because macOS ships bash 3.2, where an empty array under `set -u` is
# an error rather than nothing at all -- and the application is usually launched with no arguments,
# so the empty case is the ordinary one.
cargo build ${flags[@]+"${flags[@]}"}

# Asked of cargo rather than assumed, because a target triple or a release profile moves it.
target_dir=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
# Guarded the same way as the expansion above, and for the same reason: an empty array under
# `set -u` on bash 3.2 is an error. Written as an `if` rather than as `[ … ] && …` so that the
# loop's exit status cannot become the script's.
profile=debug
for one in ${flags[@]+"${flags[@]}"}; do
  if [ "$one" = "--release" ]; then profile=release; fi
done
built="$target_dir/$profile/lyonoyl"

if [ ! -x "$built" ]; then
  printf '\033[31m%s\033[0m\n' "cargo built something, but not at $built" >&2
  exit 1
fi

# The identifier comes from the one place that already holds it, so the two cannot drift apart.
identifier=$(python3 -c 'import json;print(json.load(open("src-tauri/tauri.conf.json"))["identifier"])')

# Kept rather than discarded: `codesign` says "replacing existing signature" on every rebuild, which
# is noise, and everything else it says is the reason signing failed. Throwing the whole stream away
# to be rid of the first line is how a failure here becomes a session that stops without saying why.
if ! said=$(codesign --force --sign "$APPLE_SIGNING_IDENTITY" \
              --identifier "$identifier" "$built" 2>&1); then
  printf '\033[31m%s\033[0m\n' "Signing failed, so this build would be a stranger to the keychain:" >&2
  printf '%s\n' "$said" >&2
  exit 1
fi

# Asked of the result, because the point is not that `codesign` ran. A build still named by its own
# hash is the thing this whole arrangement exists to prevent, and it costs nothing to check.
scripts/holds-still.sh "$built" > /dev/null

exec "$built" ${app[@]+"${app[@]}"}
