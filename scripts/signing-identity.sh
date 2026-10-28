#!/usr/bin/env bash
#
# Reads the signing identity, and refuses before anything is built rather than after.
#
# Sourced, not run: it puts values in the environment for the script that sourced it. One argument —
# `signing`, `notarizing`, `sealing` or `checking` — says how much is needed. A development build is
# signed and nothing else; a release is also notarized, which needs an App Store Connect key on top;
# an update is sealed with the update key, which needs nothing from Apple at all; and a release out
# on its two places is checked with the key's public half, which needs nothing secret.
#
# Nothing here is named in this repository (AGENTS.md invariant 21). The values live in a file the
# developer keeps outside the tree, and nothing here ever prints one — what it prints is whether a
# thing is there.

want="${1:?signing, notarizing, sealing or checking}"

no() { printf '\033[31m%s\033[0m\n' "$1" >&2; }

IDENTITY_FILE="${LYONOYL_SIGNING_ENV:-$HOME/.config/lyonoyl/signing.env}"

if [ ! -f "$IDENTITY_FILE" ]; then
  if [ "$want" = "sealing" ] || [ "$want" = "checking" ]; then
    no "There is no identity file, so there is no update key to seal or check with."
    no "scripts/make-update-key.sh makes the key and writes what a release reads into: $IDENTITY_FILE"
    exit 1
  fi
  no "There is no identity file, so this build would be signed with nothing."
  no "An unsigned build gets a new identity every time it is rebuilt, which is why the system keeps"
  no "asking about the keychain: what it holds is bound to the signature it was written under."
  no "Put one at: $IDENTITY_FILE"
  exit 1
fi

# Parsed before it is read, because a syntax error inside a sourced file ends the shell outright
# rather than returning a failure something here could catch -- so the check has to happen first.
if ! bash -n "$IDENTITY_FILE" 2>/dev/null; then
  no "That identity file is not something a shell can read."
  no "Each value goes in single quotes -- a certificate name holds spaces and brackets:"
  no "    APPLE_SIGNING_IDENTITY='Developer ID Application: Something (XXXXXXXXXX)'"
  exit 1
fi

set -a
# shellcheck disable=SC1090
. "$IDENTITY_FILE"
set +a

case "$want" in
  signing) needed="APPLE_SIGNING_IDENTITY" ;;
  notarizing)
    # Three, and only three. The notary is asked with an App Store Connect key, and on that route
    # the team is carried by the key itself -- a team identifier is read only when notarizing as an
    # Apple ID and a password, which this does not do. Demanding it here would refuse a correct
    # identity file over a value nothing reads.
    needed="APPLE_SIGNING_IDENTITY APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH"
    ;;
  sealing)
    # The two halves of the update key: the public one every build compiles in, and where the
    # secret one is kept.
    needed="LYONOYL_UPDATE_PUBKEY LYONOYL_UPDATE_KEY_PATH"
    ;;
  checking)
    # The public half alone: a seal is checked with it, and nothing is made.
    needed="LYONOYL_UPDATE_PUBKEY"
    ;;
  *)
    no "Not something there is an identity for: $want"
    exit 1
    ;;
esac

missing=0
for name in $needed; do
  if [ -z "${!name:-}" ]; then
    no "$name is not set in the identity file"
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 1

# Matched with the quotes the listing puts around a name, so the whole name has to be there. Without
# them this passes on any prefix of one -- `Developer ID Application:` alone satisfies it -- and a
# check a fragment can satisfy is a check that says yes to a name `codesign` will later refuse or,
# worse, find two of.
# Read into a variable first. Piped into `grep -q` this works only while the listing is short enough
# to be written before grep exits -- past that the upstream command dies of a broken pipe and
# `pipefail` turns a certificate that IS present into a refusal.
if [ "$want" != "sealing" ] && [ "$want" != "checking" ]; then
  identities=$(security find-identity -v -p codesigning 2>/dev/null || true)
  case "$identities" in
    *"\"$APPLE_SIGNING_IDENTITY\""*) ;;
    *)
      no "That certificate is not in this machine's keychain, or its private key is missing."
      exit 1
      ;;
  esac
fi

if [ "$want" = "notarizing" ]; then
  if [ ! -f "$APPLE_API_KEY_PATH" ]; then
    no "The notarization key is not at the path the identity file gives."
    exit 1
  fi

  # A key anybody on this machine can read is a key that notarizes as this team.
  permissions=$(stat -f '%OLp' "$APPLE_API_KEY_PATH")
  if [ "$permissions" != "600" ] && [ "$permissions" != "400" ]; then
    no "The notarization key is readable by more than its owner (mode $permissions)."
    no "chmod 600 it, then run this again."
    exit 1
  fi
fi

if [ "$want" = "sealing" ]; then
  if [ ! -f "$LYONOYL_UPDATE_KEY_PATH" ]; then
    no "The update key is not at the path the identity file gives."
    exit 1
  fi

  # A key anybody on this machine can read is a key that seals an update as this project.
  permissions=$(stat -f '%OLp' "$LYONOYL_UPDATE_KEY_PATH")
  if [ "$permissions" != "600" ] && [ "$permissions" != "400" ]; then
    no "The update key is readable by more than its owner (mode $permissions)."
    no "chmod 600 it, then run this again."
    exit 1
  fi
fi
