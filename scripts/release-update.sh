#!/usr/bin/env bash
#
# A release: what a person downloads, the update every installed copy takes, and the manifest that
# tells the copies, each of them sealed.
#
#     scripts/release-update.sh --notes <notes.json> [disk image]
#
# Run after `npm run app:release`, against the disk image it made — the newest one under `target`
# unless another is named — from the commit the image was built from. The application in the
# archive is the one a person drags out of that image, so a copy that updates itself and a copy
# downloaded fresh are the same program. A release with an image for each kind of Mac is two runs,
# one per image; the second adds its kind to what the first wrote for the same version, and nothing
# is published until both have run.
#
# The notes are what changed, for a person: one JSON object holding a text for each language the
# window has a catalogue for, under the catalogue's name. Every catalogue gets one,
# because every word a person reads is in the language they chose. A copy shows the one in its
# person's language once it has updated. Needed the first time a version is written down; given
# again, they replace what was written.
#
# It reads the update key from the identity file the release already uses — `LYONOYL_UPDATE_PUBKEY`
# and `LYONOYL_UPDATE_KEY_PATH`, which `scripts/make-update-key.sh` writes — and asks for the key's
# password at the terminal, or takes it from `LYONOYL_UPDATE_KEY_PASSWORD` where a password manager
# hands it over. Never put the password in the identity file: a key with its password beside it is
# a key with no password.
#
# Everything it makes goes under `target/site/`, at the paths every copy asks the project's site
# for:
#
#     releases/<version>/lyonoyl-mail-<version>-macos-<kind>.dmg          what a person downloads
#     releases/<version>/lyonoyl-mail-<version>-macos-<kind>.app.tar.gz   what a copy takes
#     releases/<version>/lyonoyl-mail-<version>-source.tar.gz             the source it was made from
#     releases/<version>/notes.json                                       what changed, every language
#     releases/<version>/SHA256SUMS, SHA256SUMS.minisig                   every file's digest, sealed
#     updates/v1/macos/latest.json, latest.json.minisig                   the manifest, sealed
#
# `<kind>` is `aarch64`, `x86_64`, or `universal` for an image carrying both. The code every copy
# runs reads all of it back before the run ends. It publishes nothing and commits nothing: what goes
# where, and in which order, it says at the end, and `scripts/release-check.sh` asks the site
# afterwards.

set -euo pipefail

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }
no() { printf '\033[31m%s\033[0m\n' "$1" >&2; }

# Taken out of the environment before anything else runs, and kept in a variable nothing inherits.
# Left there, it would reach every program below — node, the signer, cargo, and every build script
# and procedural macro a rebuild runs — and any of them could seal an update with it.
password="${LYONOYL_UPDATE_KEY_PASSWORD:-}"
unset LYONOYL_UPDATE_KEY_PASSWORD

notes=""
image=""
while [ $# -gt 0 ]; do
  case "$1" in
    --notes)
      notes="${2:-}"
      [ -n "$notes" ] || { no "--notes needs the file the notes are in."; exit 1; }
      shift 2
      ;;
    -*)
      no "Not something this takes: $1"
      exit 1
      ;;
    *)
      image="$1"
      shift
      ;;
  esac
done

# Made whole before moving to the root of the tree, so a path given relative to where this was
# started still means what it meant there.
absolute() {
  case "$1" in
    /*) printf '%s' "$1" ;;
    *) printf '%s/%s' "$PWD" "$1" ;;
  esac
}
[ -z "$notes" ] || notes=$(absolute "$notes")
[ -z "$image" ] || image=$(absolute "$image")

cd "$(dirname "$0")/.."

if [ -n "$notes" ] && [ ! -f "$notes" ]; then
  no "There are no notes at $notes"
  exit 1
fi

say "The tree"
# The source published beside the build is this commit, so the tree must be this commit: anything
# changed and not committed would be in the build and missing from its source.
changed=$(git status --porcelain 2>/dev/null || echo "git could not say")
if [ -n "$changed" ]; then
  no "The tree holds changes that are not committed, so the source would not be what was built:"
  printf '%s\n' "$changed" | head -10 >&2
  exit 1
fi
echo "the commit the source is made from: $(git rev-parse --short HEAD)"

say "The update key"
# shellcheck disable=SC1091
. scripts/signing-identity.sh sealing
# Read from the identity file as well, where it does not belong, it leaves the environment the same
# way and is used once.
password="${password:-${LYONOYL_UPDATE_KEY_PASSWORD:-}}"
unset LYONOYL_UPDATE_KEY_PASSWORD
echo "the public half is named, and the secret half is there and readable only by you"

say "The disk image"
if [ -z "$image" ]; then
  # The newest, because the run that made it is the run this follows.
  image=$(find target -type f -name '*.dmg' -print0 2>/dev/null \
    | xargs -0 ls -t 2>/dev/null | head -1 || true)
fi
if [ -z "$image" ] || [ ! -f "$image" ]; then
  no "There is no disk image to make an update from. npm run app:release makes one."
  exit 1
fi
echo "$image"

mounted=$(mktemp -d)
said=$(mktemp)
# `set +e` first: a detach that fails must not stop the rest of the tidying, or turn a finished run
# into a failed one.
made=""
trap 'set +e; hdiutil detach -quiet "$mounted" 2>/dev/null; rmdir "$mounted" 2>/dev/null; rm -f "$said"; [ -z "$made" ] || rm -f "$made"' EXIT
if ! hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mounted" "$image"; then
  no "The disk image could not be opened. If it is open in the Finder, eject it and run this again."
  exit 1
fi

app="$mounted/Lyonoyl Mail.app"
if [ ! -d "$app" ]; then
  no "The disk image does not hold the application."
  exit 1
fi

say "The application in it"
# Asked again rather than taken from the run that made the image. An update goes into every copy
# without Gatekeeper ever looking at it, so the questions Gatekeeper would ask are asked here, of
# exactly what goes into the archive.
codesign --verify --deep --strict "$app" || { no "The application's signature does not hold."; exit 1; }
xcrun stapler validate -q "$app" || { no "No notarization ticket is stapled to the application."; exit 1; }
spctl --assess --type execute "$app" || { no "Gatekeeper refuses the application."; exit 1; }

# Every copy this update reaches holds it to the requirement its own signature carries, and a copy
# whose requirement names no team holds an update to nothing but the seal — so one that does not
# name Apple's authority and a team is refused here, before it ships. Read into a variable and
# matched, never printed: it names the team.
requirement=$(codesign -d -r- "$app" 2>&1 || true)
case "$requirement" in
  *"anchor apple generic"*"certificate leaf[subject.OU]"*) ;;
  *)
    no "The application's signing requirement does not name Apple's authority and a team, so a copy"
    no "that took it would hold every later update to the seal alone."
    exit 1
    ;;
esac
unset requirement
echo "signed, notarized, stapled, held to one team, and accepted by Gatekeeper"

plist="$app/Contents/Info.plist"
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist")
tree=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
if [ "$version" != "$tree" ]; then
  no "The image holds $version and this tree is $tree, so it is not the build this tree made."
  exit 1
fi
echo "version $version"

binary="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")"

# A copy that takes an update carrying no key, or another key, never takes the one after it. Read
# into a variable and then compared, never piped into `grep -q`: under `pipefail` a match that ends
# grep early kills the command upstream and reads as a failure.
carried=$(grep -caF "$LYONOYL_UPDATE_PUBKEY" "$binary" || true)
if [ "$carried" = "0" ]; then
  no "The application in the image does not carry the update key the identity file names."
  no "Every copy that took it would never be updated again. Build it with that key set."
  exit 1
fi
echo "it carries the update key, so a copy that takes it can take the next one"

# The kinds of Mac it holds code for, under the names a copy asks with.
machines=""
for kind in $(lipo -archs "$binary"); do
  case "$kind" in
    arm64) machines="$machines aarch64" ;;
    x86_64) machines="$machines x86_64" ;;
    *)
      no "The application holds code for $kind, which no copy of it asks for."
      exit 1
      ;;
  esac
done
machines="${machines# }"
case "$machines" in
  *" "*) label="universal" ;;
  *) label="$machines" ;;
esac
echo "for $machines"

say "The disk image and the archive"
site="target/site"
out="$site/releases/$version"
manifests="$site/updates/v1/macos"
mkdir -p "$out" "$manifests"
stem="lyonoyl-mail-$version-macos-$label"
name="$stem.app.tar.gz"
archive="$out/$name"
rm -f "$archive"

# The image as it was made and signed, under the name a release gives it.
cp "$image" "$out/$stem.dmg"

# The bundle and nothing about this machine. No extended attributes and no AppleDouble files to
# carry them, and no owner's name: tar writes the name of whoever made an archive into every entry
# unless told otherwise, and an archive published to everybody would carry it.
#
# No time in the compressed stream either, so the same image makes the same archive byte for byte.
# A run repeated after its archive was uploaded then changes nothing that is already published.
COPYFILE_DISABLE=1 tar --options 'gzip:!timestamp' --no-mac-metadata --no-xattrs --no-acls \
  --no-fflags --uid 0 --gid 0 --numeric-owner -czf "$archive" -C "$mounted" "Lyonoyl Mail.app"
bytes=$(stat -f '%z' "$archive")
sha256=$(shasum -a 256 "$archive" | cut -d' ' -f1)
echo "$out/$stem.dmg"
echo "$archive, $bytes bytes"

say "The source"
# This commit as git holds it, under a folder named for the release. The same commit makes the same
# file byte for byte, so a second run for the same version must find the one the first made.
source="$out/lyonoyl-mail-$version-source.tar.gz"
made=$(mktemp)
git archive --format=tar.gz --prefix="lyonoyl-mail-$version/" -o "$made" HEAD
listed=$(tar -tzf "$made")
case "$listed" in
  *"lyonoyl-mail-$version/Cargo.toml"*) ;;
  *)
    rm -f "$made"
    no "The source made from this commit holds no Cargo.toml, so it is not this project's source."
    exit 1
    ;;
esac
unset listed
if [ -f "$source" ] && ! cmp -s "$made" "$source"; then
  rm -f "$made"
  no "The release of $version already holds a source made from another commit. Every image of one"
  no "release is made from one commit: build this one from the commit the first was made from."
  exit 1
fi
mv "$made" "$source"
echo "$source"

say "The manifest and the notes"
# The manifest an earlier run left under target/site is what this one adds to: the first image of a
# release with an image for each kind of Mac, or the release before this one.
LYONOYL_VERSION="$version" LYONOYL_MACHINES="$machines" LYONOYL_FILE="$name" \
  LYONOYL_BYTES="$bytes" LYONOYL_SHA256="$sha256" LYONOYL_NOTES="$notes" \
  LYONOYL_INTO="$manifests/latest.json" LYONOYL_NOTES_INTO="$out/notes.json" \
  node --input-type=module - <<'JS'
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";

const env = process.env;
const refuse = (line) => {
  console.error(`\u001b[31m${line}\u001b[0m`);
  process.exit(1);
};

const numbers = (version) => version.split(".").map(Number);
const newer = (one, other) => {
  const [a, b] = [numbers(one), numbers(other)];
  for (let at = 0; at < 3; at += 1) if (a[at] !== b[at]) return a[at] > b[at];
  return false;
};

const was = existsSync(env.LYONOYL_INTO) ? JSON.parse(readFileSync(env.LYONOYL_INTO, "utf8")) : null;
if (was !== null && newer(was.version, env.LYONOYL_VERSION))
  refuse(`The manifest already names ${was.version}, which is newer than ${env.LYONOYL_VERSION}.`);
const same = was !== null && was.version === env.LYONOYL_VERSION;

// One note for every catalogue the window has, read from the tree that made this version.
const languages = readdirSync("src/shared/i18n/locales")
  .filter((file) => file.endsWith(".json"))
  .map((file) => file.slice(0, -".json".length))
  .sort();

let notes;
if (env.LYONOYL_NOTES) {
  let written;
  try {
    written = JSON.parse(readFileSync(env.LYONOYL_NOTES, "utf8"));
  } catch {
    refuse(`The notes are not JSON: ${env.LYONOYL_NOTES}`);
  }
  if (written === null || typeof written !== "object" || Array.isArray(written))
    refuse("The notes are one object holding a text under each catalogue's name.");
  const strangers = Object.keys(written).filter((language) => !languages.includes(language));
  if (strangers.length > 0)
    refuse(`Notes in ${strangers.join(", ")}, which the window has no catalogue for.`);

  notes = {};
  for (const language of languages) {
    if (!(language in written)) refuse(`There is no note in ${language}.`);
    if (typeof written[language] !== "string") refuse(`The note in ${language} is not a text.`);
    const text = written[language].trim();
    if (text === "") refuse(`The note in ${language} is empty.`);
    notes[language] = text;
  }
} else if (same) {
  notes = was.notes;
} else {
  refuse(
    `The manifest does not name ${env.LYONOYL_VERSION} yet, so it needs the notes: ` +
      `--notes <notes.json> holding a text under each of ${languages.join(", ")}.`,
  );
}

// The kinds this run made replace what the manifest said about them; the others it named for this
// version stay, which is how a second image adds its kind to the first.
const archives = same ? { ...was.archives } : {};
for (const machine of env.LYONOYL_MACHINES.split(" "))
  archives[machine] = {
    file: env.LYONOYL_FILE,
    bytes: Number(env.LYONOYL_BYTES),
    sha256: env.LYONOYL_SHA256,
  };

const manifest = {
  version: env.LYONOYL_VERSION,
  archives: Object.fromEntries(Object.keys(archives).sort().map((kind) => [kind, archives[kind]])),
  notes,
};
// Every letter past ASCII written as an escape, which JSON reads back as the letter.
const written = JSON.stringify(manifest, null, 2).replace(
  /[\u0080-\uffff]/g,
  (letter) => `\\u${letter.charCodeAt(0).toString(16).padStart(4, "0")}`,
);
writeFileSync(env.LYONOYL_INTO, `${written}\n`);
// The same notes as a file of the release, for whoever reads them outside a copy, written as the
// letters themselves.
writeFileSync(env.LYONOYL_NOTES_INTO, `${JSON.stringify(notes, null, 2)}\n`);
console.log(`for ${Object.keys(manifest.archives).join(" and ")}`);
JS

say "The sums"
# Every file of the release by what it hashes to, written the way `shasum` writes them so anybody can
# check a download with the tool they already have. Every file in the folder, whichever run made it:
# what is in the folder is what is published.
# Written beside the folder rather than in it, so the file being written is not among the files it
# names.
summed=$(mktemp)
(
  cd "$out"
  find . -maxdepth 1 -type f ! -name 'SHA256SUMS' ! -name 'SHA256SUMS.minisig' -print \
    | sed 's|^\./||' | LC_ALL=C sort | while IFS= read -r file; do shasum -a 256 "$file"; done
) > "$summed"
mv "$summed" "$out/SHA256SUMS"
sed 's/^[0-9a-f]*  /  /' "$out/SHA256SUMS"

say "Sealing"
if [ -z "$password" ]; then
  # `IFS=` so a passphrase that starts or ends with a space is read as it was typed.
  if ! IFS= read -rsp "The update key's password: " password </dev/tty; then
    no "There is no terminal to ask for the password at, and LYONOYL_UPDATE_KEY_PASSWORD is not set."
    exit 1
  fi
  echo >&2
fi

seal() {
  # The password reaches the tool through its environment and never through a command line, which
  # any process on this machine can read.
  # A key handed over in the environment as well would collide with the one named here.
  if ! env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PATH \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$password" \
    npx tauri signer sign -f "$LYONOYL_UPDATE_KEY_PATH" "$1" >/dev/null 2>"$said"; then
    no "The seal could not be made over $1 -- a wrong password, most likely. What the tool said:"
    sed 's/^/  /' "$said" >&2
    exit 1
  fi

  # The tool writes minisign wrapped in one more layer of base64, under `.sig`. Every copy reads
  # minisign as minisign, and minisign names a seal `.minisig`, so it is unwrapped and named here,
  # once.
  base64 --decode < "$1.sig" > "$1.minisig"
  rm -f "$1.sig"
}

seal "$out/SHA256SUMS"
seal "$manifests/latest.json"
unset password
echo "the sums and the manifest are sealed"

say "What every copy makes of it"
# Read back by the code every copy runs — the seals, the manifest's shape, the sums and every file
# they name, what the folder holds besides, the archive's size and digest, the application inside
# and its signature, its version and the kinds of Mac it holds code for — through a test that runs
# only when asked to. One that is not found passes nothing, so the count is checked as well as the
# result.
asked=$(
  LYONOYL_RELEASE_PUBKEY="$LYONOYL_UPDATE_PUBKEY" \
    LYONOYL_RELEASE_MANIFEST="$PWD/$manifests/latest.json" \
    LYONOYL_RELEASE_FILES="$PWD/$out" \
    LYONOYL_RELEASE_APP="$app" \
    cargo test -p lyonoyl --lib -- --ignored --exact \
    infrastructure::updates::before_publishing::every_copy_takes_the_release_on_this_disk 2>&1
) || {
  printf '%s\n' "$asked" | tail -40 >&2
  no "A copy would refuse this release, so it is not one to publish."
  exit 1
}
case "$asked" in
  *"test result: ok. 1 passed"*) echo "every copy takes it" ;;
  *)
    printf '%s\n' "$asked" | tail -20 >&2
    no "The check of the release did not run, so nothing is known about it."
    exit 1
    ;;
esac

say "Ready, and nothing is published"
echo "  Every image of $version first: a release for both kinds of Mac is a second run, with the"
echo "  second image, before anything below. In this order, then:"
echo
echo "  1. The files of the release, so that lyonoyl.com answers for releases/$version/ with"
echo "     every file in"
echo "       $out/"
echo "  2. scripts/release-check.sh $version, which brings every file back from the site and holds"
echo "     it to the sums made here."
echo "  3. Then the manifest, so that lyonoyl.com answers for updates/v1/macos/ with what is in"
echo "       $manifests/"
echo "     It goes last: a manifest out before its files sends every copy after a file that is not"
echo "     there."
echo "  4. scripts/release-check.sh $version once more, which says the manifest is out."
