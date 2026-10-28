#!/usr/bin/env bash
# Every licence that travels inside this application, from both halves of it.
#
# What each permissive licence in the tree asks for is the same thing: the notice travels with the
# code. So it is collected here, committed, and embedded — and `scripts/preflight.sh` runs this and
# fails on any difference, because a dependency added without its notice is a dependency shipped
# without its licence, and a licence nobody agreed to arrives the same way.
#
# Only what ships. Build tools are on this machine and on nobody else's, except the few whose own
# output is compiled into the window, which `scripts/collect-npm-licences.mjs` names.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "the window's half"
node scripts/collect-npm-licences.mjs

echo "the core's half"
(cd src-tauri && cargo about generate about.hbs -o licences-rust.json)

echo "the C libraries inside it"
node scripts/collect-vendored-licences.mjs

echo "one file"
node scripts/merge-licences.mjs
