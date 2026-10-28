#!/usr/bin/env bash
#
# Empties the development mail server and fills it again from the message corpus.
#
# Development mailboxes accumulate: a message archived by hand yesterday, a folder a test emptied,
# a copy delivered twice. None of it is wrong, and all of it makes the next thing hard to read —
# a list that shows the same message twice, a count that disagrees with what is on screen, a test
# whose fixture somebody has already moved. This puts every account back to the same known mail.
#
# Only the development server, which listens on loopback and holds nothing anybody would miss.
#
# `--if-fresh` fills the development accounts only when none of them holds a single message in any
# folder, which is a server started on an empty volume, and empties nothing: no mailbox, and not
# the delivery queue. A server where any of them holds mail is left exactly as it is, because that
# mail is what somebody was looking at. `scripts/preflight.sh` runs it this way.
set -euo pipefail

only_if_fresh=0
case "${1:-}" in
  "") ;;
  --if-fresh) only_if_fresh=1 ;;
  *)
    echo "usage: $0 [--if-fresh]" >&2
    exit 2
    ;;
esac

CONTAINER=lyonoyl-mail-server
SUBMISSION=lyonoyl-mail-submission
CORPUS="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/eml"

# The development accounts, which are the ones a person clicking around will be looking at. Test
# mailboxes are not filled here: each test delivers what it needs into a mailbox of its own, which
# is what keeps a test from depending on what somebody clicked.
ACCOUNTS=(test@lyonoyl.local zeta@first-domain.test kappa@second-domain.test)
FOLDERS=(Drafts Sent Junk Trash Archive)

for container in "$CONTAINER" "$SUBMISSION"; do
  if ! docker ps --format '{{.Names}}' | grep -qx "$container"; then
    echo "The development mail server is not running: $container is missing." >&2
    echo "Start it with: docker compose -f docker/mail-server/compose.yml up -d --build --wait" >&2
    exit 1
  fi
done

if [ ! -d "$CORPUS" ]; then
  echo "The message corpus is missing: $CORPUS" >&2
  exit 1
fi

messages=$(find "$CORPUS" -name '*.eml' | wc -l | tr -d ' ')
if [ "$messages" -lt 1 ]; then
  echo "The message corpus is empty: $CORPUS" >&2
  exit 1
fi

if [ "$only_if_fresh" -eq 1 ]; then
  for address in "${ACCOUNTS[@]}"; do
    # Every folder at once: an account whose inbox was archived away still holds mail.
    if ! held=$(docker exec "$CONTAINER" doveadm mailbox status -u "$address" -t messages '*' \
      | tr -d '\r'); then
      echo "Could not ask the development server what ${address} holds." >&2
      exit 1
    fi
    case "$held" in
      messages=0) ;;
      messages=[0-9]*)
        echo "The development accounts hold mail (${address}: ${held#messages=}); left as they are."
        exit 0
        ;;
      *)
        echo "Could not tell whether ${address} holds mail: ${held}" >&2
        exit 1
        ;;
    esac
  done
  echo "None of the development accounts holds a message, so this is a fresh server; filling them."
else
  # The queue first. A message the submission server is still holding would be delivered into a
  # mailbox this has just emptied, and the known state would have one message in it nobody put
  # there.
  echo "Dropping anything waiting to be delivered..."
  docker exec "$SUBMISSION" postsuper -d ALL >/dev/null 2>&1

  echo "Emptying every mailbox on the development server..."
  docker exec "$CONTAINER" sh -c 'rm -rf /srv/mail/*'
fi

for address in "${ACCOUNTS[@]}"; do
  echo "Filling ${address}..."

  # Created before anything is delivered: an account whose folders appear one at a time reads as an
  # account that is still arriving. An account `--if-fresh` fills may have them already, which is
  # the one refusal that is not a failure.
  for folder in "${FOLDERS[@]}"; do
    if ! said=$(docker exec "$CONTAINER" doveadm mailbox create -u "$address" "$folder" 2>&1); then
      case "$said" in
        *"already exists"*) ;;
        *)
          echo "$said" >&2
          exit 1
          ;;
      esac
    fi
  done

  for message in "$CORPUS"/*.eml; do
    docker exec -i "$CONTAINER" doveadm save -u "$address" -m INBOX < "$message"
  done

  # A few of them read, so the unread count is a number rather than "all of them".
  docker exec "$CONTAINER" doveadm flags add -u "$address" '\Seen' mailbox INBOX 1:3 >/dev/null

  # Something in Sent and something filed away, so those folders are not empty for no reason. Sent
  # mail is read by whoever sent it, which is what the interface has to be given a chance to show.
  for message in $(find "$CORPUS" -name '*.eml' | sort | head -2); do
    docker exec -i "$CONTAINER" doveadm save -u "$address" -m Sent < "$message"
  done
  for message in $(find "$CORPUS" -name '*.eml' | sort | tail -2); do
    docker exec -i "$CONTAINER" doveadm save -u "$address" -m Archive < "$message"
  done
  docker exec "$CONTAINER" doveadm flags add -u "$address" '\Seen' mailbox Sent all >/dev/null

  held=$(docker exec "$CONTAINER" doveadm mailbox status -u "$address" messages INBOX | tr -d '\r')
  echo "  INBOX: $held"
  case "$held" in
    *"messages=$messages"*) ;;
    *)
      echo "  the inbox holds something other than the whole corpus" >&2
      exit 1
      ;;
  esac
done

echo
echo "Done. The mail on this device is derived and is rebuilt from this: to start from nothing"
echo "there too, quit the application and delete the accounts folder from its data directory."
