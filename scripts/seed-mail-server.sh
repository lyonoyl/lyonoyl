#!/usr/bin/env bash
# Fills the development mail server with the message corpus.
#
#   docker compose -f docker/mail-server/compose.yml up -d --build
#   scripts/seed-mail-server.sh [user] [copies]
#
# `user` defaults to test@lyonoyl.local, `copies` to 1. Every copy repeats the corpus, which is how
# a mailbox large enough to be interesting is built.
set -euo pipefail
cd "$(dirname "$0")/.."

CONTAINER=lyonoyl-mail-server
USER_NAME="${1:-test@lyonoyl.local}"
COPIES="${2:-1}"
CORPUS=tests/fixtures/eml

if ! docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"; then
  echo "The mail server is not running. Start it with:" >&2
  echo "  docker compose -f docker/mail-server/compose.yml up -d --build" >&2
  exit 1
fi

# Which corpus message goes to which mailbox. Everything else lands in the inbox.
mailbox_for() {
  case "$1" in
    *invoice*|*attachment*) echo "Archive" ;;
    *reply*) echo "Sent" ;;
    *) echo "INBOX" ;;
  esac
}

stored=0
for ((copy = 1; copy <= COPIES; copy++)); do
  for file in "$CORPUS"/*.eml; do
    mailbox=$(mailbox_for "$(basename "$file")")
    docker exec -i "$CONTAINER" doveadm save -u "$USER_NAME" -m "$mailbox" < "$file"
    stored=$((stored + 1))
  done
done

echo "Stored $stored messages for $USER_NAME."
docker exec "$CONTAINER" doveadm mailbox status -u "$USER_NAME" messages unseen uidvalidity '*'
