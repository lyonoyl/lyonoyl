#!/usr/bin/env bash
#
# Everything that must be true before a release leaves this machine.
#
# The point of this script is the mail server. Every test that asks a real mail server what it did
# — including the two that prove this application removes the message it named and nothing else,
# and the four that send through it — *skips itself and passes* when nothing answers. So `cargo
# test` on a machine with no server is green, and green about mail it never touched. This refuses
# to be that.
#
# `LYONOYL_MAIL_SERVER_REQUIRED=1` below is how it refuses. Every one of those tests reads it, and
# where it is set a server that is not answering fails the run at the test that was about to skip.
#
# Run before building anything anyone else will install. `npm run app:release` runs it, with
# `--if-changed`.
#
# It starts with what this machine has, `scripts/check-machine.mjs`, and stops where the run needs
# something it lacks. It ends with every step passed or failed and how long each took, and under a
# failed test step every failed test with where it stopped and what it said. What the tests printed
# stays in `target/preflight/`.
#
# Every step says which it is of how many and how long the run has left, from what each step took the
# last time it passed on this machine, which `scripts/progress.sh` keeps. A step whose output goes to
# a log keeps a line at the bottom saying it is still going, and the gate's self-test how many of its
# probes it has asked so far.
#
# A run that passes writes down which tree it passed, in `target/preflight/passed`, as
# `scripts/tree-fingerprint.mjs` takes a tree: every file git would carry, as it stands, and what npm
# installed. With `--if-changed`, a tree that is still that one, byte for byte, is not checked again.
# Only what changes while a tree stands still is asked again, the advisories, together with what a
# build needs of this machine. Every other tree is checked whole, and so is every tree without
# `--if-changed`. A whole run takes the record away before its first step, so a run that fails or is
# stopped leaves no pass behind.

set -euo pipefail

cd "$(dirname "$0")/.."

# shellcheck source=scripts/progress.sh
. scripts/progress.sh

case "${1:-}" in
  "") if_changed=0 ;;
  --if-changed) if_changed=1 ;;
  *)
    printf '\033[31m%s\033[0m\n' "Not something this run knows: $1. It takes nothing, or --if-changed." >&2
    exit 1
    ;;
esac

fail=0
# Each step as "<1 where it failed, else 0><TAB><name>"; each failure and each test summary as
# "<step><TAB><text>". Bash 3.2, which macOS ships, has no associative arrays.
steps=()
failures=()
summaries=()
current=""
failed_here=0
closed() {
  if [ -n "$current" ]; then steps+=("$failed_here"$'\t'"$current"); fi
  current=""
  progress_close
}
step() {
  closed
  current=$1
  failed_here=0
  progress_step "$1"
}
bad() {
  printf '\033[31m%s\033[0m\n' "$1" >&2
  fail=1
  failed_here=1
  failures+=("$current"$'\t'"$1")
  progress_failed
}
summary() { summaries+=("$current"$'\t'"$1"); }
stopped() {
  progress_failed
  printf '\n\033[31mPreflight stopped: %s\033[0m\n' "$1" >&2
  exit 1
}

# Runs a command whose output goes to a log, with the line at the bottom saying it is still going.
quietly() {
  local log=$1 status=0
  shift
  progress_ticker_start
  "$@" >"$log" 2>&1 || status=$?
  progress_ticker_stop
  return "$status"
}

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

# One run at a time. The gate's self-test plants violations in this tree for the gate to refuse, and
# a second run's gate refuses them as if they were code: two runs at once fail each other. A run
# killed outright leaves its lock behind with a process that is gone, and the next run takes it.
lock=target/preflight.lock
mkdir -p target
if ! mkdir "$lock" 2>/dev/null; then
  holder=$(cat "$lock/pid" 2>/dev/null || true)
  if [ -n "$holder" ] && kill -0 "$holder" 2>/dev/null; then
    stopped "another run is going on in this tree (process $holder): wait for it to end, or stop it"
  fi
  rm -rf "$lock"
  mkdir "$lock"
fi
echo $$ >"$lock/pid"
trap 'progress_end $?; rm -rf "$scratch" "$lock"' EXIT
mkdir -p target/preflight

# Awake for as long as this runs. A Mac that sleeps in the middle of a run suspends it, and a test
# that waits a set time for something to happen fails on waking: the time passed while nothing ran.
# `caffeinate` holds sleep off until this process ends, however it ends. It is part of macOS; the
# other two systems run this without it.
if [ "$(uname -s)" = Darwin ]; then
  caffeinate -i -s -w $$ &
fi

# What a run killed outright planted is still here, and the gate would refuse it as code. Every one
# of those files is named `__gate_probe`, and the self-test stages one of them, so the index is
# swept as well as the tree: a probe left staged would go out with the next commit.
while IFS= read -r -d '' left; do
  git rm --cached --quiet --ignore-unmatch -- "$left" >/dev/null 2>&1 || true
done < <(git ls-files -z --cached -- '*__gate_probe*')
while IFS= read -r -d '' left; do
  rm -f -- "$left"
done < <(find . \( -path ./node_modules -o -path ./target -o -path ./.git \) -prune -o -name '__gate_probe*' -print0)

# Runs a command for at most a number of seconds, and fails with 124 when it had not finished.
#
# macOS has no `timeout`, so the command runs in the background and is stopped from here. Polled
# every tenth of a second rather than slept through, so a command that answers at once costs no
# more than it takes.
within() {
  local seconds=$1
  shift
  "$@" &
  local running=$!
  local tenths=0
  while kill -0 "$running" 2>/dev/null; do
    if [ "$tenths" -ge $((seconds * 10)) ]; then
      kill "$running" 2>/dev/null || true
      wait "$running" 2>/dev/null || true
      return 124
    fi
    sleep 0.1
    tenths=$((tenths + 1))
  done
  wait "$running"
}

# What changes while a tree stands still: an advisory published against something already here.
# Nothing in the tree changed and the answer still changes, which is the whole reason to ask again
# rather than to ask once, and why it is asked again of a tree that passed. cargo-audit is there: the
# first step of a whole run stops without it, and only a whole run passes a tree.
advisories() {
  # `--deny yanked` as well as the advisories: a crate pulled from the registry is one nobody can
  # build against again, and it fails the run rather than being mentioned in passing.
  cargo audit --deny yanked || bad "cargo audit"
  npm audit --audit-level=high || bad "npm audit"
}

# Every step, and under each that failed, what failed and what its summary says, and then the
# verdict, with <what a pass says> under it. The run ends here, either way.
the_end() {
  closed
  printf '\n\033[1mSummary\033[0m\n'
  local failed_steps=0 entry name each
  for entry in ${steps[@]+"${steps[@]}"}; do
    name=${entry#*$'\t'}
    progress_time_of "$name"
    if [ "${entry%%$'\t'*}" = 0 ]; then
      printf '  \033[32m✓\033[0m %s  \033[2m%s\033[0m\n' "$name" "$progress_span_said"
      continue
    fi
    failed_steps=$((failed_steps + 1))
    printf '  \033[31m✗ %s\033[0m  \033[2m%s\033[0m\n' "$name" "$progress_span_said"
    for each in ${failures[@]+"${failures[@]}"}; do
      if [ "${each%%$'\t'*}" = "$name" ]; then printf '      %s\n' "${each#*$'\t'}"; fi
    done
    for each in ${summaries[@]+"${summaries[@]}"}; do
      if [ "${each%%$'\t'*}" = "$name" ]; then printf '%s\n' "${each#*$'\t'}" | sed 's/^/      /'; fi
    done
  done

  progress_span $((SECONDS - progress_run_began))
  if [ "$fail" -ne 0 ]; then
    printf '\n\033[31mPreflight failed at %s of %s steps, in %s. Nothing above may be waved through for a release.\033[0m\n' \
      "$failed_steps" "${#steps[@]}" "$progress_span_said" >&2
    exit 1
  fi
  printf '\n\033[32mPreflight passed in %s.\033[0m\n' "$progress_span_said"
  if [ -n "$1" ]; then printf '%s\n' "$1"; fi
  exit 0
}

# Which tree this run checks, taken before its first step: a pass is written down for this tree and
# no other, and only where it is still this tree when the run ends.
if ! command -v node >/dev/null 2>&1; then
  stopped "Node.js is not installed, and this run needs it: an LTS release from https://nodejs.org, then run this again."
fi
PASSED=target/preflight/passed
tree=$(node scripts/tree-fingerprint.mjs 2>"$scratch/tree.log") || tree=""

if [ "$if_changed" -eq 1 ] && ago=$(node scripts/tree-fingerprint.mjs --passed "$PASSED" 2>/dev/null); then
  progress_span "$ago"
  since=$progress_span_said
  progress_begin preflight-passed-before Step "This machine
Advisories"
  printf '\n%s\n' "This tree passed every step $since ago, and not a byte of it has changed since, so only what changes while a tree stands still is asked again: the advisories, and what a build needs of this machine."

  step "This machine"
  node scripts/check-machine.mjs --build || stopped "this machine is not ready for a build. The list above says what stands in the way, and what to do."

  step "Advisories"
  advisories

  the_end "Every other step passed on this tree $since ago, and nothing in it has changed since."
fi

# A pass recorded earlier is of another tree, or of this one by another run: either way it says
# nothing about this run, and it goes before the first step.
rm -f "$PASSED"
# The whole run's steps are the lines of this file that begin with `step`, in the order they run, so
# the count cannot drift from what runs.
progress_begin preflight Step "$(grep -E '^step "' scripts/preflight.sh | cut -d '"' -f 2)"

step "This machine"
# Everything the steps below call, asked at once, each missing thing with how to get it: the
# compiler the tree pins, rustup's own `rustc` first on PATH, the tools, and on macOS whether closed
# connections are still cleared. Nothing below is worth running without them.
node scripts/check-machine.mjs || stopped "this machine is not ready for the run. The list above says what stands in the way, and what to do."

step "The development mail server"

# Neither question has a bound of its own. A Docker that is still starting takes a question and
# keeps it, and `--wait` with no timeout waits for as long as that takes: hours, with nothing on
# the screen to say what it is waiting for.
DOCKER_ANSWERS_WITHIN=20
HEALTHY_WITHIN=180
COMPOSE=(docker compose -f docker/mail-server/compose.yml)
CONTAINERS=(lyonoyl-mail-server lyonoyl-mail-submission)

echo "asking Docker whether it is running, for up to ${DOCKER_ANSWERS_WITHIN} s"
if within "$DOCKER_ANSWERS_WITHIN" docker info >"$scratch/docker-info" 2>&1; then
  echo "Docker is running"
else
  answered=$?
  if [ "$answered" -eq 124 ]; then
    bad "Docker did not answer 'docker info' within ${DOCKER_ANSWERS_WITHIN} s"
  # A Docker that is running and refuses this user — on Linux, a user outside the docker group —
  # fails the same question with the same status. Starting Docker does nothing for that, so the
  # advice has to come from what it said.
  elif grep -qi 'permission denied' "$scratch/docker-info"; then
    bad "Docker refused this user; 'docker info' says:"
    tail -n 3 "$scratch/docker-info" | sed 's/^/  /' >&2
    bad "Let this user reach Docker (on Linux, add it to the docker group and sign in again),"
    bad "and run this again."
    stopped "Docker refused this user."
  else
    bad "Docker is not running; 'docker info' says:"
    tail -n 3 "$scratch/docker-info" | sed 's/^/  /' >&2
  fi
  bad "Start Docker (Docker Desktop on macOS and Windows, the docker service on Linux),"
  bad "wait until it says it is running, and run this again."
  stopped "Docker is not answering."
fi

# A Compose too old to bound the wait is not asked to wait at all: it starts the containers and
# returns, and the two ports below are waited for with a bound of their own.
if "${COMPOSE[@]}" up --help 2>/dev/null | grep -q -- '--wait-timeout'; then
  UP=("${COMPOSE[@]}" up -d --wait --wait-timeout "$HEALTHY_WITHIN")
  echo "starting it and waiting up to ${HEALTHY_WITHIN} s for both containers to report healthy"
else
  UP=("${COMPOSE[@]}" up -d)
  echo "starting it; this Compose cannot bound a wait, so the two ports below are waited for"
fi
echo "(where Docker has no images for it, it builds them first, outside that wait)"
echo "\$ ${UP[*]}"

if ! "${UP[@]}"; then
  bad "'${UP[*]}' failed. What Docker says about the two containers:"
  within 20 "${COMPOSE[@]}" ps --all 2>&1 | sed 's/^/  /' >&2 || true
  for container in "${CONTAINERS[@]}"; do
    printf '\n  the last lines %s logged:\n' "$container" >&2
    within 20 docker logs --tail 15 "$container" 2>&1 | sed 's/^/    /' >&2 || true
  done
  stopped "the mail server did not start."
fi

# Reading and sending are two ports, and a server answering on one of them is half a server: the
# tests that send read what arrived, so they need both.
missing=0
for port in 1143 1587; do
  answering=0
  for _ in $(seq 1 60); do
    # Each try is bounded: a connection held on the way to the server never completes on its own.
    if within 2 bash -c "exec 3<>/dev/tcp/127.0.0.1/$port" 2>/dev/null; then answering=1; break; fi
    sleep 1
  done

  if [ "$answering" -eq 1 ]; then
    echo "answering on 127.0.0.1:$port"
  else
    bad "nothing is answering on 127.0.0.1:$port"
    missing=1
  fi
done

if [ "$missing" -eq 0 ]; then
  echo "the tests that ask a real server will really ask one"

  # The tests below do not need the development accounts filled: a test that reads mail fills a
  # mailbox of its own. A person opening the application against a server started on an empty
  # volume does need them, and that server has nothing to lose — so it is filled, and a server
  # where they hold mail is not.
  ./scripts/reset-mail-server.sh --if-fresh \
    || bad "scripts/reset-mail-server.sh --if-fresh could not fill the development accounts"
else
  bad "without both, the tests that ask a real server skip themselves and pass, and this release"
  bad "would be untested where it matters most: the paths that change mail on somebody's server"
  bad "and send it to somebody else"
fi

# Nothing below is worth running if the tests above would be a lie.
if [ "$fail" -ne 0 ]; then
  stopped "the mail server is not ready."
fi

step "Formatting"
cargo fmt --all --check || bad "cargo fmt --all --check"

step "Clippy"
cargo clippy --workspace --all-targets -- -D warnings || bad "cargo clippy"
# The build made for screenshots, which nothing else here compiles: code behind its feature is
# otherwise found broken on the day somebody needs a screenshot.
cargo clippy -p lyonoyl --features screenshots -- -D warnings || bad "cargo clippy --features screenshots"

step "Rust tests"
# Past a test program that failed, so every failed test is named, whichever program it is in.
rust_tests=target/preflight/rust-tests.log
if ! LYONOYL_MAIL_SERVER_REQUIRED=1 cargo test --workspace --no-fail-fast 2>&1 | tee "$rust_tests"; then
  bad "cargo test --workspace"
  summary "$(node scripts/failed-tests.mjs rust "$rust_tests")"
fi

step "The release profile compiles"
cargo check --workspace --release --all-targets || bad "cargo check --release"

step "Layer boundaries, forbidden APIs, CSP"
./scripts/check-layering.sh || bad "scripts/check-layering.sh"
# The self-test runs the gate again for every few probes, which takes minutes, and writes what each
# answered into a log: the line at the bottom counts the answers so far.
answered='(refused|allowed|LET THROUGH|REFUSED WRONGLY): '
progress_ticker_start "$scratch/self-test.log" "$answered" probes
self_test=0
./scripts/check-layering.test.sh >"$scratch/self-test.log" 2>&1 || self_test=$?
progress_ticker_stop
progress_counted "$(grep -cE "$answered" "$scratch/self-test.log" || true)"
if [ "$self_test" -ne 0 ]; then
  bad "the gate let a forbidden thing through — scripts/check-layering.test.sh"
  summary "$(grep -E 'LET THROUGH|REFUSED WRONGLY' "$scratch/self-test.log" || tail -n 15 "$scratch/self-test.log")"
fi

step "Frontend"
npm run typecheck || bad "npm run typecheck"
frontend_tests=target/preflight/frontend-tests.json
rm -f "$frontend_tests"
if ! npm test -- --reporter=default --reporter=json --outputFile.json="$frontend_tests"; then
  bad "npm test"
  summary "$(node scripts/failed-tests.mjs frontend "$frontend_tests")"
fi

step "Generated TypeScript bindings"
bindings=target/preflight/bindings.log
if ! quietly "$bindings" cargo test -p lyonoyl export_bindings; then
  bad "generating bindings"
  summary "$(node scripts/failed-tests.mjs rust "$bindings")"
fi
git diff --exit-code src/shared/ipc/generated >/dev/null || bad "bindings drifted; commit them"

# What ships with the application and under what terms. Section 6 of AGENTS.md refuses GPL, LGPL,
# AGPL, SSPL, BUSL and non-commercial terms outright, and a dependency arrives with its licence
# rather than announcing it. The list is rebuilt from the tree and compared with the committed one:
# a difference is either a notice that would ship missing, or a licence nobody agreed to.
step "Licences"
LICENCES=(
  src-tauri/licences-npm.json
  src-tauri/licences-rust.json
  src-tauri/licences-vendored.json
  src-tauri/licences.json
)
# What went wrong, said out loud. Swallowed, the output would report every failure -- a network
# that is down, a manifest the collector will not parse, a crate with no licence file -- as a
# missing tool, and throw away the one line that says which.
if collecting=$(./scripts/collect-licences.sh 2>&1); then
  git diff --exit-code -- "${LICENCES[@]}" >/dev/null \
    || bad "the licence list is not the one this tree produces; read the diff and commit it"
  # The comparison above reruns the same collector, so a package the collector cannot see is missing
  # from both sides alike and the two agree. The window is built and read instead: every package in
  # it has a notice, or the run fails.
  node scripts/check-shipped-licences.mjs || bad "a package ships in the window without its notice"
else
  printf '%s\n' "$collecting" >&2
  bad "collecting licences"
fi

step "Advisories"
advisories

# The bundle the application ships, built rather than only type-checked. `tsc --noEmit` says the
# types agree; it says nothing about an import that resolves for the checker and not for the
# bundler, or an asset that is not where the build expects it.
step "The frontend bundle"
if ! quietly "$scratch/bundle.log" npm run build; then
  bad "npm run build"
  summary "$(tail -n 20 "$scratch/bundle.log")"
fi

# The application, built the way `npm run app:build` builds it, so a run that passes here is a build
# that builds. `cargo check` above says the code compiles; only a build links it, runs the frontend
# build the bundle carries, and packs the icons and resources a `.app` needs. On macOS that is the
# `.app`; elsewhere the binary, since a bundle there is packaging for whoever builds it.
step "The application"
case "$(uname -s)" in
  Darwin) build_args=(--bundles app) ;;
  *) build_args=(--no-bundle) ;;
esac
if quietly "$scratch/application.log" npx tauri build "${build_args[@]}"; then
  echo "built with npx tauri build ${build_args[*]}"
else
  tail -40 "$scratch/application.log"
  bad "npx tauri build ${build_args[*]}"
fi

# A pass is written down for the tree the run began with, and only while it is still that tree.
passed_said=""
if [ "$fail" -eq 0 ]; then
  if [ -z "$tree" ]; then
    passed_said="The pass is not recorded: $(head -n 1 "$scratch/tree.log")"
  elif node scripts/tree-fingerprint.mjs --record "$PASSED" "$tree"; then
    passed_said="Recorded as passed: while this tree stays as it is, npm run app:release asks only the advisories and this machine again."
  else
    passed_said="The tree changed while this ran, so the pass is not recorded, and npm run app:release checks it whole."
  fi
fi
the_end "$passed_said"
