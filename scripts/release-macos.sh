#!/usr/bin/env bash
#
# A macOS build somebody else can open.
#
# An unsigned application is refused by Gatekeeper on every machine that did not build it, so a
# release, and the disk image it comes in, is signed with a Developer ID certificate and notarized
# by Apple. Neither the certificate nor the account is named anywhere in this repository — AGENTS.md
# invariant 21 — so this script reads them from a file the developer keeps outside the tree, and
# says plainly when there is none.
#
# **The other half of what signing buys is the keychain.** macOS keeps a password behind a list of
# the applications allowed to read it, and that list names an application by its signature. An
# ad-hoc signed build gets a new identity on every rebuild, so the list stops matching and the
# system either asks again or refuses — which is a client that cannot open its own databases. A
# Developer ID signature is the same on every build, so the list keeps matching.
#
# Where the identity lives:
#
#     ~/.config/lyonoyl/signing.env      chmod 600, never in this tree
#
#         APPLE_SIGNING_IDENTITY   the full name of the Developer ID Application certificate
#         APPLE_API_ISSUER         the issuer of the App Store Connect key
#         APPLE_API_KEY            that key's identifier
#         APPLE_API_KEY_PATH       where the `.p8` file is, also outside this tree
#
# Nothing here ever prints a value. What it prints is whether a thing is there.

set -euo pipefail

cd "$(dirname "$0")/.."

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

say "The signing identity"
# shellcheck disable=SC1091
. scripts/signing-identity.sh notarizing
echo "certificate in the keychain, notarization key present and readable only by you"

say "Finder, which arranges the disk image's window"
# Tauri lays out the window the disk image opens in by asking Finder, through AppleScript, while it
# builds the image. macOS lets the terminal running this do that only once it has been allowed, and
# a terminal that may not fails that step at the end of the build, where Tauri says only that its
# script failed. Asked here first, macOS puts its question while somebody is watching, and a
# refusal stops the release before anything is built.
if ! osascript -e 'tell application "Finder" to get name' >/dev/null 2>&1; then
  printf '\033[31m%s\033[0m\n' "This terminal may not control Finder, which arranges the disk image's window. Allow it under System Settings, Privacy & Security, Automation, then run this again." >&2
  exit 1
fi
echo "Finder answers this terminal"

say "Everything that must be true before a release leaves this machine"
npm run preflight

say "Building, signing and notarizing"

# Where this was built is taken out of what ships. A Rust binary carries the absolute path of every
# source file compiled into it -- measured on a release build of this application: seven hundred and
# ten occurrences of one home directory, five hundred and twenty-eight distinct paths, most of them
# under the dependency cache. That is a home directory, and invariant 21 does not care that it got
# there by accident.
#
# Read from the environment and never written down here, which is the same rule the signing identity
# follows. `--remap-path-prefix` is a compiler flag rather than a manifest setting because the
# manifest option for it is not something this toolchain accepts.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup --remap-path-prefix=$PWD=/lyonoyl"
# Tauri signs with APPLE_SIGNING_IDENTITY and notarizes with the three APPLE_API_* values, all of
# which are in the environment by now. `tauri.conf.json` names none of them, deliberately: a
# checkout carries no identity, and one is added by the machine that builds rather than by the
# repository.
# The name of the certificate is taken out of the output as it goes past. Tauri announces what it
# is signing with -- "Signing with identity \"Developer ID Application: <organisation> (<team>)\"",
# five times in an ordinary build -- and a release run on a hosted machine writes that into a log
# anybody can read. Invariant 21 is about the repository, and a build this repository defines writing
# an identity into its own public output is the same thing by another road.
#
# Replaced rather than dropped, so the line still says signing happened. `pipefail` is set, so a
# build that fails still fails here.
npx tauri build 2>&1 | sed -E \
  's/(Developer ID Application:)[^"]*/\1 <this team>/g; s/(TeamIdentifier=)[A-Z0-9]*/\1<this team>/g'

say "What Gatekeeper makes of it"

# Found, not assumed. A universal build writes under `target/universal-apple-darwin/` and an
# ordinary one under `target/release/`, so a hard-coded path points at whichever an earlier build
# happened to leave behind -- a check that passes on a stale artifact while the new one goes out
# unexamined. The newest is the one this run just made, because this run is what built it.
image=$(find target -type f -name '*.dmg' -print0 2>/dev/null \
  | xargs -0 ls -t 2>/dev/null | head -1)

if [ -z "$image" ]; then
  printf '\033[31m%s\033[0m\n' "The build made no disk image, so there is nothing to release." >&2
  exit 1
fi

echo "$image"

# Tauri signs the disk image and does not notarize it, so the image is notarized below, once the
# application inside has answered everything. Until then it is opened rather than taken at its word:
# a signed image can carry an unsigned application.
codesign --verify --strict --verbose=2 "$image"

mounted=$(mktemp -d)
hdiutil attach -nobrowse -quiet -mountpoint "$mounted" "$image"
trap 'hdiutil detach -quiet "$mounted" 2>/dev/null; rmdir "$mounted" 2>/dev/null' EXIT

app="$mounted/Lyonoyl Mail.app"
if [ ! -d "$app" ]; then
  printf '\033[31m%s\033[0m\n' "The disk image does not hold the application." >&2
  exit 1
fi

# Everything below is asked of the copy a person would drag out of the image, which is the only
# artifact any of this is about.
scripts/holds-still.sh "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# Asked of the binary rather than assumed of the flag. Some paths reach a build from a dependency
# that compiles C of its own, where a Rust compiler flag does not go -- so the only thing that says
# whether a home directory shipped is looking in what shipped.
inside="$app/Contents/MacOS"
# `-a` because this is a binary, and without it `grep` reports nothing at all on one rather than
# reporting no match -- a check that says what a person wants to hear whatever is in the file. A
# `grep` that finds nothing exits 1, and under `pipefail` that ended the script there without a word,
# on exactly the binary that passes: so its finding nothing is taken as the answer it is.
carried=$({ grep -raoiF "$HOME" "$inside" 2>/dev/null || true; } | wc -l | tr -d ' ')
if [ "$carried" != "0" ]; then
  printf '\033[31m%s\033[0m\n' "The application carries the directory it was built in, $carried times." >&2
  exit 1
fi

# Read into a variable and then matched, never piped into `grep -q`. Under `pipefail` a `grep -q`
# that finds its line exits at once, the command upstream dies of a broken pipe, and the pipeline
# reports failure -- so the check fails precisely when it succeeds. It cost an afternoon to see.
#
# Matched on the name codesign prints rather than on the bit it stands for: the flags are a set, and
# `0x10800(runtime,hard)` is as much the hardened runtime as `0x10000(runtime)` is. A pattern written
# against the number refuses the first of those.
directory=$(codesign -dv --verbose=4 "$app" 2>&1 | grep -E '^CodeDirectory' || true)
case "$directory" in
  *"(runtime)"* | *"(runtime,"* | *",runtime)"* | *",runtime,"*) ;;
  *)
    printf '\033[31m%s\033[0m\n' "The hardened runtime is not on, and Apple will not notarize it." >&2
    exit 1
    ;;
esac

# Linked to nothing but the system, read and matched the same way. On macOS SQLCipher takes its
# ciphers from the system; a build made under OPENSSL_DIR, OPENSSL_LIB_DIR with OPENSSL_INCLUDE_DIR,
# plain or prefixed with the target, or LIBSQLITE3_SYS_USE_PKG_CONFIG links a library from wherever
# that pointed instead, outside the system, which the Mac that built it has and another may not;
# there the application stops before its first window.
#
# Every binary links the system's own C library, so an answer without it is otool having read
# nothing, and a check that reads nothing does not pass. An outside library is printed with the
# account it lies under replaced, in any letter case, since what this prints may be shown to
# somebody.
linked=$(otool -L "$inside"/* 2>/dev/null | grep -E '^[[:space:]]' | awk '{print $1}' || true)
case "$linked" in
  *"/usr/lib/libSystem.B.dylib"*) ;;
  *)
    printf '\033[31m%s\033[0m\n' "otool read no libraries from the application, so nothing says what it links." >&2
    exit 1
    ;;
esac
outside=$(printf '%s\n' "$linked" | grep -vE '^(/System/Library/|/usr/lib/|$)' || true)
if [ -n "$outside" ]; then
  printf '\033[31m%s\033[0m\n' "The application links libraries from outside the system:" >&2
  printf '%s\n' "$outside" | sed -E 's#^/([Uu][Ss][Ee][Rr][Ss]|[Hh][Oo][Mm][Ee])/[^/]+/#/\1/<account>/#' >&2
  printf '%s\n' "Unset OPENSSL_DIR, OPENSSL_LIB_DIR and OPENSSL_INCLUDE_DIR, plain or prefixed with the target, and LIBSQLITE3_SYS_USE_PKG_CONFIG, then run cargo clean -p libsqlite3-sys --release." >&2
  exit 1
fi

# The one that catches a missing secret. Tauri skips notarization with a warning and exits zero when
# the notary credentials are absent, so a build with no secret is green and produces an application
# every other Mac refuses. A stapled ticket is the proof that the notary actually saw it.
xcrun stapler validate "$app"

spctl --assess --type execute --verbose=4 "$app"

say "Notarizing the disk image"

# The image goes to the notary as well. A Mac that downloaded the image asks Gatekeeper about the
# image's own signature before it opens it, and refuses a signed image the notary never saw,
# whatever the application inside carries: "Unnotarized Developer ID". The machine that built the
# image never asks, because nothing marked it as downloaded, so it opens here whatever it is.
#
# Stapling writes into the image, so it is let go of first.
hdiutil detach -quiet "$mounted"
rmdir "$mounted"
trap - EXIT

notary=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
# The status the notary gives the image is what decides, so it is read from the answer, and anything
# but Accepted stops the release with the notary's own account of why. A submission that fails on
# the way falls through to the same refusal rather than stopping before it, which would swallow the
# answer it was given.
answer=$(xcrun notarytool submit "$image" "${notary[@]}" --wait --output-format json) || true
status=$(plutil -extract status raw -o - - <<<"$answer" 2>/dev/null || true)
if [ "$status" != "Accepted" ]; then
  printf '\033[31m%s\033[0m\n' "The notary did not accept the disk image: ${status:-no answer}." >&2
  submission=$(plutil -extract id raw -o - - <<<"$answer" 2>/dev/null || true)
  [ -z "$submission" ] || xcrun notarytool log "$submission" "${notary[@]}" >&2 || true
  exit 1
fi
echo "accepted by the notary"
xcrun stapler staple "$image"

# Asked the way a Mac that downloaded it asks, by the image's own signature, rather than taken from
# the ticket having been written. Not quiet: a step that fails here says why rather than ending the
# script without a word. Verbose enough to say it was accepted and why, and no more: one
# more level names the certificate's holder.
codesign --verify --strict "$image"
xcrun stapler validate "$image"
spctl --assess --type open --context context:primary-signature --verbose "$image"

# What this build will and will not do about updates, said out loud rather than discovered later.
#
# The key is compiled in through `option_env!`, so a build made without it is not a build with a
# broken updater — it is a build whose updater never runs and never asks the network anything. That
# is a fine thing to ship and a bad thing to ship by accident, and the only difference between the
# two is whether anybody was told.
if [ -n "${LYONOYL_UPDATE_PUBKEY:-}" ]; then
  say "Updates: on"
  echo "  This build takes an update sealed by the key that was set when it was built, and asks its"
  echo "  person before it looks for one. The update for the copies already out there is made from"
  echo "  this image: scripts/release-update.sh --notes <notes.json>."
else
  say "Updates: off"
  echo "  LYONOYL_UPDATE_PUBKEY was not set, so this build never checks for an update and never"
  echo "  asks the network about one. Nothing in it can be updated in place; a new version is a"
  echo "  new download. scripts/make-update-key.sh makes the key and sets it, once."
fi

say "Done"
echo "$image"
