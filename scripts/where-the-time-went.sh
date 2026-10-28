#!/usr/bin/env bash
#
# What the last run spent reading mail, out of its own journal.
#
# The question "why is updating so slow" has one honest answer and it is not a guess: the journal
# records what each pass cost and where its time went (sync.read), what each push told the server
# and handed over (push.finished), what reading back a burst of pushes cost (push.readBack), and
# how many times the application said hello to a server. This reads the newest journal and says it
# in one screen.
#
# Nothing here reaches a network or a mailbox. It reads counts and durations, which is all the
# journal holds — no address, no mailbox name, no subject.

set -euo pipefail

logs="${LYONOYL_LOGS:-$HOME/Library/Application Support/Lyonoyl Mail/logs}"
if [ ! -d "$logs" ]; then
    echo "No journal here: $logs" >&2
    echo "On Linux look under ~/.local/share/lyonoyl-mail/logs, on Windows under %LOCALAPPDATA%." >&2
    exit 1
fi

newest=$(ls -t "$logs"/*.log 2>/dev/null | head -1 || true)
if [ -z "$newest" ]; then
    echo "The journal directory is empty: $logs" >&2
    exit 1
fi

echo "Journal: $newest"
echo

rounds=$(grep -c "round.finished" "$newest" || true)
passes=$(grep -c "sync.read" "$newest" || true)

if [ "$passes" = "0" ]; then
    echo "No pass has been recorded yet."
    echo "A round comes every two minutes; the journal must also be kept at the detail level."
    exit 0
fi

echo "Rounds recorded: $rounds        Passes recorded: $passes"
echo

# What happened before any mail was read. Without these entries the journal starts at the first
# pass, and the seconds a window spends before that are in no record at all.
if grep -q "start\." "$newest"; then
    echo "Starting up:"
    grep "start\." "$newest" | sed 's/^/  /'
    first=$(grep -m1 "sync.read" "$newest" | awk '{print $2}')
    if [ -n "$first" ]; then
        echo "  first mailbox read at ${first}ms"
    fi
    echo
fi

# One line per pass, newest last, with the split. `took` is the whole pass.
echo "Each pass — how long, and where it went:"
grep "sync.read" "$newest" | tail -20 | sed 's/^/  /'
echo

if [ "$rounds" != "0" ]; then
    echo "Each round — every account together:"
    grep "round.finished" "$newest" | tail -10 | sed 's/^/  /'
    echo
fi

# What a person did, told to the server: how long, what it carried, and the read that followed
# only where it moved a message.
if grep -q "push.finished" "$newest"; then
    echo "Each push — telling a server what was done, and handing over what was written:"
    grep -E "push\.(finished|readBack)" "$newest" | tail -20 | sed 's/^/  /'
    echo
fi

# The number that decides everything else. A unit of work may not carry its own connection:
# see docs/performance/response-budgets.md.
echo "Hellos to a server, per pass:"
grep "sync.read" "$newest" \
    | sed -n 's/.*opened=\([0-9]*\).*folders=\([0-9]*\).*/  opened=\1 over \2 folders/p' \
    | sort | uniq -c | sort -rn | head -10
echo

# Which phase spends them. Four numbers, four different defects.
echo "…and where a pass spends its time:"
grep "sync.read" "$newest" | tail -10 \
    | sed -n 's/.*inList=\([0-9]*\)ms inFolders=\([0-9]*\)ms inBackfill=\([0-9]*\)ms inBodies=\([0-9]*\)ms.*/  listing=\1ms folders=\2ms backfill=\3ms bodies=\4ms/p'
echo

echo "…and which part of a pass opened them:"
grep "sync.read" "$newest" \
    | sed -n 's/.*onList=\([0-9]*\) onFolders=\([0-9]*\) onBackfill=\([0-9]*\) onBodies=\([0-9]*\).*/  listing=\1 folders=\2 backfill=\3 bodies=\4/p' \
    | sort | uniq -c | sort -rn | head -10
echo

# Of the time a pass took, how much was saying hello rather than reading mail. This is the number
# that says whether the connection or the server is the slow part.
echo "Of each pass, how much was hello and goodbye:"
grep "sync.read" "$newest" | tail -10 \
    | sed -n 's/.*took=\([0-9]*\)ms .*saidHello=\([0-9]*\)ms.*/\1 \2/p' \
    | awk '{ if ($1 > 0) printf "  %6d ms of %6d ms — %d%%\n", $2, $1, ($2 * 100) / $1 }'
echo

# A pass that asked fifty times and got nothing looks like a quiet account without this.
asked_and_missed=$(grep "sync.read" "$newest" | sed -n 's/.*bodiesFailed=\([0-9]*\).*/\1/p' \
    | awk '{total += $1} END {print total + 0}')
if [ "$asked_and_missed" != "0" ]; then
    echo "Messages asked for whole and not got: $asked_and_missed"
    echo "That is conversations with a server that ended in nothing, and they cost the same as the ones that did not."
fi
