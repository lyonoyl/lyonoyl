#!/usr/bin/env bash
#
# Hands a file to Apple's notary and asks after it until it answers, saying meanwhile what it says.
#
# Sourced by `scripts/release-macos.sh`, after `scripts/progress.sh`, not run. `notarytool submit
# --wait` asks the same, and prints nothing at all until the answer, which takes anything from a
# minute to an hour: a release that has said nothing for twenty minutes reads as one that has stopped.
# Asked every half minute instead, the notary's own word for where the file stands goes on the line
# `scripts/progress.sh` keeps at the bottom.
#
#     notarize <file> <how notarytool signs in...>
#
# Sets `notarized` to the status the notary gave the file, or to nothing where it never answered, and
# `submission` to what the notary calls the upload, or to nothing where the upload never went.

# Seconds between two questions, and how many questions in a row may go unanswered before it stops
# asking: five minutes without a word from the notary is a network that is gone.
notary_every=30
notary_unanswered=10

# shellcheck disable=SC2034 # what it sets is read by the script that sourced this one
notarize() {
  local file=$1 answer said unanswered=0
  shift
  notarized=""
  submission=""

  progress_note "uploading to the notary"
  answer=$(xcrun notarytool submit "$file" "$@" --output-format json 2>/dev/null) || true
  submission=$(plutil -extract id raw -o - - <<<"$answer" 2>/dev/null) || submission=""
  if [ -z "$submission" ]; then
    progress_note ""
    return 0
  fi

  while :; do
    answer=$(xcrun notarytool info "$submission" "$@" --output-format json 2>/dev/null) || true
    said=$(plutil -extract status raw -o - - <<<"$answer" 2>/dev/null) || said=""
    case "$said" in
      "In Progress")
        unanswered=0
        progress_note "notary: In Progress"
        ;;
      "")
        unanswered=$((unanswered + 1))
        if [ "$unanswered" -ge "$notary_unanswered" ]; then break; fi
        progress_note "notary: no answer, asking again"
        ;;
      *)
        notarized=$said
        break
        ;;
    esac
    sleep "$notary_every"
  done
  progress_note ""
  return 0
}
