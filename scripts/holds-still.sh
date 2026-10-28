#!/usr/bin/env bash
#
# Whether a build's identity survives the next rebuild.
#
# The keychain keeps a secret behind a rule saying which application may read it, and that rule is
# the application's *designated requirement*. An unsigned build has no identity to name, so the rule
# is the hash of the binary — which is different the moment anything is rebuilt, and the secret is
# then unreadable by the same application. A signed build's rule names the bundle and the
# certificate instead, and holds still for ever.
#
# So this asks for the rule and refuses one with a hash in it. That is the property worth checking:
# "it is signed" is not the same claim, and a build can be signed and still be named by its hash.

set -euo pipefail

app="${1:?the application to ask about}"

rule=$(codesign -d -r- "$app" 2>&1 | tail -1)

case "$rule" in
  *cdhash*)
    printf '\033[31mThis build is named by the hash of its own binary.\033[0m\n' >&2
    printf '\033[31mThe next rebuild is a different application as far as the keychain is concerned.\033[0m\n' >&2
    exit 1
    ;;
  *"anchor apple generic"*)
    echo "identity holds still: named by the bundle and the certificate, not by a hash"
    ;;
  *)
    printf '\033[31mThis build is named by something unexpected, so nothing can be said about it.\033[0m\n' >&2
    exit 1
    ;;
esac
