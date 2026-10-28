#!/usr/bin/env bash
#
# The gate's own test. A gate is worth exactly what it stops, and nothing about a passing gate says
# whether it would stop anything. A rule that greps for one spelling is passed by the next: an API
# that ships with Tauri under another name, a fully-qualified call that needs no `use` line, a style
# object that writes `cursor: "pointer"` with quotes. Each probe below is one way past a rule,
# planted to show that the rule refuses it.
#
# **A rule arrives here with a probe, and that is what makes it a rule rather than a grep.** What a
# probe can show is a file written into the tree; what it cannot is a rule that only fires when a
# file already in the tree is edited -- the domain crate taking a dependency, a translation library
# in `package.json`, `panic = "abort"` in the workspace manifest, a guard that stops guarding, the
# ledger of exempted files, the shop window held to section 7. Those are read rather than planted.
#
# The count is not the claim, and a sentence saying "all of them" would be one nobody could check:
# several probes assert on part of a rule's message and several are built rather than written out,
# so no tally here would stay true. What is asked of a change is simpler -- a rule added without a
# probe is a rule nobody has seen refuse anything. The bundle configuration would be one of them, and is not, because
# the gate reads it through `LYONOYL_BUNDLE_CONFIG` and the probe hands it a copy -- a weakened
# content policy must not sit in the tree even for the length of a run.
#
# Each probe is written into a real directory of the tree, the gate is run, and the probe is
# removed. A trap puts everything back if this is interrupted.

set -uo pipefail

cd "$(dirname "$0")/.."

PROBE_TSX="src/features/message-reader/__gate_probe.tsx"
PROBE_JSX="src/features/message-reader/__gate_probe.jsx"
PROBE_RS="src-tauri/src/infrastructure/__gate_probe.rs"
PROBE_IMAP="src-tauri/src/infrastructure/imap/__gate_probe.rs"
PROBE_PS1="scripts/__gate_probe.ps1"
PROBE_CMD="scripts/__gate_probe.cmd"
PROBE_MD="docs/__gate_probe.md"
PROBE_EML="tests/fixtures/eml/__gate_probe.eml"
PROBE_NAMED="docs/__gate_probe_q33@nothing.zz.md"
PROBE_LINK="docs/__gate_probe_link"
PROBE_STAGED="docs/__gate_probe_staged.md"
PROBE_MD_TWO="docs/__gate_probe_two.md"
PROBE_BIN="docs/__gate_probe.bin"
PROBE_PNG="docs/__gate_probe.png"
PROBE_APP="src-tauri/src/application/__gate_probe.rs"
PROBE_COMMAND="src-tauri/src/commands/__gate_probe.rs"
PROBE_PORT="src-tauri/src/ports/__gate_probe.rs"
PROBE_READ="src-tauri/src/read_model/__gate_probe.rs"
PROBE_CMD_RS="src-tauri/src/commands/__gate_probe.rs"
PROBE_CAP="src-tauri/capabilities/__gate_probe.json"
PROBE_DOMAIN="crates/lyonoyl-domain/src/__gate_probe.rs"
PROBE_DIR="src/__gate_probe"
PROBE_SCENE_DIR="mocks/screenshots/__gate_probe"
PROBE_SCENE="$PROBE_SCENE_DIR/scene.json"
PROBE_SCENE_HTML="$PROBE_SCENE_DIR/letter.html"
PROBE_SCENE_DEEP="$PROBE_SCENE_DIR/deeper/scene.json"
# Two the gate reads through `git ls-files`, so a probe has to be staged to exist for it. Neither
# touches a manifest that is really in the tree: the identity is an empty file under a name, and the
# manifest naming an author is one of its own.
PROBE_KEY="__gate_probe.p12"
PROBE_MANIFEST="__gate_probe_crate/Cargo.toml"
# A copy, and the gate is told to read it. Editing the bundle configuration in place and putting it
# back afterwards leaves a weakened content policy on disk for as long as a run lasts -- and for
# good when two runs overlap or one is killed outright. Anybody glancing at their editor in between
# sees what reads as a security regression sitting in the tree.
#
# The probes above are still planted in the tree, because the checks that catch them read what git
# tracks and what is about to be tracked. A file is not a policy: one is a thing the gate must find,
# the other is a thing that must not be loosened even for a second.
# With the X's, and without `-t`: GNU coreutils refuses a `-t` template that carries none, and with
# no `set -e` above, the gate on Linux would carry on with an empty path and report the security
# gate as broken.
CONFIG="$(mktemp "${TMPDIR:-/tmp}/lyonoyl-bundle-config.XXXXXX")"
export LYONOYL_BUNDLE_CONFIG="$CONFIG"

cp src-tauri/tauri.conf.json "$CONFIG"
# A probe staged for the gate to read through git. Something else touching the index at that moment —
# an editor refreshing what it shows — makes `git add` fail on the lock, and a probe that never
# reached the index is a probe the gate cannot see; so the add is tried until git lists the file.
stage() {            # stage <path> [-f]
  local tries=0
  until git ls-files --error-unmatch "$1" >/dev/null 2>&1; do
    git add ${2:-} "$1" >/dev/null 2>&1 || true
    tries=$((tries + 1))
    if [ "$tries" -ge 20 ]; then
      printf '\033[31m  NOT STAGED: %s\033[0m\n' "$1"
      missed=1
      return
    fi
    sleep 0.25
  done
}

# Every probe, wherever it was planted, staged or not.
sweep_probes() {
  rm -f "$PROBE_TSX" "$PROBE_JSX" "$PROBE_RS" "$PROBE_PS1" "$PROBE_CMD" "$PROBE_MD" "$PROBE_EML"
  rm -f "$PROBE_NAMED" "$PROBE_LINK" "$PROBE_MD_TWO" "$PROBE_BIN" "$PROBE_PNG"
  git rm --cached -q -f --ignore-unmatch "$PROBE_STAGED" >/dev/null 2>&1
  rm -f "$PROBE_STAGED"
  rm -f "$PROBE_APP" "$PROBE_PORT" "$PROBE_READ" "$PROBE_CMD_RS" "$PROBE_CAP" "$PROBE_DOMAIN"
  rm -f "$PROBE_IMAP"
  rmdir "$PROBE_DIR" 2>/dev/null
  rm -f "$PROBE_SCENE" "$PROBE_SCENE_HTML" "$PROBE_SCENE_DEEP"
  rmdir "$(dirname "$PROBE_SCENE_DEEP")" "$PROBE_SCENE_DIR" 2>/dev/null
  git rm --cached -q --ignore-unmatch "$PROBE_KEY" "$PROBE_MANIFEST" >/dev/null 2>&1
  rm -rf "$PROBE_KEY" "$(dirname "$PROBE_MANIFEST")"
}

clean() {
  sweep_probes
  rm -f "$CONFIG"
}
# Interrupted, and killed outright. The first two are trapped; the third cannot be, so the probes are
# swept on the way in as well -- one left by a run nobody finished is one the next run removes,
# rather than one somebody finds in their editor and wonders about. The copy of the bundle
# configuration is not swept here, because it was made a moment ago and is this run's.
sweep_probes
trap clean EXIT INT TERM

missed=0

# Runs the gate and requires it to say something about this violation.
refuses() {          # refuses <what the message should mention> <what was tried>
  # The gate's output is taken first and searched after. Piped into `grep` under `pipefail`, the
  # gate's own non-zero exit — which is what refusing looks like — becomes the pipeline's status,
  # and a caught violation reads as a missed one.
  local said
  said="$(bash scripts/check-layering.sh 2>&1)"

  if printf '%s' "$said" | grep -qF "$1"; then
    printf '  refused: %s\n' "$2"
  else
    printf '\033[31m  LET THROUGH: %s\033[0m\n' "$2"
    missed=1
  fi
}

# Several probes at once, and the gate asked once. A probe is a file and the gate reports every
# violation it finds, so planting six and reading six answers out of one run says exactly what six
# runs would -- and a run takes seconds, so asking once for every probe takes minutes.
#
# Only for probes that cannot mask one another: each is its own file or its own line, and each is
# asserted by text only it produces. An `allows` probe is never batched, because it asserts on the
# absence of any mention and a neighbour's violation would be one.
refuses_each() {     # refuses_each <what the message should mention>...
  local said
  said="$(bash scripts/check-layering.sh 2>&1)"

  local want
  for want in "$@"; do
    if printf '%s' "$said" | grep -qF "$want"; then
      printf '  refused: %s\n' "$want"
    else
      printf '\033[31m  LET THROUGH: %s\033[0m\n' "$want"
      missed=1
    fi
  done
}

# The other half of a gate: what it must NOT refuse. A rule that catches everything catches nothing,
# because the first false refusal is the day somebody adds an exemption and stops reading it.
allows() {           # allows <what was tried>
  local said
  said="$(bash scripts/check-layering.sh 2>&1)"

  if printf '%s' "$said" | grep -q "__gate_probe"; then
    printf '\033[31m  REFUSED WRONGLY: %s\033[0m\n' "$1"
    printf '%s\n' "$said" | grep "__gate_probe" | sed 's/^/    /'
    missed=1
  else
    printf '  allowed: %s\n' "$1"
  fi
}

printf '\nA use case reaching for an adapter, in each shape that reaches one (section 1)\n'
# Everything below is a use case holding an adapter, and a rule reading only `use` at the start of
# a line lets each one through: a fully-qualified call needs no import at all, so the rule that
# blocks the import blocks nothing.
# All six in one file, one line each. The gate quotes the line it refuses, so each is told apart by
# its own text rather than by the message, which is the same for all of them.
{
  echo 'use crate::infrastructure::sqlite::Store;'
  echo 'pub(crate) use crate::infrastructure::sqlite::Store;'
  echo 'use crate::{ports::body::Viewing, infrastructure::html::Sanitizer};'
  echo 'pub fn probe() {'
  echo '    let raw = crate::infrastructure::smtp::written("id", &message);'
  echo '    let _ = super::infrastructure::html::Sanitizer;'
  echo '    let _ = tauri::AppHandle::default();'
  echo '}'
} > "$PROBE_APP"
refuses_each \
  'use crate::infrastructure::sqlite::Store;' \
  'pub(crate) use crate::infrastructure::sqlite::Store;' \
  'use crate::{ports::body::Viewing, infrastructure::html::Sanitizer};' \
  'crate::infrastructure::smtp::written' \
  'super::infrastructure::html::Sanitizer' \
  'tauri::AppHandle::default()'

# An adapter that never appears as a word of its own: a type in a signature is the same dependency.
printf 'pub fn probe() -> Result<(), rusqlite::Error> {\n    Ok(())\n}\n' > "$PROBE_APP"
refuses "adapter type in an application signature" "rusqlite::Error in a return type"

# And what it must not refuse: the comments are the design record, and a comment naming the adapter
# a use case may not hold is how that record is written.
printf '// crate::infrastructure::smtp is where writing a message belongs, not here.\npub fn probe() {}\n' > "$PROBE_APP"
allows "a comment naming crate::infrastructure"
rm -f "$PROBE_APP"

printf '\nA command building an adapter rather than taking one from bootstrap (section 1)\n'
# Both shapes in one file, one line each: a type imported from `infrastructure` and then built, and
# one written out in full and built where it stands.
{
  echo 'use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;'
  echo 'pub fn probe(pool: Pool) {'
  echo '    let _ = SqliteAccountRepository::new(pool);'
  echo '    let _ = crate::infrastructure::net::TransportPolicy::new();'
  echo '}'
} > "$PROBE_COMMAND"
refuses_each \
  'SqliteAccountRepository::new(pool);' \
  'crate::infrastructure::net::TransportPolicy::new();'

# And what it must not refuse: an adapter bootstrap hands over, and a test module building the one
# it tests.
{
  echo 'pub fn probe(pool: Pool) {'
  echo '    let _ = crate::bootstrap::adapters::accounts(pool);'
  echo '}'
  echo ''
  echo '#[cfg(test)]'
  echo 'mod tests {'
  echo '    use crate::infrastructure::secure_storage::in_memory::InMemory;'
  echo ''
  echo '    fn built() {'
  echo '        let _ = InMemory::new();'
  echo '    }'
  echo '}'
} > "$PROBE_COMMAND"
allows "an adapter bootstrap hands over, and one a test module builds"
rm -f "$PROBE_COMMAND"

printf '\nThe domain reaching outside itself (section 1)\n'
printf 'use rusqlite::Connection;\n\npub fn probe(_: &Connection) {}\n' > "$PROBE_DOMAIN"
refuses "domain imports the outside world" "rusqlite in the domain crate"
rm -f "$PROBE_DOMAIN"

printf '\nA directory the frontend map does not have (section 1)\n'
mkdir -p "$PROBE_DIR"
refuses "directory outside the map" "a fifth directory under src/"
rmdir "$PROBE_DIR"

printf '\nA port that speaks an adapter'"'"'s language (section 1)\n'
printf 'pub trait Probe {\n    fn go(&self) -> Result<(), rusqlite::Error>;\n}\n' > "$PROBE_PORT"
refuses "port depends on an adapter" "rusqlite::Error in a port signature"
rm -f "$PROBE_PORT"

printf '\nA read model that writes (section 1)\n'
printf 'pub const PROBE: &str = "DELETE FROM messages WHERE id = ?1";\n' > "$PROBE_READ"
refuses "read_model performs a write" "DELETE in the read model"

# Rust is not SQL: `impl Drop` is a destructor, and refusing it would be the first false refusal.
printf 'pub struct Probe;\n\nimpl Drop for Probe {\n    fn drop(&mut self) {}\n}\n' > "$PROBE_READ"
allows "impl Drop in the read model"
rm -f "$PROBE_READ"

printf '\nOne feature reaching into another (section 1)\n'
printf 'import { thing } from "@/features/settings/thing";\n\nexport const probe = () => thing;\n' > "$PROBE_TSX"
refuses "one feature imports another" "message-reader importing from settings"
rm -f "$PROBE_TSX"

printf '\nThe window showing a sentence Rust wrote, or one nobody translated\n'
{
  echo 'import en from "@/shared/i18n/locales/en.json";'
  echo 'export const catalogue = en;'
  echo 'export const A = (err: Error) => <p>{err.message}</p>;'
  echo 'export const B = () => <p>Get new mail</p>;'
  echo 'export const C = () => <input placeholder="Search your mail" />;'
} > "$PROBE_TSX"
refuses_each \
  'error text shown instead of a catalog key' \
  'Get new mail' \
  'Search your mail' \
  'locale catalog imported outside'
rm -f "$PROBE_TSX"

printf '\nThe hand cursor, which says pressable to a mouse and to nobody else (invariant 19)\n'
# Four ways to ask for it, told apart by the line each is written on. The middle two are what a
# rule counting characters between the two words gets wrong in both directions.
{
  echo 'export const A = () => <button className="cursor-pointer" />;'
  echo 'export const B = () => <button style={{ cursor: "pointer" }} />;'
  echo 'export const C = ({ on }: { on: boolean }) => <button style={{ cursor: on ? "pointer" : "default" }} />;'
  echo 'export const D = (el: HTMLElement) => el.style.setProperty("cursor", "pointer");'
} > "$PROBE_TSX"
refuses_each \
  'className="cursor-pointer"' \
  'style={{ cursor: "pointer" }}' \
  'cursor: on ? "pointer" : "default"' \
  'setProperty("cursor", "pointer")'

# And the three it must not refuse. A comment is where the rule itself is written down; the other
# two set no cursor at all, and refusing them is how an exemption gets added and stops being read.
printf '// cursor: pointer is refused here; see invariant 19.\nexport const probe = 1;\n' > "$PROBE_TSX"
allows "a comment naming the hand cursor"

printf 'export const probe = 1; // cursor: pointer is refused here\n' > "$PROBE_TSX"
allows "the same, written at the end of a line"

printf 'const cursor = { pointer: 1 };\nexport const probe = cursor.pointer;\n' > "$PROBE_TSX"
allows "a thing named cursor with a field named pointer"
rm -f "$PROBE_TSX"

printf '\nHostile markup reaching the window that holds IPC (invariant 8)\n'
{
  echo 'import DOMPurify from "dompurify";'
  echo 'export const Probe = ({ h }: { h: string }) => <p dangerouslySetInnerHTML={{ __html: h }} />;'
  echo 'export const cleaned = DOMPurify;'
} > "$PROBE_TSX"
refuses_each \
  'dangerouslySetInnerHTML' \
  'an HTML sanitizer in the frontend'
rm -f "$PROBE_TSX"

printf '\nThe window reaching past the one door it has\n'
{
  echo 'import { invoke } from "@tauri-apps/api/core";'
  echo 'import { sendNotification } from "@tauri-apps/plugin-notification";'
  echo 'export const probe = [invoke, sendNotification];'
} > "$PROBE_TSX"
refuses_each \
  '@tauri-apps outside src/shared/ipc' \
  'the window raising its own notification'
rm -f "$PROBE_TSX"

printf '{\n  "permissions": ["opener:default"]\n}\n' > "$PROBE_CAP"
refuses "a way out of the application it does not use" "an opener permission granted to the window"
rm -f "$PROBE_CAP"

printf '\nSomething the core must not do to a person'"'"'s mail or their secret\n'
# Split before it is written, not after. Planting the whole line would put the expected message
# into the file as well -- `let _ = connection.transaction();@deferred transaction` -- so the probe
# would not be the code it claims to be, and a run interrupted mid-way would leave that in the tree
# for somebody to find in their editor and wonder about.
# Eight rules, eight distinct messages, one run. Each line trips a rule the others do not.
{
  echo 'pub fn probe() {'
  while IFS= read -r line; do
    printf '    %s\n' "${line%%@*}"
  done <<'PLANTED'
let _ = session.run("UID STORE 1 +FLAGS (\Deleted)");@
let _ = session.uid_store("1", "+FLAGS");@
let _ = session.run("UID FETCH 1 BODY[TEXT]");@
let _ = stream.connect(&address);@
let _ = async_imap::Client::new(stream);@
let _ = password.expose_secret();@
let _ = connection.transaction();@
let _ = rfd::FileDialog::new();@
PLANTED
  echo '}'
} > "$PROBE_CMD_RS"
refuses_each \
  'a command that changes mail on the server' \
  'a call that changes mail on the server' \
  'a body fetch that would mark mail read' \
  'connect without the retry policy' \
  'a session opened with no deadline' \
  'a secret is unwrapped outside the places allowed to send it' \
  'deferred transaction' \
  'rfd used outside the dialog adapter'

rm -f "$PROBE_CMD_RS"

# `append` is the one call that changes a server whose name Rust gives to other things as well, so
# it is told apart by what it is handed. The command, beside the reading code in the adapter's own
# folder, with a first argument that is itself a call; again outside the folder, laid out the way
# rustfmt lays out a call too long for its line; and on one line with no bracket in what it is
# handed, the way the corpus filler writes it, where only the commas tell it from the ones below.
cat > "$PROBE_IMAP" <<'PLANTED'
pub async fn probe(session: &mut Session, raw: &str) {
    let _ = session.append(sent.name(), None, None, raw).await;
}
PLANTED
cat > "$PROBE_RS" <<'PLANTED'
pub async fn probe(session: &mut Session, raw: &str) {
    let _ = session
        .append(
            "Sent",
            Some("(\\Seen)"),
            None,
            raw,
        )
        .await;
}
PLANTED
cat > "$PROBE_CMD_RS" <<'PLANTED'
pub async fn probe(session: &mut Session, raw: &str) {
    let _ = session.append("Sent", None, None, raw).await;
}
PLANTED
refuses_each \
  "a call that adds mail on the server -> $PROBE_IMAP" \
  "a call that adds mail on the server -> $PROBE_RS" \
  "a call that adds mail on the server -> $PROBE_CMD_RS"
rm -f "$PROBE_IMAP" "$PROBE_RS" "$PROBE_CMD_RS"

# The command written out by hand, in the very words the one file that sends it uses, beside that
# file in the adapter's own folder: the line the gate lets off there is let off nowhere else.
cat > "$PROBE_IMAP" <<'PLANTED'
pub async fn probe(session: &mut Session, mailbox: &str, raw: &str) {
    let _ = session
        .run_command(format!("APPEND {mailbox} (\\Seen) {{{}}}", raw.len()))
        .await;
}
PLANTED
refuses "a command that changes mail on the server -> $PROBE_IMAP" \
  "an append written out beside the file that sends one"
rm -f "$PROBE_IMAP"

# And the two it must leave alone, two to a line where a builder chains them: a file opened to add
# to and a collection joined to another, each handed the one argument the command never is.
cat > "$PROBE_RS" <<'PLANTED'
pub fn probe(options: &mut OpenOptions, keep: bool, more: &mut Vec<u8>) {
    options.append(true).append(keep);
    let mut written = Vec::new();
    written.append(more);
    written.append(&mut self.pending);
}
PLANTED
allows "a file opened to add to, and a collection joined to another"
rm -f "$PROBE_RS"

# A connection opened to act anywhere but the file that acts, by each road to the constructor:
# written plainly beside the reading code, through a turbofish, as a raw identifier on a type in
# angle brackets, and handed on as a value under the name it was imported by. On any of them the
# guard lets the five through, however a call to them is spelled.
cat > "$PROBE_IMAP" <<'PLANTED'
pub fn probe(stream: TcpStream) -> ReadOnly<TcpStream> {
    ReadOnly::acting(stream)
}
PLANTED
cat > "$PROBE_RS" <<'PLANTED'
pub fn probe(stream: TcpStream) -> ReadOnly<TcpStream> {
    ReadOnly::<TcpStream>::acting(stream)
}
PLANTED
cat > "$PROBE_CMD_RS" <<'PLANTED'
pub fn probe(stream: TcpStream) -> ReadOnly<TcpStream> {
    <ReadOnly<TcpStream>>::r#acting(stream)
}
PLANTED
cat > "$PROBE_APP" <<'PLANTED'
use crate::infrastructure::imap::read_only::ReadOnly as Guard;

pub fn probe(stream: Option<TcpStream>) -> Option<Guard<TcpStream>> {
    stream.map(Guard::acting)
}
PLANTED
refuses_each \
  "a connection opened to act -> $PROBE_IMAP" \
  "a connection opened to act -> $PROBE_RS" \
  "a connection opened to act -> $PROBE_CMD_RS" \
  "a connection opened to act -> $PROBE_APP"
rm -f "$PROBE_IMAP" "$PROBE_RS" "$PROBE_CMD_RS" "$PROBE_APP"

# And what only shares the word: the purpose named, the trait an acting session is driven through,
# and a binding called after it.
cat > "$PROBE_RS" <<'PLANTED'
pub fn probe(acting: &mut dyn Acting, purpose: Purpose) -> bool {
    let _ = crate::ports::mail_actions::Acting::carry_out(acting, action);
    purpose == Purpose::Acting
}
PLANTED
allows "the purpose named, and the trait an acting session is driven through"
rm -f "$PROBE_RS"

# A client of the IMAP library made where no connection is opened, by each road to it. The first is
# a file that signs in on its own: the plain constructor, beside the reading code, with the deadline
# every connection has and no guard, refused both for where it is and for what it is made over. The
# rest are told apart by the line the gate quotes. Four write the path to the type, each over no
# guard: the parameter spelled out, the type in angle brackets, the type as a raw identifier, and
# the constructor handed on as a value. Six take it in by an import or an alias: under another name,
# in a group from the library's root spread over lines the way rustfmt spreads one, as everything at
# the root, with the library under another name, with the root under another name inside a group,
# and as a type alias. Four take everything at the root inside a group: beside the root itself,
# beside a type from one of its modules, in a group of its own nested in the first, and spread over
# lines the way rustfmt spreads one. And four hide what they take behind a comment, which the
# compiler reads as a space: a semicolon in a line comment before the client, a line of comment
# before the glob, a semicolon in a block comment with another nested in it before the client, and
# a block comment between the library and the name it is given.
cat > "$PROBE_IMAP" <<'PLANTED'
pub async fn probe(stream: TcpStream) {
    let client = async_imap::Client::new(Deadline::over(stream, Silence::of(WAIT)));
}
PLANTED
cat > "$PROBE_RS" <<'PLANTED'
pub fn probe(stream: TcpStream, streams: Option<TcpStream>) {
    let _ = async_imap::Client::<TcpStream>::new(stream);
    let _ = <async_imap::Client<TcpStream>>::new(stream);
    let _ = async_imap::r#Client::new(stream);
    let _ = streams.map(async_imap::Client::new);
}
PLANTED
cat > "$PROBE_CMD_RS" <<'PLANTED'
use async_imap::Client as Imap;
use async_imap::{
    error::Error,
    Client,
};
use async_imap::*;
use async_imap as imap;
use async_imap::{self as library, types::Flag};
type Connection<S> = async_imap::Client<S>;
use async_imap::{self, *};
use async_imap::{error::Error, *};
use async_imap::{error::Error, {*}};
use async_imap::{
    error::Result,
    *,
};
use async_imap::{
    error::Error, // the library's; not this one's
    Client,
};
use async_imap::{
    error::Result,
    // the rest of the root
    *,
};
use async_imap::{/* errors; /* nested */ and more; */ error::Error, Client};
use async_imap/* the library, under a name */as imap;
PLANTED
outside="a client of the IMAP library outside the two files that open a connection"
unguarded="a client of the IMAP library over a stream the guard does not read"
imported="the IMAP library's client taken in by an import or an alias"
refuses_each \
  "$outside -> $PROBE_IMAP:2:" \
  "$unguarded -> $PROBE_IMAP:2:" \
  "$outside -> $PROBE_RS:2:" \
  "$outside -> $PROBE_RS:3:" \
  "$outside -> $PROBE_RS:4:" \
  "$outside -> $PROBE_RS:5:" \
  "$unguarded -> $PROBE_RS:2:" \
  "$unguarded -> $PROBE_RS:3:" \
  "$unguarded -> $PROBE_RS:4:" \
  "$unguarded -> $PROBE_RS:5:" \
  "$imported -> $PROBE_CMD_RS:1:" \
  "$imported -> $PROBE_CMD_RS:2:" \
  "$imported -> $PROBE_CMD_RS:6:" \
  "$imported -> $PROBE_CMD_RS:7:" \
  "$imported -> $PROBE_CMD_RS:8:" \
  "$imported -> $PROBE_CMD_RS:9:" \
  "$imported -> $PROBE_CMD_RS:10:" \
  "$imported -> $PROBE_CMD_RS:11:" \
  "$imported -> $PROBE_CMD_RS:12:" \
  "$imported -> $PROBE_CMD_RS:13:" \
  "$imported -> $PROBE_CMD_RS:17:" \
  "$imported -> $PROBE_CMD_RS:21:" \
  "$imported -> $PROBE_CMD_RS:26:" \
  "$imported -> $PROBE_CMD_RS:27:"
rm -f "$PROBE_IMAP" "$PROBE_RS" "$PROBE_CMD_RS"

# And what only shares the library, beside the reading code that uses it: its errors and its types,
# everything in one of its modules, on its own and inside a group from the root, a group from its
# root that leaves the client out, on one line and over several, a comment in such a group that names
# the client, the session a connection already opened hands on, and an alias of that session.
cat > "$PROBE_IMAP" <<'PLANTED'
use async_imap::error::Error as ImapError;
use async_imap::types::{Flag, NameAttribute};
use async_imap::types::*;
use async_imap::{error::Result, types::Fetch};
use async_imap::{
    imap_proto::Response,
    types::Mailbox,
};
use async_imap::{error::Error, types::*};
use async_imap::{
    // the Client itself is made in mod.rs; this file reads its answers
    imap_proto::Response as Answer,
};

type Held<S> = async_imap::Session<S>;

pub async fn probe(session: &mut async_imap::Session<TcpStream>) -> Option<ImapError> {
    None
}
PLANTED
allows "the library's errors and types, a group that leaves the client out or names it in a comment, and a session"
rm -f "$PROBE_IMAP"

# An account's passwords written past the ones held this run: the store alone handed to the use
# cases that add, change and remove an account. The gate quotes each function by its signature, so
# the probes are told apart by name. The first two hand over the store and make no call to
# `passwords`, and the two after them do the same with the path to the use case written another way:
# its type parameters spelled out, and the type named inside angle brackets. The next five make the
# call and reach the store some other way, each by one road the others do not take: a binding of its
# own, the state taken apart, the field reached through `inner()`, a store made on the spot, and one
# asked of the start. The last names the call only in a comment.
cat > "$PROBE_CMD_RS" <<'PLANTED'
pub async fn probe_adds(state: State<'_, AppState>) {
    let secrets = state.secrets.clone();
    let _ = AddAccount::new(SqliteAccountRepository::new(pool), secrets, factory);
}

pub async fn probe_changes(state: State<'_, AppState>) {
    let _ = crate::application::accounts::ChangeAccount::new(accounts, state.secrets.clone(), factory);
}

pub async fn probe_spells(state: State<'_, AppState>) {
    let _ = AddAccount::<_, _, _>::new(SqliteAccountRepository::new(pool), state.secrets.clone(), factory);
}

pub async fn probe_brackets(state: State<'_, AppState>) {
    let _ = <RemoveAccount<_, _, _>>::new(accounts, state.secrets.clone(), indexes);
}

pub async fn probe_removes(state: State<'_, AppState>) {
    let store = &state.secrets;
    let _ = passwords(&state.credentials, &state.secrets);
    let _ = RemoveAccount::new(accounts, store.clone(), indexes);
}

pub async fn probe_unpacks(state: State<'_, AppState>) {
    let _held = passwords(&state.credentials, &state.secrets);
    let AppState { secrets, .. } = state.inner();
    let _ = AddAccount::new(SqliteAccountRepository::new(pool), secrets.clone(), factory);
}

pub async fn probe_reaches(state: State<'_, AppState>) {
    let _held = passwords(&state.credentials, &state.secrets);
    let _ = ChangeAccount::new(accounts, state.inner().secrets.clone(), factory);
}

pub async fn probe_builds(state: State<'_, AppState>) {
    let _held = passwords(&state.credentials, &state.secrets);
    let fresh = SecretStore::System(SystemCredentialStore::new());
    let _ = AddAccount::new(SqliteAccountRepository::new(pool), fresh, factory);
}

pub async fn probe_asks(state: State<'_, AppState>) {
    let _held = passwords(&state.credentials, &state.secrets);
    let fresh = crate::bootstrap::the_store_for(&directory);
    let _ = RemoveAccount::new(accounts, fresh, indexes);
}

pub async fn probe_mentions(state: State<'_, AppState>) {
    // passwords(&state.credentials, &state.secrets)
    let _ = AddAccount::new(SqliteAccountRepository::new(pool), held, factory);
}
PLANTED
refuses_each 'fn probe_adds' 'fn probe_changes' 'fn probe_spells' 'fn probe_brackets' \
  'fn probe_removes' 'fn probe_unpacks' 'fn probe_reaches' 'fn probe_builds' 'fn probe_asks' \
  'fn probe_mentions'

# And the form it asks for, with a comment that names the store, beside a test module building the
# same use case from a store of its own: a comment reaches nothing, a test is not a command, and what
# a test hands over is the test's business.
cat > "$PROBE_CMD_RS" <<'PLANTED'
pub async fn probe_adds(state: State<'_, AppState>) {
    // Never state.secrets alone: what is held this run would be written past.
    let secrets = passwords(&state.credentials, &state.secrets);
    let _ = AddAccount::new(SqliteAccountRepository::new(pool), secrets, factory);
}

#[cfg(test)]
mod tests {
    fn probe_removes() {
        let _ = RemoveAccount::new(accounts, store.clone(), indexes);
    }
}
PLANTED
allows "an account's use case handed the passwords held this run"
rm -f "$PROBE_CMD_RS"

printf '\nAn identity carried in by machinery rather than by prose (invariant 21)\n'
: > "$PROBE_KEY"
stage "$PROBE_KEY" -f
refuses "an identity file does not belong in this repository" "a .p12 forced past .gitignore"
git rm --cached -q --ignore-unmatch "$PROBE_KEY" >/dev/null 2>&1
rm -f "$PROBE_KEY"

mkdir -p "$(dirname "$PROBE_MANIFEST")"
printf '[package]\nname = "probe"\nauthors = ["somebody"]\n' > "$PROBE_MANIFEST"
stage "$PROBE_MANIFEST" -f
refuses "a manifest naming its authors" "an authors field in a crate manifest"
git rm --cached -q --ignore-unmatch "$PROBE_MANIFEST" >/dev/null 2>&1
rm -rf "$(dirname "$PROBE_MANIFEST")"

printf '\nA signer named in the bundle configuration (invariant 21)\n'
for field in signingIdentity teamId appleId certificateThumbprint signCommand publisher; do
  python3 -c "
import json, sys
config = sys.argv[1]
written = json.load(open('src-tauri/tauri.conf.json'))
written.setdefault('bundle', {})[sys.argv[2]] = 'whoever'
open(config, 'w').write(json.dumps(written, indent=2) + '\n')
" "$CONFIG" "$field"
  refuses "a signing identity in the bundle configuration" "$field"
done
cp src-tauri/tauri.conf.json "$CONFIG"

printf '\nA script injected into every frame, under each name that does it\n'
# Two of these ship with Tauri and the gate knew one. All three reach the frame holding a message.
{
  echo 'pub fn probe() {'
  for spelling in initialization_script_for_all_frames initialization_script_on_all_frames js_init_script_on_all_frames; do
    printf '    let _ = "%s";\n' "$spelling"
  done
  echo '}'
} > "$PROBE_RS"
refuses_each \
  initialization_script_for_all_frames \
  initialization_script_on_all_frames \
  js_init_script_on_all_frames
rm -f "$PROBE_RS"

printf '\n`unsafe` anywhere but the one call that puts a picture in the Dock\n'
{
  echo '#[allow(unsafe_code)]'
  echo 'pub fn probe_one() { unsafe { probe_two() } }'
  echo 'pub unsafe fn probe_two() {}'
  echo 'pub struct Probe;'
  echo 'unsafe impl Send for Probe {}'
  echo 'pub const PROBE: &str = "style-src '"'"'unsafe-inline'"'"'";'
} > "$PROBE_RS"
refuses_each "__gate_probe.rs:1:" "__gate_probe.rs:2:" "__gate_probe.rs:3:" "__gate_probe.rs:5:"
said="$(bash scripts/check-layering.sh 2>&1)"
if printf '%s' "$said" | grep -qF "__gate_probe.rs:6:"; then
  printf '\033[31m  REFUSED WRONGLY: a content policy keyword in a string\033[0m\n'
  missed=1
else
  printf '  allowed: a content policy keyword in a string\n'
fi
rm -f "$PROBE_RS"

printf '\nsrcdoc, in the form that walked past a match on `srcdoc =`\n'
printf 'export const probe = (f: HTMLIFrameElement, h: string) => {\n  f["srcdoc"] = h;\n};\n' > "$PROBE_TSX"
refuses "srcdoc" 'frame["srcdoc"] = html'
rm -f "$PROBE_TSX"

printf '\nA second frame, in a file extension the old globs did not read\n'
# `<iframe srcdoc={html} sandbox="allow-scripts" />` in a `.jsx` matched neither of the two greps.
printf 'export const Probe = () => <iframe srcdoc={"x"} />;\n' > "$PROBE_JSX"
refuses "only iframe" "a second iframe in .jsx"
rm -f "$PROBE_JSX"

# On its own, and asserted by the rule's own words. Planted beside `srcdoc`, this would pass with
# the allow-* rule deleted: the srcdoc rule quotes the line it refuses, that line holds
# `allow-scripts`, and a probe looking for `allow-` finds the neighbour's message.
printf 'export const Probe = () => <iframe sandbox="allow-scripts" />;\n' > "$PROBE_JSX"
refuses "forbidden permanently: allow-* in a sandbox attribute" "allow-scripts in .jsx"
rm -f "$PROBE_JSX"

printf '\nParsing hostile HTML in the window that holds IPC (invariant 8)\n'
# None of these needs a dependency, and each parses HTML as surely as `dangerouslySetInnerHTML`.
{
  echo 'export const probe = (el: HTMLElement, h: string) => {'
  echo '  new DOMParser().parseFromString(h, "text/html");'
  echo '  el.innerHTML = h;'
  echo '  el.insertAdjacentHTML("beforeend", h);'
  echo '};'
} > "$PROBE_TSX"
refuses_each \
  'new DOMParser().parseFromString' \
  'el.innerHTML = h' \
  'el.insertAdjacentHTML'
rm -f "$PROBE_TSX"

printf '\nA content security policy that permits what it does not name\n'
python3 - "$CONFIG" <<'POLICIES'
import json, subprocess, sys

config = sys.argv[1]
base = json.loads(open(config).read())

# Each of these passed the checks that read three named directives and skipped the rest.
tried = [
    ("img-src absent, inheriting https: from default-src",
     "default-src 'self' https:; script-src 'self'; frame-src lyomsg:"),
    ("an empty policy, which permits everything", ""),
    ("no default-src, so every omitted directive permits everything", "script-src 'self'"),
    ("frame-src 'self', which frames a message as part of this window",
     "default-src 'none'; script-src 'self'; img-src 'self'; frame-src 'self'"),
    ("'unsafe-inline' in script-src",
     "default-src 'none'; script-src 'self' 'unsafe-inline'; img-src 'self'; frame-src lyomsg:"),
]

missed = False
for what, policy in tried:
    written = json.loads(json.dumps(base))
    written["app"]["security"]["csp"] = policy
    open(config, "w").write(json.dumps(written, indent=2) + "\n")

    said = subprocess.run(["bash", "scripts/check-layering.sh"], capture_output=True, text=True).stdout
    if "LAYERING" in said and "csp" in said:
        print(f"  refused: {what}")
    else:
        print(f"\033[31m  LET THROUGH: {what}\033[0m")
        missed = True

# The one nobody looked at: the policy the development window runs under, which is where a
# relaxation is made before it is copied across.
written = json.loads(json.dumps(base))
written["app"]["security"]["devCsp"] = "default-src 'self' https:"
open(config, "w").write(json.dumps(written, indent=2) + "\n")
said = subprocess.run(["bash", "scripts/check-layering.sh"], capture_output=True, text=True).stdout
if "devCsp" in said:
    print("  refused: devCsp permitting the open network")
else:
    print("\033[31m  LET THROUGH: devCsp permitting the open network\033[0m")
    missed = True

sys.exit(1 if missed else 0)
POLICIES
[ $? -eq 0 ] || missed=1

printf '\nA byte a Windows PowerShell will misread, in a script meant to be run there\n'
# An em dash, written as UTF-8 with no byte order mark: Windows PowerShell 5.1 decodes it as ANSI,
# and the last of the three characters it becomes is a right double quote, which closes a string
# forty lines before anybody reads an error. The rule reads bytes with `awk` in the C locale: macOS
# has no `grep -P`, and a check built on it finds nothing there and looks as if it had checked.
printf 'Write-Host "a \342\200\224 b"\n' > "$PROBE_PS1"
refuses "non-ASCII byte" "an em dash in a .ps1"
rm -f "$PROBE_PS1"

printf 'echo a \342\200\224 b\r\n' > "$PROBE_CMD"
refuses "non-ASCII byte" "an em dash in a .cmd"
rm -f "$PROBE_CMD"

printf '\nAn address that could reach somebody\n'
# By its shape and not by its spelling. A list of names holds only the ones somebody already found,
# and could never hold the ones that matter: writing a real name into a gate to refuse it is putting
# it in the repository.
{
  printf 'A note about a person.\n\n'
  printf 'Written to by w7x2q@nothing.zz on a Tuesday.\n'
} > "$PROBE_MD"
refuses "could reach somebody" "a real recipient at a real domain"
rm -f "$PROBE_MD"

# A name no list would hold. The addresses here name nobody and reach nowhere -- `.zz` is not a
# domain anybody has -- because a probe carrying a real one would be the very thing this gate exists
# to refuse, sitting in the file that proves the gate works.
printf 'Reviewed by k4m1p@nowhere.zz.\n' > "$PROBE_MD"
refuses "could reach somebody" "a name no list would hold"
rm -f "$PROBE_MD"

# Each address on its own. Beside one that may stand -- the cast at a reserved domain, a picture's
# density name -- another is refused all the same.
{
  printf 'Copied to zeta@example.com and q1@nothing.zz together.\n'
  printf 'The logo is logo@2x.png, sent by q2@nothing.zz.\n'
} > "$PROBE_MD"
refuses_each "q1@nothing.zz" "q2@nothing.zz"
rm -f "$PROBE_MD"

# A domain to its end. One that only begins like a reserved one, or like the development server's,
# is a domain somebody may own.
{
  printf 'q3@mail.testing.zz\n'
  printf 'q4@mail.invalidator.zz\n'
  printf 'q5@lyonoyl.zz\n'
  printf 'q6@example.com.zz\n'
} > "$PROBE_MD"
refuses_each "q3@mail.testing.zz" "q4@mail.invalidator.zz" "q5@lyonoyl.zz" "q6@example.com.zz"
rm -f "$PROBE_MD"

# Only the cast stands at a domain that answers. A generic word is not one this repository built.
printf 'Write to user@nothing.zz or noreply@nothing.zz.\n' > "$PROBE_MD"
refuses_each "user@nothing.zz" "noreply@nothing.zz"
rm -f "$PROBE_MD"

# A message kept as a fixture, through its encodings: base64, and a quoted-printable soft break in
# the middle of an address, each hide it from anything that reads bytes.
{
  printf 'From: zeta@example.com\r\nTo: kappa@example.net\r\nSubject: probe\r\n'
  printf 'MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary="b"\r\n\r\n'
  printf -- '--b\r\nContent-Type: text/plain; charset=utf-8\r\n'
  printf 'Content-Transfer-Encoding: base64\r\n\r\n'
  printf '%s\r\n' "$(printf 'Reply to q8@nothing.zz today.' | base64)"
  printf -- '--b\r\nContent-Type: text/plain; charset=utf-8\r\n'
  printf 'Content-Transfer-Encoding: quoted-printable\r\n\r\n'
  printf 'Reply to q9@noth=\r\ning.zz today.\r\n'
  printf -- '--b--\r\n'
} > "$PROBE_EML"
refuses_each "q8@nothing.zz" "q9@nothing.zz"
rm -f "$PROBE_EML"

# An account in a scene the build for screenshots plays may stand at the product's own domain, in
# the scene's own file and nowhere else, and at that domain whole. The sponsors' mailbox stands in
# the README alone.
mkdir -p "$(dirname "$PROBE_SCENE_DEEP")"
printf '{ "accounts": [ { "address": "q90@lyonoyl.com" }, { "address": "Q91@Lyonoyl.Com" } ] }\n' > "$PROBE_SCENE"
allows "an account at the product's own domain in a scene"
rm -f "$PROBE_SCENE"
printf '{ "accounts": [ { "address": "q92@mail.lyonoyl.com" }, { "address": "q93@lyonoyl.com.zz" } ] }\n' > "$PROBE_SCENE"
printf '<p>Write to q94@lyonoyl.com.</p>\n' > "$PROBE_SCENE_HTML"
printf '{ "accounts": [ { "address": "q95@lyonoyl.com" } ] }\n' > "$PROBE_SCENE_DEEP"
printf 'Write to q96@lyonoyl.com, or to patron@lyonoyl.com.\n' > "$PROBE_MD"
refuses_each "q92@mail.lyonoyl.com" "q93@lyonoyl.com.zz" "q94@lyonoyl.com" "q95@lyonoyl.com" \
  "q96@lyonoyl.com" "patron@lyonoyl.com"
rm -f "$PROBE_SCENE" "$PROBE_SCENE_HTML" "$PROBE_SCENE_DEEP" "$PROBE_MD"
rmdir "$(dirname "$PROBE_SCENE_DEEP")" "$PROBE_SCENE_DIR"

# What nothing answers for may stand under any name: a generic word under example.com, and the
# development mail server's own domain.
printf 'Sent to user@mail.example.com and test@lyonoyl.local.\n' > "$PROBE_MD"
allows "a generic name where nothing answers"
rm -f "$PROBE_MD"

# Read as a person reads it, through what hides an address from anything reading bytes: a name and a
# domain in another script, an at sign percent-encoded, written as an entity or full width, a
# recipient inside a bounce address, a body in base64 inside a string, and somebody at a machine.
# Each reading in a file of its own, so one cannot stand in for another.
{
  printf 'Write to \317\210\316\276\317\206@\317\210\316\276\317\206\317\211.zz.\n'
  printf 'Write to q35&#64;nothing.zz.\n'
  printf 'Write to q36\357\274\240nothing.zz.\n'
} > "$PROBE_MD"
{
  printf 'mailto:q37%%40nothing.zz\n'
  printf 'Return-Path: bounces+q38=nothing.zz@lists.example.com\n'
  printf 'Copy it over with scp notes.md q39@buildbox:/tmp\n'
  printf 'q40@laptop ~ %% ls\n'
} > "$PROBE_MD_TWO"
printf 'pub const PROBE: &str = "%s";\n' "$(printf 'Reply to q34@nothing.zz before Friday.' | base64)" > "$PROBE_RS"
refuses_each "$(printf '\317\210\316\276\317\206')@" "q37@nothing.zz" "q35@nothing.zz" "q36@nothing.zz" \
  "q38@nothing.zz" "q39@buildbox" "q40@laptop" "q34@nothing.zz"
rm -f "$PROBE_MD" "$PROBE_MD_TWO" "$PROBE_RS"

# Text a Windows tool wrote, which has a zero byte in every other place; a file's own name; where a
# link points; and what is staged for the next commit when the disk no longer holds it.
printf 'Written to q32@nothing.zz.\n' | iconv -f UTF-8 -t UTF-16LE > "$PROBE_MD"
printf 'nothing to see\n' > "$PROBE_NAMED"
ln -s "/Users/somebodyreal/notes.md" "$PROBE_LINK"
printf 'Sent by q31@nothing.zz.\n' > "$PROBE_STAGED"
stage "$PROBE_STAGED"
printf 'Nothing here now.\n' > "$PROBE_STAGED"
refuses_each "q32@nothing.zz" "__gate_probe_q33@nothing.zz.md" "/Users/somebodyreal" "q31@nothing.zz"
git rm --cached -q -f "$PROBE_STAGED"
rm -f "$PROBE_MD" "$PROBE_NAMED" "$PROBE_LINK" "$PROBE_STAGED"

# What may stand however it is written: the cast in its own Greek at a name under .example, a
# picture's density name in capitals, and a reserved address a quoted-printable soft break cuts in
# two on the face of a message.
printf 'The cast in its own script: \316\266\316\256\317\204\316\261@\316\266\316\256\317\204\316\261.example, and logo@1.5x.PNG.\n' > "$PROBE_MD"
{
  printf 'From: zeta@example.com\r\nSubject: probe\r\nMIME-Version: 1.0\r\n'
  printf 'Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n'
  printf 'Copied to user@example.ne=\r\nt today.\r\n'
} > "$PROBE_EML"
allows "the cast in another script, a picture's name, and a reserved address cut in two"
rm -f "$PROBE_MD" "$PROBE_EML"

# What a string in code holds once the compiler has read it: base64 glued to the escape before it,
# a quoted-printable line continued with a backslash; an encoded word on its own. And a message whose
# percent-encoding and entities are themselves cut by quoted-printable soft breaks, and a file that
# is not text with an address in plain letters inside it.
{
  printf 'pub const BODY: &[u8] = b"Content-Transfer-Encoding: base64\\r\\n\\r\\nUmVwbHkgdG8gcTczQG5vdGhpbmcuenogYmVmb3JlIEZyaWRheS4=\\r\\n";\n'
  printf 'pub const QP: &[u8] = b"Content-Transfer-Encoding: quoted-printable\\r\\n\\r\\n\\\n'
  printf '    Reply to q45@noth=\\r\\n\\\n'
  printf '    ing.zz today.\\r\\n";\n'
} > "$PROBE_RS"
{
  printf 'From: zeta@example.com\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n'
  printf 'Content-Transfer-Encoding: quoted-printable\r\n\r\n'
  printf '<a href=3D"https://lists.example.org/unsubscribe?email=3Dq43%%40noth=\r\n'
  printf 'ing.zz">Unsubscribe</a> Write to q44&#64;noth=\r\n'
  printf 'ing.zz today.\r\n'
} > "$PROBE_EML"
printf 'PK\003\004\000\000 plain q49@nothing.zz inside \000\000\n' > "$PROBE_BIN"
printf 'Subject: =?utf-8?Q?q46=40nothing.zz?=\n' > "$PROBE_MD_TWO"
refuses_each "q73@nothing.zz" "q45@nothing.zz" "q46@nothing.zz" "q43@nothing.zz" "q44@nothing.zz" \
  "q49@nothing.zz"
rm -f "$PROBE_RS" "$PROBE_EML" "$PROBE_BIN" "$PROBE_MD_TWO"

# Somebody at a machine as each system's shell draws its prompt, one of them in colour; a home
# directory however a system writes it; and identities a real system minted.
{
  printf 'somebodydeb@buildbox:~/lyonoyl$ cargo test\n'
  printf '[somebodyfed@buildbox lyonoyl]$ cargo test\n'
  printf 'somebodymac@Somebodys-MacBook-Pro lyonoyl %% cargo test\n'
  printf 'somebodygit@LAPTOP-Q1 MINGW64 ~/lyonoyl (dev)\n'
  printf '\033[32msomebodyansi@laptop\033[0m ~ %% ls\n'
  printf '{"path": "\\/Users\\/somebodyjson\\/Library\\/Logs"}\n'
  printf 'cd /users/somebodylower/Development\n'
  printf '/c/users/somebodygitbash/lyonoyl\n'
  printf '\\\\buildbox\\c$\\Users\\somebodyunc\\lyonoyl\n'
  printf '%%SystemDrive%%\\Users\\somebodysysdrive\\AppData\n'
  printf '\\\\wsl.localhost\\Ubuntu\\home\\somebodywsl\\lyonoyl\n'
  printf 'C:\\Documents and Settings\\somebodyxp\\\n'
  printf '/homes/somebodynfs/lyonoyl\n'
  printf 'Users/somebodyrelative/AppData/Local\n'
  printf 'Authority=Developer ID Application: Somebody Realname (ABCDEFGHIJ)\n'
  printf 'C:\\$Recycle.Bin\\S-1-5-21-1234567890-1234567890-1234567890-1001\\\n'
  printf 'X-MS-Exchange-CrossTenant-Id: 12345678-1234-4123-8123-123456789abc\n'
  printf '/O=EXCHANGELABS/OU=EXCHANGE ADMINISTRATIVE GROUP/CN=RECIPIENTS/CN=0a1b2c3d-somebodydn\n'
} > "$PROBE_MD"
refuses_each "somebodydeb@buildbox" "somebodyfed@buildbox" "somebodymac@Somebodys-MacBook-Pro" \
  "somebodygit@LAPTOP-Q1" "somebodyansi@laptop" "/Users/somebodyjson" "/users/somebodylower" \
  "/c/users/somebodygitbash" "somebodyunc" "somebodysysdrive" "somebodywsl" "somebodyxp" \
  "somebodynfs" "somebodyrelative" "a signing identity" "S-1-5-21-1234567890" \
  "12345678-1234-4123-8123-123456789abc" "somebodydn"
rm -f "$PROBE_MD"

# What looks like those shapes and names nobody: an address at a reserved domain after a flag or a
# variable, a forwarder's SRS around one, a placeholder path, an approximation, a handle in a cell
# beside a network's name, a package's version after rsync, the cast in its own Greek at a
# provider, a tenant that counts from zero and a team made of one letter.
{
  printf 'git send-email --to=zeta.quorin@example.com 0001.patch\n'
  printf 'LYONOYL_TEST_ACCOUNT=zeta.quorin@example.com cargo test\n'
  printf 'Return-Path: <SRS0=AbCd=XY=example.com=zeta.quorin@forwarder.example.net>\n'
  printf 'Install into C:\\Users\\<you>\\AppData\\Local and /Users/\342\200\246 for the rest.\n'
  printf 'Halve it: ~n/2 steps.\n'
  printf '<table><tr><td>Instagram</td><td><a href="https://social.example/zeta.quorin">@zeta.quorin</a></td></tr></table>\n'
  printf 'rsync -az --exclude node_modules react@19.2.4\n'
  printf 'Written to \316\266\316\256\317\204\316\261@gmail.com.\n'
  printf 'X-MS-Exchange-CrossTenant-Id: 00000000-0000-4000-8000-000000000006\n'
  printf 'Developer ID Application: Something (XXXXXXXXXX)\n'
} > "$PROBE_MD"
allows "shapes that name nobody"
rm -f "$PROBE_MD"

# A certificate in a real name with its team zeroed, or in quotes with no team; a team, an API key's
# issuer; a recipient inside the address SPF checked; a second forwarder's SRS; an address split by a
# tag, by a character nobody sees, and inside an SRS address; an attachment's name the message itself
# writes in two pieces.
{
  printf 'Developer ID Application: Somebody Realname (XXXXXXXXXX)\n'
  printf 'APPLE_SIGNING_IDENTITY="Developer ID Application: Somebody Othername"\n'
  printf 'export APPLE_TEAM_ID=ABCDE12345\n'
  printf 'xcrun notarytool submit Lyonoyl.dmg --team-id "FGHIJ67890" --wait\n'
  printf 'APPLE_API_ISSUER=12345678-1234-4123-8123-123456789abc\n'
  printf 'Authentication-Results: mx.example.com; spf=pass smtp.mailfrom=bounces+q56=nothing.zz@lists.example.com\n'
  printf 'Return-Path: <SRS1=HHHH=forwarder.example.net==HHHH=TT=nothing.zz=q60@forwarder.example.org>\n'
  printf '<p>Write to q51@noth<b></b>ing.zz</p>\n'
  printf 'Write to q52@noth\342\200\213ing.zz today.\n'
  printf 'Forwarded as SRS0=AbCd=XY=nothing.zz=q53@forwarder.example.net\n'
  printf '%s\n' 'eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4' 'eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4IHJlcGx5IHRvIHE2M0Bub3Ro' 'aW5nLnp6'
} > "$PROBE_MD"
{
  printf 'From: zeta@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary="b"\r\n\r\n'
  printf -- '--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment;\r\n'
  printf " filename*0*=utf-8''q61%%40no;\r\n filename*1*=thing.zz.txt\r\n\r\nnothing\r\n--b--\r\n"
} > "$PROBE_EML"
refuses_each "Somebody Realname" "Somebody Othername" "ABCDE12345" "FGHIJ67890" \
  "12345678-1234-4123-8123-123456789abc" "q56@nothing.zz" "q60@nothing.zz" "q51@nothing.zz" \
  "q52@nothing.zz" "q53@nothing.zz" "q61@nothing.zz" "q63@nothing.zz"
rm -f "$PROBE_MD" "$PROBE_EML"

# What names nobody and looks like it might: a screenshot's random bytes and a photograph attached to
# a message, a feature folder called home and a web page under one, SSH's algorithms, the accounts a
# system or a container makes for itself, an unsubscribe link with an address in its query, a label
# beside a handle, a directory name zeroed, the cast with a tag and with its surname at a provider, a
# forge's SSH user, and a package's tag on a line that mentions ssh.
python3 -c 'import random; r = random.Random(1); open("docs/__gate_probe.png", "wb").write(b"\x89PNG\r\n\x1a\n" + r.randbytes(2_000_000))'
python3 - <<'PHOTO'
import base64, random
r = random.Random(17)
picture = base64.encodebytes(r.randbytes(300_000)).decode().replace("\n", "\r\n")
with open("tests/fixtures/eml/__gate_probe.eml", "w", newline="") as message:
    message.write(
        "From: zeta@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n"
        "--b\r\nContent-Type: image/jpeg\r\nContent-Transfer-Encoding: base64\r\n\r\n" + picture + "--b--\r\n"
    )
PHOTO
{
  printf 'import { HomeScreen } from "../features/home/HomeScreen";\n'
  printf 'See https://example.com/home/index.html for the rest.\n'
  printf 'Ciphers chacha20-poly1305@openssh.com,aes256-gcm@openssh.com\n'
  printf 'KexAlgorithms=curve25519-sha256@libssh.org\n'
  printf 'Clone into /home/ubuntu/lyonoyl and keep keys in C:\\Users\\Administrator\\.ssh\n'
  printf 'root@lyonoyl-mail-server:/# doveadm who\n'
  printf 'List-Unsubscribe: <https://lists.example.org/u?list-id=0000&e-mail=zeta.quorin@example.com>\n'
  printf '<p><span>Instagram</span><span>@zeta.quorin</span></p>\n'
  printf '/O=EXAMPLE/OU=GROUP/CN=RECIPIENTS/CN=00000000000000000000000000000000-0000\n'
  printf 'Written to zeta+news@gmail.com and to zeta.quorin@gmail.com.\n'
  printf 'git clone git@github.com:example/repo.git\n'
  printf 'ssh in and run npx create-tauri-app@latest there.\n'
} > "$PROBE_MD"
allows "what names nobody and looks as if it might"
rm -f "$PROBE_MD" "$PROBE_PNG" "$PROBE_EML"

# And the next commit. Under the product's name it is the product's, at its address and unsigned.
export GIT_AUTHOR_NAME="Lyonoyl Mail" GIT_AUTHOR_EMAIL="0+somebody@users.noreply.github.com"
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=true
refuses_each "under the product's name at an address the product does not commit under" "commit.gpgsign is on"
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0

# Under a name of its own it is a contributor's, and not asked about: signature and time zone
# included.
export GIT_AUTHOR_NAME="Zeta Quorin" GIT_AUTHOR_EMAIL="zeta@example.com" GIT_AUTHOR_DATE="@1000000000 +0700"
export GIT_COMMITTER_NAME="Zeta Quorin" GIT_COMMITTER_EMAIL="zeta@example.com"
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=true
said="$(bash scripts/check-layering.sh 2>&1)"
if printf '%s' "$said" | grep -qE "the next commit would be|commit.gpgsign is on"; then
  printf '\033[31m  REFUSED WRONGLY: a contributor under a name of their own\033[0m\n'
  missed=1
else
  printf '  allowed: a contributor under a name of their own\n'
fi
unset GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_AUTHOR_DATE GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0

# And the built cast at a real provider, which is not that and must pass: a test about recognising a
# provider cannot be written against example.com.
printf 'The form fills in for zeta@gmail.com.\n' > "$PROBE_MD"
allows "the built cast at a provider"
rm -f "$PROBE_MD"

printf '\nA path holding somebody home directory\n'
printf 'It was built at C:\\Users\\somebodyreal and there.\n' > "$PROBE_MD"
refuses "a path holding a home directory" "a Windows home path as a person writes it"

printf 'It was built at /Users/somebodyreal/code.\n' > "$PROBE_MD"
refuses "home directory" "a path holding a home directory"
rm -f "$PROBE_MD"

# Each path on its own, as each address is: one that names nobody does not excuse the one beside it.
printf 'Shared from C:\\Users\\Public and from /home/somebodyreal.\n' > "$PROBE_MD"
refuses "/home/somebodyreal" "a home path beside one that names nobody"
rm -f "$PROBE_MD"

# The other ways a home directory is written: ~name/ as a shell and an IMAP server write it, the
# folder an agent tool names after a project's path, a drive in small letters, and a name outside
# ASCII.
{
  printf 'Delivered to ~somebodyreal/mail/INBOX.\n'
  printf 'Kept in .claude/projects/-Users-somebodyhere-Development-app/memory.\n'
  printf 'Saved to c:\\users\\somebodyelse\\notes.\n'
  printf 'Built in /Users/\316\250\316\276\317\206/code.\n'
} > "$PROBE_MD"
refuses_each "~somebodyreal" "-Users-somebodyhere-" "somebodyelse" "/Users/$(printf '\316\250\316\276\317\206')"
rm -f "$PROBE_MD"

# Folders that belong to nobody, and the cast, wherever a sentence ends.
printf 'Shared in /Users/Shared/notes, in ~zeta/mail/INBOX, and in C:\\Users\\Public.\n' > "$PROBE_MD"
allows "folders that belong to nobody"
rm -f "$PROBE_MD"

printf 'Put it in C:\\\\Users\\\\Public and it is nobody.\n' > "$PROBE_MD"
allows "a shared directory that names nobody"
rm -f "$PROBE_MD"

printf '\nA calendar date, or how a thing got this way, outside the tests (invariant 23)\n'
# A date in a comment, in the window's code and in a document; a rule's history and who decided it,
# in a document. Each is a record of how the tree got the way it is, and git holds that. Written in
# pieces, so this file states none of them itself.
printf '// Measured on 2026-%s-04.\npub const PROBE: u8 = 1;\n' 03 > "$PROBE_RS"
printf '// Quicker since 04.03.%s.\nexport const probe = 1;\n' 2026 > "$PROBE_TSX"
{
  printf 'Measured in March %s.\n' 2026
  printf 'This rule used%sto be stricter.\n' ' '
  printf 'The owner%sdecided to keep it.\n' ' '
} > "$PROBE_MD"
refuses_each "a calendar date outside the tests and what a program wrote (AGENTS.md invariant 23) -> src-tauri/src/infrastructure/__gate_probe.rs:1" \
  "a calendar date outside the tests and what a program wrote (AGENTS.md invariant 23) -> src/features/message-reader/__gate_probe.tsx:1" \
  "a calendar date outside the tests and what a program wrote (AGENTS.md invariant 23) -> docs/__gate_probe.md:1" \
  "how a thing got this way, or who decided it (AGENTS.md invariant 23) -> docs/__gate_probe.md:2" \
  "how a thing got this way, or who decided it (AGENTS.md invariant 23) -> docs/__gate_probe.md:3"
rm -f "$PROBE_RS" "$PROBE_TSX" "$PROBE_MD"

# Where a date belongs, and a phrase a document names rather than says: a message in the corpus, a
# Rust test module, and words in quotation marks.
printf 'Subject: probe 2026-%s-04\n\nbody\n' 03 > "$PROBE_EML"
printf 'pub const PROBE: u8 = 1;\n\n#[cfg(test)]\nmod tests {\n    // A message of 2026-%s-04 sorts first.\n}\n' 03 > "$PROBE_RS"
printf 'No "this used%sto be" and no `superseded%sby` here.\n' ' ' ' ' > "$PROBE_MD"
allows "a date in the corpus and in a test module, and a phrase named in quotation marks"
rm -f "$PROBE_EML" "$PROBE_RS" "$PROBE_MD"

printf '\nA file longer than a person reads in one sitting\n'
# Written as code rather than as a test module: a file that is one `#[cfg(test)] mod` is exempt by
# design, so a probe made of tests would prove the exemption and not the rule.
{
  printf 'pub fn a_file_nobody_can_hold_at_once() {\n'
  for _ in $(seq 1 1170); do printf '    let _ = 1;\n'; done
  printf '}\n'
} > "$PROBE_RS"
refuses "over 1171" "1172 lines outside its test modules"
rm -f "$PROBE_RS"

# One line shorter is the limit itself, and the limit is a file's to use: "at most", not "under".
{
  printf 'pub fn a_file_at_the_limit() {\n'
  for _ in $(seq 1 1169); do printf '    let _ = 1;\n'; done
  printf '}\n'
} > "$PROBE_RS"
allows "1171 lines outside its test modules, the most a file may hold"
rm -f "$PROBE_RS"

printf '\nThe same, in the interface\n'
# Not named `.test.tsx`: a file the name says is a test is exempt, and a probe wearing that name
# would prove the exemption rather than the rule.
{
  printf 'export const aScreenNobodyCanHoldAtOnce = () => {\n'
  for _ in $(seq 1 1170); do printf '  const _unused = 1;\n'; done
  printf '};\n'
} > "$PROBE_TSX"
refuses "over 1171" "1172 lines of interface"
rm -f "$PROBE_TSX"

clean
trap - EXIT

printf '\nAnd the tree as it is, which must still pass\n'
if bash scripts/check-layering.sh >/dev/null 2>&1; then
  printf '  boundaries hold\n'
else
  printf '\033[31m  the gate refuses this repository\033[0m\n'
  missed=1
fi

if [ "$missed" -ne 0 ]; then
  printf '\n\033[31mThe gate let something through. A rule it does not enforce is a rule this project does not have.\033[0m\n' >&2
  exit 1
fi

printf '\nEvery prohibition refused what it exists to refuse.\n'
