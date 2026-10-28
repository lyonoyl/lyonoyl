#!/usr/bin/env bash
#
# Whether every place the project publishes to holds the release this machine made, asked the way
# every copy asks: through the same providers, in the same order.
#
#     scripts/release-check.sh <version>
#
# Run once the places hold the files `scripts/release-update.sh` made under
# `target/site/releases/<version>/` — lyonoyl.com answering for `releases/<version>/` — and again once
# they hold the manifest. It brings every file of the release back from each place, holds each to
# the sums sealed here, and says whether each place hands over the manifest made here yet. A place
# that answers a browser and refuses a copy — a redirect chain it cannot follow, a challenge in
# front of the files, a certificate it does not take — is found here rather than by every copy.
#
# It needs the update key's public half from the identity file, and nothing secret. It changes
# nothing, here or anywhere it asks.

set -euo pipefail

say() { printf '\n\033[1m%s\033[0m\n' "$1"; }
no() { printf '\033[31m%s\033[0m\n' "$1" >&2; }

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  no "Which release? scripts/release-check.sh <version>, three numbers."
  exit 1
fi

cd "$(dirname "$0")/.."

files="target/site/releases/$version"
manifest="target/site/updates/v1/macos/latest.json"
if [ ! -f "$files/SHA256SUMS" ]; then
  no "There is no release of $version made here: $files holds no SHA256SUMS."
  no "scripts/release-update.sh makes one."
  exit 1
fi
if [ ! -f "$manifest" ] || [ ! -f "$manifest.minisig" ]; then
  no "There is no sealed manifest in $(dirname "$manifest")/ to compare the places' with."
  exit 1
fi

say "The update key"
# shellcheck disable=SC1091
. scripts/signing-identity.sh checking
echo "the public half is named"

say "What each place holds"
report=$(mktemp)
trap 'rm -f "$report"' EXIT
asked=$(
  LYONOYL_RELEASE_PUBKEY="$LYONOYL_UPDATE_PUBKEY" \
    LYONOYL_RELEASE_VERSION="$version" \
    LYONOYL_RELEASE_FILES="$PWD/$files" \
    LYONOYL_RELEASE_MANIFEST="$PWD/$manifest" \
    LYONOYL_RELEASE_REPORT="$report" \
    cargo test -p lyonoyl --lib -- --ignored --exact \
    infrastructure::updates::before_publishing::every_place_holds_the_release 2>&1
) && held=yes || held=no

# What the check found, whichever way it went; a report it never wrote is a check that never ran.
if [ ! -s "$report" ]; then
  printf '%s\n' "$asked" | tail -30 >&2
  no "The check did not run, so nothing is known about any place."
  exit 1
fi
sed 's/^/  /' "$report"

case "$held:$asked" in
  yes:*"test result: ok. 1 passed"*)
    say "Every place holds every file of $version"
    echo "  Once the manifest is out, this says so above: then the release is out."
    ;;
  *)
    no "Not every place holds the release made here, so the manifest does not go out yet."
    exit 1
    ;;
esac
