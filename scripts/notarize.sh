#!/usr/bin/env bash
#
# Hands a file to Apple's notary and asks after it until it answers, putting what it says on the
# bottom line: `notarytool submit --wait` prints nothing until the end, which can be an hour away.
#
#     notarize <file> <how notarytool signs in...>
#
# Sourced by `scripts/release-macos.sh`, after `scripts/progress.sh`. Sets `notarized` to the status
# the notary gave, and `submission` to the upload's identifier; each is empty where there was none.

# Seconds between questions, and unanswered questions in a row before it gives up.
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
