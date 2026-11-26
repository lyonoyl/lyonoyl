#!/usr/bin/env bash
#
# How far a long run has got, and how long it has left.
#
# Sourced by `scripts/preflight.sh` and `scripts/release-macos.sh`, not run. A run names its steps
# up front, and each step then says which it is of how many, how long the run has taken so far, and
# how long it has left: what each step took the last time it passed on this machine, kept in
# `target/progress/<run>.tsv`, one step a line, with its seconds and, where it counts something, how
# many it counted. Until every step left has passed here once there is no estimate, and the line says
# so rather than guessing.
#
# A step whose output goes to a log shows nothing for as long as it runs, and a screen that shows
# nothing for minutes is a run that looks stuck. While one runs, `progress_ticker_start` keeps a line
# at the bottom saying how long it has been going and how long is left: redrawn every few seconds on
# a terminal, and printed once a minute anywhere else. Output passed through `progress_relay` scrolls
# above that line, and anywhere but a terminal the line is printed only once the output has been
# quiet for a minute.
#
# Bash 3.2, which macOS ships: no associative arrays, and no empty array expanded under `set -u`. A
# subshell and a pipeline count `$SECONDS` from the same start as the script, so every measure here
# is `$SECONDS`. Every function returns 0: a run is never stopped by the account of how it is going.

progress_word=Step
progress_titles=()
# What each step took the last time it passed here, and what it counted; what another run says it
# will take, where one is given; and what it took in this run, and whether it passed.
progress_last=()
progress_last_counts=()
progress_expected=()
progress_took=()
progress_counts=()
progress_passed=()
progress_at=-1
progress_failed_here=0
progress_began=0
progress_run_began=0
progress_file=""
progress_beat=""
progress_note_file=""
progress_ticker=""
progress_parent=$$
# How often the bottom line is redrawn on a terminal, and how long output may be quiet anywhere else
# before a line says the run is still going, in seconds.
progress_every=5
progress_quiet=60

# A count of seconds or of things, and nothing else.
progress_is_count() {
  case ${1:-} in
    '' | *[!0-9]*) return 1 ;;
  esac
  return 0
}

# Sets `progress_found` to where the step named $1 stands among the run's steps, or -1.
progress_find() {
  progress_found=-1
  local i=0
  while [ "$i" -lt "${#progress_titles[@]}" ]; do
    if [ "${progress_titles[$i]}" = "$1" ]; then
      progress_found=$i
      return 0
    fi
    i=$((i + 1))
  done
  return 0
}

# progress_begin <run> <what a step is called> <the steps, one a line, in the order they run>
progress_begin() {
  progress_word=$2
  progress_titles=()
  progress_last=()
  progress_last_counts=()
  progress_expected=()
  progress_took=()
  progress_counts=()
  progress_passed=()
  local title name seconds count
  while IFS= read -r title; do
    if [ -z "$title" ]; then continue; fi
    progress_titles+=("$title")
    progress_last+=("")
    progress_last_counts+=("")
    progress_expected+=("")
    progress_took+=("")
    progress_counts+=("")
    progress_passed+=(0)
  done <<<"$3"

  mkdir -p target/progress 2>/dev/null || true
  progress_file="target/progress/$1.tsv"
  progress_beat="target/progress/$1.beat"
  progress_note_file="target/progress/$1.note"
  : 2>/dev/null >"$progress_note_file" || true

  # The last run's times, matched to this run's steps by name. A line that is not a step and a time
  # is passed over, so a file somebody edited costs an estimate and never the run.
  if [ -f "$progress_file" ]; then
    while IFS=$'\t' read -r name seconds count || [ -n "$name" ]; do
      progress_find "$name"
      if [ "$progress_found" -lt 0 ] || ! progress_is_count "$seconds"; then continue; fi
      progress_last[progress_found]=$seconds
      if progress_is_count "$count"; then progress_last_counts[progress_found]=$count; fi
    done <"$progress_file"
  fi

  progress_at=-1
  progress_run_began=$SECONDS
  progress_parent=$$
  return 0
}

# progress_expect <step> <seconds>: how long a step will take, from somewhere other than its own last
# time, used instead of it. Nothing, or anything but a count, leaves the step without an estimate.
progress_expect() {
  progress_find "$1"
  if [ "$progress_found" -lt 0 ]; then return 0; fi
  if progress_is_count "${2:-}"; then
    progress_expected[progress_found]=$2
  else
    progress_expected[progress_found]=none
  fi
  return 0
}

# Sets `progress_total` to how long every step of <run> took the last time each passed here, or to
# nothing where that run has no times kept.
progress_total_of() {
  progress_total=""
  local file="target/progress/$1.tsv" name seconds rest sum=0
  if [ ! -f "$file" ]; then return 0; fi
  while IFS=$'\t' read -r name seconds rest || [ -n "$name" ]; do
    if progress_is_count "$seconds"; then sum=$((sum + seconds)); fi
  done <"$file"
  # shellcheck disable=SC2034 # read by the script that sourced this one
  progress_total=$sum
  return 0
}

# Sets `progress_span_said` to <seconds> in words: 45 s, 3 min 5 s, 1 h 2 min.
progress_span() {
  local s=${1:-0}
  progress_is_count "$s" || s=0
  if [ "$s" -lt 60 ]; then
    progress_span_said="$s s"
  elif [ "$s" -lt 3600 ] && [ $((s % 60)) -eq 0 ]; then
    progress_span_said="$((s / 60)) min"
  elif [ "$s" -lt 3600 ]; then
    progress_span_said="$((s / 60)) min $((s % 60)) s"
  else
    progress_span_said="$((s / 3600)) h $((s % 3600 / 60)) min"
  fi
  return 0
}

# Sets `progress_about_said` to a time still to come, in words, rounded up to the minute.
progress_about() {
  local s=${1:-0} minutes
  progress_is_count "$s" || s=0
  if [ "$s" -lt 60 ]; then
    progress_about_said="under a minute"
    return 0
  fi
  minutes=$(((s + 59) / 60))
  if [ "$minutes" -lt 60 ]; then
    progress_about_said="about $minutes min"
  else
    progress_about_said="about $((minutes / 60)) h $((minutes % 60)) min"
  fi
  return 0
}

# Sets `progress_so_far` to how long the run has taken, and `progress_to_go` to how long it has
# left and how far through it is, or to nothing where a step still to run has no time to go by.
progress_where() {
  local spent=$((SECONDS - progress_run_began)) in_step=$((SECONDS - progress_began))
  local left=0 i=$progress_at last percent=0
  progress_span "$spent"
  progress_so_far=$progress_span_said
  progress_to_go=""
  if [ "$i" -lt 0 ]; then return 0; fi
  while [ "$i" -lt "${#progress_titles[@]}" ]; do
    last=${progress_expected[$i]}
    if [ -z "$last" ]; then last=${progress_last[$i]}; fi
    if ! progress_is_count "$last"; then return 0; fi
    if [ "$i" -eq "$progress_at" ]; then
      if [ "$last" -gt "$in_step" ]; then left=$((left + last - in_step)); fi
    else
      left=$((left + last))
    fi
    i=$((i + 1))
  done
  if [ $((spent + left)) -gt 0 ]; then percent=$((spent * 100 / (spent + left))); fi
  if [ "$percent" -gt 99 ]; then percent=99; fi
  progress_about "$left"
  progress_to_go="$progress_about_said to go · $percent %"
  return 0
}

# progress_step <step>: the step named begins, and the one before it has ended.
progress_step() {
  progress_close
  progress_find "$1"
  progress_at=$progress_found
  progress_failed_here=0
  progress_began=$SECONDS
  if [ "$progress_at" -ge 0 ]; then
    printf '\n\033[1m%s %s of %s · %s\033[0m\n' "$progress_word" "$((progress_at + 1))" "${#progress_titles[@]}" "$1"
  else
    printf '\n\033[1m%s\033[0m\n' "$1"
  fi
  progress_where
  if [ -n "$progress_to_go" ]; then
    printf '\033[2m  %s in · %s\033[0m\n' "$progress_so_far" "$progress_to_go"
  else
    printf '\033[2m  %s in · no estimate yet: every step left has to pass here once first\033[0m\n' "$progress_so_far"
  fi
  return 0
}

# The step going now has failed, so its time is not one to go by.
progress_failed() {
  progress_failed_here=1
  return 0
}

# progress_counted <n>: how many things the step going now counted, for the next run to count to.
progress_counted() {
  if [ "$progress_at" -ge 0 ] && progress_is_count "${1:-}"; then
    progress_counts[progress_at]=$1
  fi
  return 0
}

# progress_note <text>: a word from whatever the step is waiting on, for the bottom line to carry.
# Nothing takes it off.
progress_note() {
  if [ -n "$progress_note_file" ]; then
    printf '%s\n' "${1:-}" 2>/dev/null >"$progress_note_file" || true
  fi
  return 0
}

# The step going now has ended, as passed unless it was said to have failed.
progress_close() {
  progress_ticker_stop
  if [ "$progress_at" -ge 0 ]; then
    progress_took[progress_at]=$((SECONDS - progress_began))
    if [ "$progress_failed_here" -eq 0 ]; then progress_passed[progress_at]=1; fi
  fi
  progress_at=-1
  return 0
}

# Sets `progress_span_said` to how long the step named took in this run, or to nothing.
progress_time_of() {
  progress_find "$1"
  progress_span_said=""
  if [ "$progress_found" -ge 0 ] && progress_is_count "${progress_took[$progress_found]}"; then
    progress_span "${progress_took[$progress_found]}"
  fi
  return 0
}

# progress_end [<the run's exit status>]: the run has ended, however it ended. The step going now
# ends, as failed where the status is not 0, and the time of every step that passed is kept for the
# next run, beside the last time of every one that did not.
progress_end() {
  if [ -z "$progress_file" ]; then return 0; fi
  if [ "${1:-0}" != 0 ]; then progress_failed; fi
  progress_close
  local i=0 seconds count kept="$progress_file.$$"
  if : 2>/dev/null >"$kept"; then
    while [ "$i" -lt "${#progress_titles[@]}" ]; do
      if [ "${progress_passed[$i]}" = 1 ]; then
        seconds=${progress_took[$i]}
        count=${progress_counts[$i]}
      else
        seconds=${progress_last[$i]}
        count=${progress_last_counts[$i]}
      fi
      if progress_is_count "$seconds"; then
        printf '%s\t%s\t%s\n' "${progress_titles[$i]}" "$seconds" "$count" >>"$kept"
      fi
      i=$((i + 1))
    done
    mv -f "$kept" "$progress_file" 2>/dev/null || rm -f "$kept"
  fi
  rm -f "$progress_beat" "$progress_note_file"
  progress_file=""
  return 0
}

# progress_ticker_start [<log> <pattern> <what the pattern counts>]: a line at the bottom for as long
# as the step goes on or until `progress_ticker_stop`, saying how long the step has been going, how
# many lines of <log> match <pattern> of how many matched the last time, whatever `progress_note`
# said last, and how long the run has left. Short enough for a terminal eighty columns wide.
progress_ticker_start() {
  progress_ticker_stop
  local log=${1:-} pattern=${2:-} noun=${3:-} expected=""
  if [ "$progress_at" -ge 0 ]; then expected=${progress_last_counts[$progress_at]}; fi
  printf '%s' "$SECONDS" 2>/dev/null >"$progress_beat" || true
  (
    trap - EXIT
    progress_tick "$log" "$pattern" "$noun" "$expected"
  ) &
  progress_ticker=$!
  return 0
}

# The ticker itself, in a subshell of its own, until it is stopped or the run it belongs to is gone.
progress_tick() {
  local log=$1 pattern=$2 noun=$3 expected=$4
  local beat count note line terminal=0 width=80 sleeping=""
  # Asked of the terminal itself: inside `$(…)` the output of `tput` is a pipe, and it would answer
  # for that rather than for the window.
  if [ -t 1 ]; then
    terminal=1
    width=$(stty size 2>/dev/null </dev/tty | cut -d ' ' -f 2) || width=80
    # A terminal that has no size to give says 0.
    if ! progress_is_count "$width" || [ "$width" -lt 20 ]; then width=80; fi
  fi
  # Stopped while it sleeps, it takes its sleep with it: a sleep left running holds the output open,
  # and whatever reads that output would wait for it.
  trap 'if [ -n "$sleeping" ]; then kill "$sleeping" 2>/dev/null; fi; exit 0' TERM
  while kill -0 "$progress_parent" 2>/dev/null; do
    sleep "$progress_every" &
    sleeping=$!
    wait "$sleeping" 2>/dev/null || true
    sleeping=""

    beat=$SECONDS
    read -r beat 2>/dev/null <"$progress_beat" || true
    progress_is_count "$beat" || beat=$SECONDS
    if [ "$terminal" -eq 0 ] && [ $((SECONDS - beat)) -lt "$progress_quiet" ]; then continue; fi

    progress_span $((SECONDS - progress_began))
    line="  … still going, $progress_span_said"
    if [ -n "$pattern" ]; then
      count=$(grep -cE "$pattern" "$log" 2>/dev/null || true)
      progress_is_count "$count" || count=0
      if [ -n "$expected" ]; then
        line="$line · $count of $expected $noun"
      else
        line="$line · $count $noun so far"
      fi
    fi
    note=""
    read -r note 2>/dev/null <"$progress_note_file" || true
    if [ -n "$note" ]; then line="$line · $note"; fi
    progress_where
    if [ -n "$progress_to_go" ]; then line="$line · $progress_to_go"; fi

    if [ "$terminal" -eq 1 ]; then
      printf '\r\033[2m%s\033[0m\033[K' "${line:0:$((width - 1))}"
    else
      printf '\033[2m%s\033[0m\n' "$line"
      printf '%s' "$SECONDS" 2>/dev/null >"$progress_beat" || true
    fi
  done
  return 0
}

# The bottom line goes, and the step's own output carries on from the start of a clean line.
progress_ticker_stop() {
  if [ -n "$progress_ticker" ]; then
    kill "$progress_ticker" 2>/dev/null || true
    wait "$progress_ticker" 2>/dev/null || true
    progress_ticker=""
    if [ -t 1 ]; then printf '\r\033[K'; fi
  fi
  return 0
}

# Passes its input through a line at a time, noting when each went past, so the bottom line is
# printed only into a quiet stretch. On a terminal each line first clears the bottom line away.
progress_relay() {
  local line terminal=0
  if [ -t 1 ]; then terminal=1; fi
  while IFS= read -r line || [ -n "$line" ]; do
    if [ "$terminal" -eq 1 ]; then
      printf '\r\033[K%s\n' "$line"
    else
      printf '%s\n' "$line"
    fi
    printf '%s' "$SECONDS" 2>/dev/null >"$progress_beat" || true
  done
  return 0
}
