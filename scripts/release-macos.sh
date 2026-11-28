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
#
# Which Macs it is for is asked, and Apple silicon is the answer when nothing is. A release command
# names the system, then the processor as Apple, Microsoft and Node name it:
#
#     npm run app:release:mac            arm64, Apple silicon, the same as :mac:arm64
#     npm run app:release:mac:arm64      arm64, Apple silicon, which runs this with --arm64
#     npm run app:release:mac:x64        x64, Intel, with --x64
#     npm run app:release:mac:both       both, with --both: a full release for each, one after the other
#
# Each stops at the notarized image, a build to try or to keep. `:notes` after the processor, as in
# `npm run app:release:mac:both:notes`, which adds --notes, makes it the release itself. Before
# anything is built it asks for a tree that is its commit, the version's notes and the update key's
# password; then it lays the release out through `scripts/release-update.sh`. The notes are
# `release-notes/<version>.json` at the root of the tree, a folder git leaves out, beside
# `<version>.md`, the text of the GitHub release. Everything of the version is then in
# `publish/<version>/`, at the root of the tree and out of git as well, and publishing is done by
# hand from there.

set -euo pipefail

# The update key's password, taken out of the environment before anything else runs, as
# scripts/release-update.sh takes it: left there, every program below would inherit it.
password="${LYONOYL_UPDATE_KEY_PASSWORD:-}"
unset LYONOYL_UPDATE_KEY_PASSWORD

no() { printf '\033[31m%s\033[0m\n' "$1" >&2; }
say() { printf '\n\033[1m%s\033[0m\n' "$1"; }

kind=""
with_notes=""
for word in "$@"; do
  case "$word" in
    --arm64 | --x64 | --both)
      # One choice a run: two would be a release of whichever was named last.
      if [ -n "$kind" ] && [ "$kind" != "${word#--}" ]; then
        no "One of --arm64, --x64 or --both, not two."
        exit 1
      fi
      kind="${word#--}"
      ;;
    --notes) with_notes=yes ;;
    *)
      no "Not something this takes: $word. It takes --arm64, --x64 or --both, and --notes."
      exit 1
      ;;
  esac
done
kind="${kind:-arm64}"

cd "$(dirname "$0")/.."

version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)

# The kind of Mac this one is, as Rust names it. A terminal running under Rosetta answers x86_64 on
# Apple silicon, and building for Apple silicon there is building for another kind, which is what it is.
case "$(uname -m)" in
  arm64) here=aarch64-apple-darwin ;;
  x86_64) here=x86_64-apple-darwin ;;
  *) here="" ;;
esac

# A kind other than this Mac's own needs Rust's library for that kind: asked for here, before
# anything is built, rather than found missing after the check and half a build.
case "$kind" in
  arm64) asked_for="aarch64-apple-darwin" ;;
  x64) asked_for="x86_64-apple-darwin" ;;
  both) asked_for="aarch64-apple-darwin x86_64-apple-darwin" ;;
esac
missing=""
for target in $asked_for; do
  if [ "$target" = "$here" ]; then continue; fi
  installed=" $(rustup target list --installed 2>/dev/null | tr '\n' ' ')"
  case "$installed" in
    *" $target "*) ;;
    *) missing="$missing $target" ;;
  esac
done
if [ -n "$missing" ]; then
  no "A release for $kind needs Rust's library for${missing}. Add it once with: rustup target add${missing}"
  exit 1
fi

if [ -z "$version" ]; then
  no "Cargo.toml names no version under [workspace.package], so there is nothing to call this build."
  exit 1
fi

# What the release itself needs is asked before anything is built, never found missing after the
# build: a tree that is its commit, since the source published beside it is that commit; the notes
# of the version; and the update key's password, asked once and tried on a throwaway file.
if [ -n "$with_notes" ]; then
  if [ -n "$(git status --porcelain 2>/dev/null || echo unknown)" ]; then
    no "The tree holds changes that are not committed. Commit them first: the source published with the release is the commit."
    exit 1
  fi
  for file in "$version.json" "$version.md"; do
    if [ ! -f "release-notes/$file" ]; then
      no "There are no notes for $version: release-notes/$file, at the root of the tree."
      exit 1
    fi
  done
  # shellcheck disable=SC1091
  (. scripts/signing-identity.sh sealing) >/dev/null
  if [ -z "$password" ]; then
    # `IFS=` so a passphrase that starts or ends with a space is read as it was typed.
    if ! IFS= read -rsp "The update key's password: " password </dev/tty; then
      no "There is no terminal to ask for the update key's password at."
      exit 1
    fi
    echo >&2
  fi
  probe=$(mktemp)
  if ! (
    # shellcheck disable=SC1091
    . scripts/signing-identity.sh sealing >/dev/null
    env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PATH \
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$password" \
      npx tauri signer sign -f "$LYONOYL_UPDATE_KEY_PATH" "$probe" >/dev/null 2>&1
  ); then
    rm -f "$probe" "$probe.sig"
    no "The update key does not open with that password, so nothing was built."
    exit 1
  fi
  rm -f "$probe" "$probe.sig"
fi

# Both is the release for each kind, one after the other: each its own build, its own checks and its
# own notarized image, and with the notes each lays its own image out, the second adding its kind
# to what the first wrote. Nothing of one is mixed into the other, and the password is asked once.
if [ "$kind" = both ]; then
  LYONOYL_UPDATE_KEY_PASSWORD="$password" LYONOYL_RELEASE_PART="1 of 2" \
    scripts/release-macos.sh --arm64 ${with_notes:+--notes}
  LYONOYL_UPDATE_KEY_PASSWORD="$password" LYONOYL_RELEASE_PART="2 of 2" \
    scripts/release-macos.sh --x64 ${with_notes:+--notes}
  exit 0
fi

# What is made, said first: the version is the tree's own, from Cargo.toml, and the processor the
# one asked for.
say "Lyonoyl Mail $version, macOS $kind${LYONOYL_RELEASE_PART:+, $LYONOYL_RELEASE_PART}"

case "$kind" in
  arm64) triple=aarch64-apple-darwin wanted="arm64" ;;
  x64) triple=x86_64-apple-darwin wanted="x86_64" ;;
esac

# This Mac's own kind builds where every other build here does, `target/release`; another kind is
# built under its own name, where Tauri writes it.
if [ "$triple" = "$here" ]; then
  target_flags=()
  bundles="target/release/bundle"
else
  target_flags=(--target "$triple")
  bundles="target/$triple/release/bundle"
fi

# shellcheck source=scripts/progress.sh
. scripts/progress.sh
# shellcheck source=scripts/notarize.sh
. scripts/notarize.sh
trap 'progress_end $?' EXIT

STAGE_IDENTITY="The signing identity"
STAGE_FINDER="Finder, which arranges the disk image's window"
STAGE_CHECK="Everything that must be true before a release leaves this machine"
STAGE_BUILD="Building, signing and notarizing"
STAGE_GATEKEEPER="What Gatekeeper makes of it"
STAGE_IMAGE="Notarizing the disk image"
STAGE_LAYOUT="The release laid out for publishing"
stages="$STAGE_IDENTITY
$STAGE_FINDER
$STAGE_CHECK
$STAGE_BUILD
$STAGE_GATEKEEPER
$STAGE_IMAGE"
if [ -n "$with_notes" ]; then
  stages="$stages
$STAGE_LAYOUT"
fi
# Each kind keeps its own times.
run=release
if [ "$kind" != arm64 ]; then run="release-$kind"; fi
progress_begin "$run" Stage "$stages"

# The check's estimate is the preflight's own last time on the path it is about to take.
if node scripts/tree-fingerprint.mjs --passed target/preflight/passed >/dev/null 2>&1; then
  progress_total_of preflight-passed-before
else
  progress_total_of preflight
fi
if [ -n "$progress_total" ]; then progress_expect "$STAGE_CHECK" "$progress_total"; fi

progress_step "$STAGE_IDENTITY"
# Checked in a subshell and loaded only for the build: in the check's environment, Tauri would sign
# and notarize the check's own build, and the update key would recompile the application's crate.
# shellcheck disable=SC1091
(. scripts/signing-identity.sh notarizing)
echo "certificate in the keychain, notarization key present and readable only by you"

progress_step "$STAGE_FINDER"
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

# A second volume named after the application leaves Tauri unable to let go of its own image at the
# last step: "Resource busy". One this tree's build left mounted is let go of; any other is refused.
while IFS= read -r device; do
  if ! hdiutil detach -quiet "$device" 2>/dev/null; then
    printf '\033[31m%s\033[0m\n' "A disk image an earlier build left mounted cannot be let go of. Eject it in Finder, then run this again." >&2
    exit 1
  fi
  echo "let go of a disk image an earlier build left mounted"
done < <(hdiutil info 2>/dev/null | awk -v here="$PWD/target/" '
  /^image-path/ {
    path = substr($0, index($0, ": ") + 2)
    ours = index(path, here) == 1 && path ~ /\/bundle\/macos\/rw\.[0-9]+\.[^\/]*\.dmg$/
    next
  }
  ours && /^\/dev\/disk[0-9]+[ \t]/ { print $1; ours = 0 }
')
mounts=$(mount)
case "$mounts" in
  *" on /Volumes/Lyonoyl Mail ("* | *" on /Volumes/Lyonoyl Mail "[0-9]*)
    printf '\033[31m%s\033[0m\n' "A disk image of Lyonoyl Mail is open, and the one this builds takes the same name. Eject it in Finder, then run this again." >&2
    exit 1
    ;;
esac
echo "no disk image of Lyonoyl Mail is open"

progress_step "$STAGE_CHECK"
# Checked whole, unless the preflight already passed this tree as it stands: then it asks again only
# what changes while a tree stands still.
npm run preflight -- --if-changed

progress_step "$STAGE_BUILD"
# shellcheck disable=SC1091
. scripts/signing-identity.sh notarizing

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
#
# `-l` passes each line on as it comes, so the line at the bottom knows when the build went quiet.
#
# The window Finder opens to lay the image out looks finished, and dragging the application out of it
# keeps the volume busy, which fails the build at its last step.
echo "Near the end Finder opens the disk image's window by itself, to lay it out. Leave it alone until this says Done."
progress_ticker_start
npx tauri build ${target_flags[@]+"${target_flags[@]}"} 2>&1 | sed -l -E \
  's/(Developer ID Application:)[^"]*/\1 <this team>/g; s/(TeamIdentifier=)[A-Z0-9]*/\1<this team>/g' \
  | progress_relay
progress_ticker_stop

progress_step "$STAGE_GATEKEEPER"

# Found where this kind of build writes, not assumed: this Mac's own kind under `target/release/` and
# another under its own name, so one path for every kind points at whichever an earlier build
# happened to leave behind -- a check that passes on a stale artifact while the new one goes out
# unexamined. The newest there is the one this run just made, because this run is what built it, and
# the code it holds is asked about below as well.
image=$(find "$bundles" -type f -name '*.dmg' -print0 2>/dev/null \
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
trap 'progress_end $?; hdiutil detach -quiet "$mounted" 2>/dev/null; rmdir "$mounted" 2>/dev/null' EXIT

app="$mounted/Lyonoyl Mail.app"
if [ ! -d "$app" ]; then
  printf '\033[31m%s\033[0m\n' "The disk image does not hold the application." >&2
  exit 1
fi

# Everything below is asked of the copy a person would drag out of the image, which is the only
# artifact any of this is about.

# The kinds of Mac it holds code for, held to the kind asked for: an image of another kind is
# refused here rather than published as this one. lipo reading nothing is an answer that matches
# no kind, so it does not pass.
executable="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist")"
held=$({ lipo -archs "$executable" 2>/dev/null || true; } | tr ' ' '\n' | { grep . || true; } \
  | LC_ALL=C sort | tr '\n' ' ')
asked=$(printf '%s' "$wanted" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ')
if [ "$held" != "$asked" ]; then
  said="${held% }"
  no "The application holds code for ${said:-nothing lipo could read}, and this release is for ${asked% }."
  exit 1
fi
echo "code for ${held% }"
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

progress_step "$STAGE_IMAGE"

# The image goes to the notary as well. A Mac that downloaded the image asks Gatekeeper about the
# image's own signature before it opens it, and refuses a signed image the notary never saw,
# whatever the application inside carries: "Unnotarized Developer ID". The machine that built the
# image never asks, because nothing marked it as downloaded, so it opens here whatever it is.
#
# Stapling writes into the image, so it is let go of first.
hdiutil detach -quiet "$mounted"
rmdir "$mounted"
trap 'progress_end $?' EXIT

notary=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
# The status the notary gives the image is what decides, so it is read from the notary's answer, and
# anything but Accepted stops the release with the notary's own account of why. An upload that fails,
# and a notary that stops answering, fall through to the same refusal rather than stopping before it,
# which would swallow the answer that was given.
progress_ticker_start
notarize "$image" "${notary[@]}"
progress_ticker_stop
if [ "$notarized" != "Accepted" ]; then
  printf '\033[31m%s\033[0m\n' "The notary did not accept the disk image: ${notarized:-no answer}." >&2
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

# Named the way every file of a release is: the product, the version, the system and the processor.
# Tauri names it after the application, with a space in it. The name is no part of what is signed
# or stapled, so the image is the same one under it.
named="$(dirname "$image")/lyonoyl-mail-$version-macos-$kind.dmg"
mv -f "$image" "$named"
image="$named"

# What this build will and will not do about updates, said out loud rather than discovered later.
#
# The key is compiled in through `option_env!`, so a build made without it is not a build with a
# broken updater — it is a build whose updater never runs and never asks the network anything. That
# is a fine thing to ship and a bad thing to ship by accident, and the only difference between the
# two is whether anybody was told.
if [ -n "${LYONOYL_UPDATE_PUBKEY:-}" ]; then
  say "Updates: on"
  echo "  This build takes an update sealed by the key that was set when it was built, and asks its"
  echo "  person before it looks for one."
  if [ -z "$with_notes" ]; then
    echo "  The release itself, laid out with its notes, is the same with :notes after the kind:"
    echo "  npm run app:release:mac:arm64:notes, :mac:x64:notes or :mac:both:notes."
  fi
else
  say "Updates: off"
  echo "  LYONOYL_UPDATE_PUBKEY was not set, so this build never checks for an update and never"
  echo "  asks the network about one. Nothing in it can be updated in place; a new version is a"
  echo "  new download. scripts/make-update-key.sh makes the key and sets it, once."
fi

if [ -n "$with_notes" ]; then
  progress_step "$STAGE_LAYOUT"
  # With its own questions of the image, its own prompt for the password, and its own account of
  # what goes where, in which order.
  LYONOYL_UPDATE_KEY_PASSWORD="$password" scripts/release-update.sh --notes "release-notes/$version.json" "$image"
  # The text of the GitHub release beside its files, so everything of the version is in one place.
  cp "release-notes/$version.md" "publish/$version/release.md"
  echo "  Everything of $version: publish/$version/ -- files/ and release.md for GitHub,"
  echo "  manifest/ for the site."
fi

progress_end
progress_span "$SECONDS"
say "Done in $progress_span_said"
echo "$image"

