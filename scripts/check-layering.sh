#!/usr/bin/env bash
# Architecture boundary gate. The compiler does not check dependency direction within a single crate.
# Rules come from AGENTS.md, section 1.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
violation() { echo "LAYERING: $1"; fail=1; }

check() {          # check <directory> <regex of forbidden imports> <explanation>
  local dir="$1" pattern="$2" why="$3"
  [ -d "$dir" ] || return 0

  # Every spelling of an import, and then every use of the name without one.
  #
  # `(pub )?use` caught two of the four ways a line can reach across a boundary. It missed
  # `pub(crate) use`, which this tree already writes four times, and it missed the one that needs no
  # import at all: a fully-qualified path written straight into the body of a function. That last is
  # not a corner case -- a real violation was sitting in it, in shipping code, and the gate said
  # boundaries hold.
  while IFS= read -r hit; do
    violation "$why -> $hit"
  done < <(grep -rnE "^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?use[[:space:]].*($pattern)" "$dir" 2>/dev/null)

  while IFS= read -r hit; do
    violation "$why -> $hit"
  done < <(grep -rnE "($pattern)::" "$dir" 2>/dev/null \
    | grep -vE "^[^:]*:[0-9]+:[[:space:]]*(//|\*)" \
    | grep -vE "^[^:]*:[0-9]+:[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?use[[:space:]]" || true)
}

# The domain lives in its own crate, so the compiler already forbids reaching the outside world:
# a dependency that is not in crates/lyonoyl-domain/Cargo.toml cannot be imported at all.
# This check guards the manifest itself — adding a dependency there is an architectural decision.
if [ -f crates/lyonoyl-domain/Cargo.toml ]; then
  while IFS= read -r dep; do
    case "$dep" in
      thiserror*) ;;
      "") ;;
      *) violation "domain crate gained a dependency: $dep" ;;
    esac
  done < <(awk '/^\[dependencies\]/{f=1;next} /^\[/{f=0} f && NF' crates/lyonoyl-domain/Cargo.toml)
fi

check crates/lyonoyl-domain/src \
  'tauri|rusqlite|serde|reqwest|async_imap|mail_parser|mail_send|keyring' \
  'domain imports the outside world'

# Application knows only ports and the domain.
# By the name of the layer rather than by one way of reaching it. `crate::infrastructure` missed a
# grouped import -- `use crate::{ports::body::Viewing, infrastructure::html::Sanitizer}` puts the
# two apart -- and missed `super::infrastructure::` as well. Nothing under `application/` has a
# reason to write the word followed by `::`, so the word is what is refused.
check src-tauri/src/application \
  'infrastructure|rusqlite|tauri|async_imap|reqwest|keyring' \
  'application imports an adapter'

# Adapter types must not appear in signatures either, not just in imports.
while IFS= read -r hit; do
  violation "adapter type in an application signature -> $hit"
done < <(grep -rnE '(rusqlite|async_imap|reqwest)::' src-tauri/src/application 2>/dev/null)

# Ports must not pull in adapter types.
check src-tauri/src/ports \
  'rusqlite|async_imap|reqwest|tauri' \
  'port depends on an adapter'

# A command is transport: it hands a use case what `bootstrap` chose and builds no adapter of its
# own, so changing an implementation is a change to the composition root and nowhere else. Refused
# by its shape rather than by a list of types: anything reached through `infrastructure`, imported
# or written out in full, followed by a constructor. A test module builds what it tests, and the
# line numbers of what is reported stay those of the file.
python3 - <<'COMMANDS_BUILD_NOTHING' || fail=1
import re, sys, pathlib

def outside_tests(text):
    kept, at = [], 0
    for opened in re.finditer(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", text):
        if opened.start() < at:
            continue
        kept.append(text[at : opened.start()])
        depth, end = 1, opened.end()
        while depth and end < len(text):
            depth += {"{": 1, "}": -1}.get(text[end], 0)
            end += 1
        kept.append("\n" * text[opened.start() : end].count("\n"))
        at = end
    kept.append(text[at:])
    return "".join(kept)

built = False
for path in sorted(pathlib.Path("src-tauri/src/commands").rglob("*.rs")):
    code = outside_tests(path.read_text(encoding="utf-8"))
    reached = set()
    for statement in re.findall(r"\buse\s[^;]*;", code):
        if "infrastructure::" in statement:
            after = statement.split("infrastructure::", 1)[1]
            reached |= set(re.findall(r"\b([A-Z]\w*)\b", after))
    named = r"\b(?:" + "|".join(sorted(reached)) + r")" if reached else r"(?!)"
    constructor = re.compile(
        r"(?:" + named + r"|infrastructure::(?:\w+::)*[A-Z]\w*)::(?:new|of_\w+|open\w*)\s*\("
    )
    for number, line in enumerate(code.splitlines(), 1):
        if constructor.search(line) and not line.lstrip().startswith("//"):
            print(f"LAYERING: a command builds an adapter -> {path}:{number}: {line.strip()}")
            built = True

sys.exit(1 if built else 0)
COMMANDS_BUILD_NOTHING

# read_model reads directly but never writes.
if [ -d src-tauri/src/read_model ]; then
  while IFS= read -r hit; do
    violation "read_model performs a write -> $hit"
  # SQL keywords are written in upper case here, Rust identifiers are not, so `impl Drop for` and
  # `drop(connection)` do not look like statements.
  done < <(grep -rnE '\b(INSERT|UPDATE|DELETE|DROP|CREATE)\b' src-tauri/src/read_model 2>/dev/null)
fi

# Frontend: invoke/listen only inside shared/ipc.
if [ -d src ]; then
  while IFS= read -r hit; do
    case "$hit" in src/shared/ipc/*) continue ;; esac
    violation "@tauri-apps outside src/shared/ipc -> $hit"
  done < <(grep -rln "@tauri-apps/api" src --include='*.ts' --include='*.tsx' 2>/dev/null)
fi

# The frontend map has four directories and no others; a stray one means the layout drifted.
# `shadcn add` recreates src/components/ if components.json is reset, which is how this last happened.
if [ -d src ]; then
  for dir in src/*/; do
    case "$(basename "$dir")" in
      app|entities|features|shared) ;;
      *) violation "directory outside the map in AGENTS.md section 1 -> $dir" ;;
    esac
  done
fi

# A feature does not reach into another feature; shared code moves to shared/ or entities/.
if [ -d src/features ]; then
  for dir in src/features/*/; do
    feature=$(basename "$dir")
    while IFS= read -r hit; do
      case "$hit" in *"@/features/$feature/"*) continue ;; esac
      violation "one feature imports another -> $hit"
    done < <(grep -rn "@/features/" "$dir" 2>/dev/null)
  done
fi

# Interface text: catalogs are reached through the module, never imported directly.
if [ -d src ]; then
  while IFS= read -r hit; do
    case "$hit" in src/shared/i18n/*) continue ;; esac
    violation "locale catalog imported outside src/shared/i18n -> $hit"
  done < <(grep -rln "i18n/locales/" src --include='*.ts' --include='*.tsx' 2>/dev/null)
fi

# No translation library: the hand-rolled module is the decision, not a stopgap.
if [ -f package.json ]; then
  while IFS= read -r dep; do
    violation "translation library added: $dep (a hand-rolled module was chosen)"
  done < <(python3 -c "
import json
manifest = json.load(open('package.json'))
declared = {**manifest.get('dependencies', {}), **manifest.get('devDependencies', {})}
banned = ('i18next', 'react-intl', '@formatjs', '@lingui', 'node-polyglot', 'vue-i18n', 'intl-messageformat')
for name in declared:
    if name.startswith(banned):
        print(name)
")
fi

# Interface text goes through the catalog. A literal written into the markup is text that one
# language gets and no other language ever sees: invisible to the translator and to the
# completeness test. Unknown keys are caught by the compiler, missing translations by
# `catalog.test.ts`; text that never asked for a key is caught only here.
if [ -d src ]; then
  inline_text=$(python3 - <<'FIND_INLINE_TEXT'
import pathlib
import re

LETTER = "A-Za-z\u0400-\u04ff"
# Text between tags on one line, with no interpolation in it: <p>Hello</p>, but not
# <p>{t("greeting")}</p>. The body is words and punctuation only, so that code which happens to sit
# between a ">" and a "<" — a generic type, an arrow function — is not read as a sentence.
# The "=" excludes an arrow function whose return type is generic — `=> Promise<boolean>` put
# "Promise" between a ">" and a "<" and was read as a sentence somebody had written on screen.
TEXT = re.compile(r"(?<![=\-])>[ \t]*([{L}][{L}0-9 ,.!?'’—–…-]*)[ \t]*<".format(L=LETTER))
# Attributes a screen reader announces or a field shows, given a quoted literal. A reader hears
# more than `aria-label`: a described control, a renamed role and a spoken value are all sentences
# somebody wrote, and each is invisible to a person reading the screen with their eyes.
ATTR = re.compile(
    r"\b(aria-label|aria-description|aria-roledescription|aria-valuetext|aria-placeholder"
    r"|placeholder|title|alt|label)=\"([^\"]*[{L}]{{2}}[^\"]*)\"".format(L=LETTER)
)
WORD = re.compile(r"[{L}]{{2}}".format(L=LETTER))
# A sentence handed to the markup from outside it. Text does not become a key by leaving the
# component: a `.ts` file that returns `{ label: "Get new mail" }` is the same untranslated string
# one indirection away, and a scan of `.tsx` alone is a rule with a door next to it.
# Two letter-words with a space between them, so a catalog key — one word, dots, no spaces — and an
# identifier are not mistaken for prose.
PROSE = re.compile(
    r"\b(label|title|message|placeholder|description|hint|tooltip|heading|caption|summary|text|"
    r"name|error)\s*[:=]\s*\"([{L}][^\"]*[ ][^\"]*)\"".format(L=LETTER)
)

for path in sorted(
    [*pathlib.Path("src").rglob("*.tsx"), *pathlib.Path("src").rglob("*.ts")]
):
    name = str(path)
    # Primitives carry no text of their own; tests assert on the rendered result.
    if ".test." in name or "/ui/primitives/" in name:
        continue

    source = path.read_text(encoding="utf-8")
    for number, line in enumerate(source.splitlines(), 1):
        for found in ATTR.finditer(line):
            print("{}:{}: {}=\"{}\"".format(name, number, found.group(1), found.group(2)))
        for found in PROSE.finditer(line):
            if WORD.search(found.group(2)):
                print("{}:{}: {} = \"{}\"".format(
                    name, number, found.group(1), found.group(2)))

    if not name.endswith(".tsx"):
        continue

    for found in TEXT.finditer(source):
        text = found.group(1).strip()
        if not WORD.search(text):
            continue
        number = source.count("\n", 0, found.start()) + 1
        print("{}:{}: {}".format(name, number, text))
FIND_INLINE_TEXT
)
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    violation "text written in place instead of a catalog key -> $hit"
  done <<< "$inline_text"
fi

# Nothing here identifies anybody (invariants 21 and 22): no address that could reach somebody, no
# path holding somebody's home directory, and no commit made under somebody's name. Its own file,
# because reading a repository the way a person reads it -- through every encoding that hides an
# address from anything reading bytes -- is a program rather than a pattern.
python3 scripts/check-anonymity.py || fail=1

# Invariant 23: everything states what is true now. A calendar date stands in a test -- a test file,
# a Rust test module, the message corpus -- in the interface's catalogues, in a file a program wrote
# -- the bindings, the licences, the lock files, the update manifest -- and in the licence itself,
# and nowhere else; and no sentence tells how a thing got the way it is or who decided it. In a
# document, words in quotation marks or in code are named rather than said.
python3 - <<'WHAT_IS_TRUE_NOW' || fail=1
import re
import subprocess
import sys

MONTH = "January|February|March|April|May|June|July|August|September|October|November|December"
DATED = re.compile(r"\b(?:19|20)\d\d-\d\d-\d\d\b|\b\d{1,2}\.\d{1,2}\.(?:19|20)\d\d\b"
                   rf"|\b(?:{MONTH})\s+(?:\d{{1,2}},?\s+)?(?:19|20)\d\d\b|\b\d{{1,2}}\s+(?:{MONTH})\b")
HISTORY = re.compile(r"\bused to be\b|\bsuperseded by\b|\bamended by\b|\bformerly\b|\b(?:we|I) decided\b"
                     r"|\bowner(?:'s)?\s+(?:said|decided|decision|decisions|chose|asked|wanted|agreed)\b",
                     re.IGNORECASE)
NAMED = re.compile(r'"[^"\n]*"|`[^`\n]*`')
TEST_FILE = re.compile(r"(?:^|/)tests?/|\.test\.[jt]sx?$|(?:^|/|_)tests\.rs$")
TEST_MODULE = re.compile(r"^(pub[^ ]* )?mod \w+ \{$")
WRITTEN_BY_A_PROGRAM = re.compile(r"^src/shared/ipc/generated/|^src-tauri/licences"
                                  r"|(?:^|/)(?:Cargo\.lock|package-lock\.json)$|^LICENSE$")
CATALOGUES = "src/shared/i18n/locales/"


def listed(*arguments):
    answer = subprocess.run(["git", *arguments, "-z"], capture_output=True, check=True)
    return [name for name in answer.stdout.split(b"\0") if name]


def outside_tests(path, lines):
    """The lines of a file that are not a test: all of them, but for a Rust test module."""
    skipping = False
    for number, line in enumerate(lines, 1):
        if skipping:
            skipping = line != "}"
            continue
        if path.endswith(".rs") and line.strip() == "#[cfg(test)]" and number < len(lines):
            if TEST_MODULE.match(lines[number]):
                skipping = True
                continue
        yield number, line


failed = False
names = sorted(set(listed("ls-files") + listed("ls-files", "--others", "--exclude-standard")))
for name in names:
    path = name.decode("utf-8", "surrogateescape")
    a_test = TEST_FILE.search(path) is not None
    if a_test or path.startswith(CATALOGUES) or WRITTEN_BY_A_PROGRAM.search(path) is not None:
        continue
    try:
        data = open(name, "rb").read()
    except OSError:
        continue
    if b"\0" in data:
        continue
    document = path.endswith(".md")
    for number, line in outside_tests(path, data.decode("utf-8", "replace").split("\n")):
        if DATED.search(line):
            print(f"LAYERING: a calendar date outside the tests and what a program wrote (AGENTS.md invariant 23) -> {path}:{number}")
            failed = True
        if HISTORY.search(NAMED.sub(" ", line) if document else line):
            print(f"LAYERING: how a thing got this way, or who decided it (AGENTS.md invariant 23) -> {path}:{number}")
            failed = True

sys.exit(1 if failed else 0)
WHAT_IS_TRUE_NOW

# The hand cursor, nowhere. What can be pressed says so by lighting up under the pointer, and it
# says it to everybody -- including somebody driving with a keyboard, who never sees a cursor. The
# hand is a second answer to a question already answered, and it is the answer that stops at the
# mouse. `cursor: default` and a resize handle's own cursor are not this: neither of them claims
# something is pressable.
if [ -d src ]; then
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    violation "the hand cursor is not how this says a thing can be pressed -> $hit"
  # What sets the hand, rather than two words standing near each other. Counting characters between
  # them was wrong in both directions: `cursor: on ? "pointer" : "default"` sets it and walked
  # past, while `cursor.pointer` on an object sets nothing and was refused. What they have in
  # common is an assignment -- a colon in CSS or a style object, an equals through the DOM, or the
  # pair `setProperty` takes -- with `pointer` as the value it lands on.
  #
  # Comments go first, and all of them: the exclusion matched a line that *began* with `//`, so the
  # rule written down in a trailing comment was refused as if it were code.
  done < <(grep -rn '.' src 2>/dev/null \
    | sed 's|//.*||' \
    | grep -E 'cursor-pointer|cursor["'\'']?[[:space:]]*[:=][^;{}]*[[:<:]]pointer[[:>:]]|["'\'']cursor["'\''][[:space:]]*,[^;)]*["'\'']pointer["'\'']' \
    || true)
fi

# The text inside a refusal is English written in Rust. `describeError` turns the code into a
# sentence from the catalog instead, so a person who chose Russian gets Russian; reading `.message`
# walks around that and puts one language on screen for everybody. The same holds for a thrown
# `Error` — its text comes from a library, and no library speaks the language the person picked.
if [ -d src ]; then
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    violation "error text shown instead of a catalog key -> $hit"
  done < <(grep -rnE '\b(err|error|cause|failure|reason)\.message\b' src \
    --include='*.ts' --include='*.tsx' 2>/dev/null | grep -v '\.test\.' || true)
fi

# A catalog is data. Rendering it as markup would turn a translation into an injection point.
# Matches the JSX attribute and the props form, not a mention in prose.

if grep -rnE 'dangerouslySetInnerHTML[[:space:]]*[=:]' src --include='*.tsx' --include='*.ts' >/dev/null 2>&1; then
  violation "forbidden: dangerouslySetInnerHTML (AGENTS.md §3)"
fi

# Reading, everywhere but one file. A mail client that can change the server while its
# synchronisation is still being debugged is one bug away from deleting someone's mail, so in what
# ships, a command that changes a mailbox is sent from `actions.rs` alone, and only two things ask
# it to: the operation log, which makes a change retryable and reversible, and the outbox, filing a
# copy of what it sent.
if [ -d src-tauri/src ]; then
  while IFS= read -r hit; do
    # The guard names every one of these commands: it is what refuses them on the wire, and its
    # tests write each of them at a stream that must not pass them on.
    case "$hit" in
      src-tauri/src/infrastructure/imap/read_only.rs:*) continue ;;
      # The file that sends all five, where it writes the one it sends by hand: `APPEND`, because
      # the client library keeps nothing of a refusal that arrives before the message does. Only
      # that line: the other four are its method calls, and spelled out even here they are refused.
      'src-tauri/src/infrastructure/imap/actions.rs:'*'format!("APPEND '*) continue ;;
      # The file that proves what goes on the wire. It is `#[cfg(test)]`, so none of it ships, and
      # a rule that forbade the commands here would forbid testing the thing it guards.
      src-tauri/src/transcript_tests.rs:*) continue ;;
    esac
    violation "a command that changes mail on the server -> $hit"
  # Quoted and upper case: an IMAP command, not the word "store" in prose or a Rust identifier.
  done < <(grep -rnE '"[^"]*\b(STORE|EXPUNGE|APPEND|UID (MOVE|COPY))\b' src-tauri/src 2>/dev/null)

  # The client library builds the command from a method name, so the rule above — which looks for
  # the command spelled out in a string — sees nothing at all. Without this, a gate that says "no
  # command that changes mail appears here" would be saying it about a file that sends five: the
  # four UID forms, which change the messages they name, and `APPEND`, which adds one.
  #
  # The five belong to one file in what ships: the one the stream guard opens an acting connection
  # for, and a rule below keeps it the only one. Every other command on the guard's list is refused
  # on the wire on every connection, whatever calls it, but for the `SELECT` an acting connection
  # opens its mailbox with — see `read_only.rs` and the tests beside it.
  while IFS= read -r hit; do
    case "$hit" in
      src-tauri/src/infrastructure/imap/actions.rs:*) continue ;;
    esac
    violation "a call that changes mail on the server -> $hit"
  done < <(grep -rnE '\.(uid_store|uid_mv|uid_copy|uid_expunge|expunge)\(' src-tauri/src 2>/dev/null)

  # `append` also names a file opened to add to and a collection joined to another, and each of
  # those is handed one argument where the command takes four: the mailbox, its flags, a date and
  # the message. So a call that closes on the line it opens on, around one argument with no comma or
  # bracket in it, is left alone, and every other is the command — one that rustfmt spreads over
  # several lines as well. Another type's `append` that takes more arguments reads as one too.
  while IFS= read -r hit; do
    case "$hit" in
      src-tauri/src/infrastructure/imap/actions.rs:*) continue ;;
      # Fills a test's own mailbox on the development server with the corpus. It is `#[cfg(test)]`,
      # so none of it ships.
      src-tauri/src/infrastructure/imap/corpus_mailbox.rs:*) continue ;;
    esac
    violation "a call that adds mail on the server -> $hit"
  done < <(grep -rnE '\.append\(' src-tauri/src 2>/dev/null \
    | awk '{ rest = $0; gsub(/\.append\([^,()]*\)/, "", rest); if (rest ~ /\.append\(/) print }')

  # The two rules above read a call as it is plainly written, and a path to the method, a raw
  # identifier or a macro writes it another way. What holds whatever the spelling is the connection
  # it goes out on: the guard lets the five through on one opened to act and on no other. So that is
  # opened in `actions.rs` alone, and in what ships every connection carries the guard, as the rules
  # after this one hold: a call to the five is refused unless it goes out on a connection that file
  # opened, as one in `open_session` or `mailboxes_listed` can. Read by the constructor's path, so
  # the type written out, a turbofish, an imported name and the constructor as a value are one call.
  while IFS= read -r hit; do
    case "$hit" in
      src-tauri/src/infrastructure/imap/actions.rs:*) continue ;;
      # The guard, which defines what a connection is for and tests both kinds.
      src-tauri/src/infrastructure/imap/read_only.rs:*) continue ;;
      # Both `#[cfg(test)]`, so none of either ships: the corpus filler adds mail to a test's own
      # mailbox on the development server, and the transcripts prove what `actions.rs` sends.
      src-tauri/src/infrastructure/imap/corpus_mailbox.rs:*) continue ;;
      src-tauri/src/transcript_tests.rs:*) continue ;;
    esac
    violation "a connection opened to act -> $hit"
  done < <(grep -rnE '::(r#)?acting\b' src-tauri/src 2>/dev/null)

  # A connection is a client of the IMAP library, and the library makes one over whatever stream it
  # is handed. Over a stream the guard never saw, nothing is refused at all — not the five, and not a
  # mailbox deleted. So in what ships a client is made in two files, `mod.rs` for the connection that
  # reads and `actions.rs` for the one that acts, and three rules hold that, read by one script:
  #
  # - Where. The library's client is named by its path in no other file. That is the path every
  #   spelling of the constructor writes: the type with its parameter, in angle brackets or as a raw
  #   identifier, and the constructor handed on as a value.
  # - By its path. Nowhere, those two files included, is it taken in by an import or an alias: under
  #   its own name or another, in a group from the library's root, as everything at the root on its
  #   own or in such a group, with the library or its root under another name, or as a type alias.
  #   Each statement is read to its semicolon, however many lines it takes, and a comment in one is
  #   read as the space it stands for. Written out, the client is what the other two read.
  # - Over the guard. Where one is made, it is made over the guard, written into the call where a
  #   reader sees it: either kind, around the stream the client is handed, read to the closing
  #   bracket however many lines rustfmt spreads the call over. The constructor handed on as a value
  #   shows no stream, and is refused for it.
  #
  # The script runs from a function rather than from inside `<(…)`: the bash macOS ships reads an
  # apostrophe or a lone bracket in a heredoc there as something left open, drops the rule, and the
  # gate goes on to say the boundaries hold. And it is checked, so a script that does not run is a
  # refusal rather than a pass.
  imap_clients() {
    python3 - <<'CLIENTS'
import pathlib
import re

# The client, by its path.
path = re.compile(r"async_imap\s*::\s*(?:r#)?Client\b")
# Where a statement that takes a name in starts: an import, a crate under a name, a type alias. It
# starts a line, so a comment is not one.
statement = re.compile(
    r"^[ \t]*(?:pub(?:\s*\([^)]*\))?\s+)?(?:use|extern\s+crate|type)\s", re.MULTILINE
)
# What in one of them reaches the client: its name, everything at the root on its own or in a group
# from the root, or the library or its root under another name. A glob that opens a group or follows
# a comma in one reads as one at the root however deep the group sits, so everything in one of the
# modules of the library is taken in by its path, as types::* is.
reaches = re.compile(
    r"async_imap[^;]*\b(?:r#)?Client\b"
    r"|async_imap\s*::\s*(?:\{(?:[^;]*[{,])?\s*)?\*"
    r"|\basync_imap\s+as\b"
    r"|async_imap\s*::\s*\{[^;]*\bself\s+as\b"
)
# The constructor, however its type is written out, and the guard of either kind.
made = re.compile(
    r"async_imap\s*::\s*(?:r#)?Client\b(?:\s*::)?\s*(?:<[^()]*>)?\s*::\s*(?:r#)?new\b"
)
guarded = re.compile(r"ReadOnly\s*::\s*(?:<[^()]*>\s*::\s*)?(?:r#)?(?:guarding|acting)\s*\(")


# A statement from where it starts to the semicolon that ends it, however many lines it takes, with
# every comment in it read as the space it stands for, a comment nested in another included: a
# semicolon in one ends nothing, and a name in one takes nothing in.
def read_to_its_end(text, at):
    kept, depth = [], 0
    while at < len(text):
        pair = text[at : at + 2]
        if pair == "/*":
            kept.append(" ")
            depth, at = depth + 1, at + 2
        elif depth and pair == "*/":
            depth, at = depth - 1, at + 2
        elif depth:
            at += 1
        elif pair == "//":
            kept.append(" ")
            end = text.find("\n", at)
            at = end if end >= 0 else len(text)
        else:
            kept.append(text[at])
            at += 1
            if kept[-1] == ";":
                break
    return "".join(kept)


for file in sorted(pathlib.Path("src-tauri/src").rglob("*.rs")):
    text = file.read_text(encoding="utf-8", errors="replace")
    found = {(where.start(), "where") for where in path.finditer(text)}
    for taken in statement.finditer(text):
        if reaches.search(read_to_its_end(text, taken.start())):
            found.add((taken.start(), "imported"))
    for call in made.finditer(text):
        handed = text[call.end() :].lstrip()
        stream = None
        if handed.startswith("("):
            depth = 0
            for end, character in enumerate(handed):
                depth += {"(": 1, ")": -1}.get(character, 0)
                if depth == 0:
                    stream = handed[1:end]
                    break
        if stream is None or not guarded.search(stream):
            found.add((call.start(), "unguarded"))
    lines = text.splitlines()
    for line, rule in sorted({(text.count("\n", 0, at), rule) for at, rule in found}):
        print(f"{rule}\t{file}:{line + 1}:{lines[line]}")
CLIENTS
  }
  clients=$(imap_clients) || violation "the rules on IMAP clients did not run"
  while IFS=$'\t' read -r rule hit; do
    [ -n "$hit" ] || continue
    case "$rule:$hit" in
      where:src-tauri/src/infrastructure/imap/mod.rs:*) continue ;;
      where:src-tauri/src/infrastructure/imap/actions.rs:*) continue ;;
      # All three `#[cfg(test)]`, so none of them ships. The sign-in tests and the corpus filler
      # open their connections over the guard. The transcripts drive scripted streams that stand in
      # for a server: most of them prove what the adapter writes, and the ones that prove the guard
      # as well are wrapped in it.
      where:src-tauri/src/infrastructure/imap/signing_in_tests.rs:*) continue ;;
      where:src-tauri/src/infrastructure/imap/corpus_mailbox.rs:*) continue ;;
      where:src-tauri/src/transcript_tests.rs:*) continue ;;
      unguarded:src-tauri/src/transcript_tests.rs:*) continue ;;
    esac
    case "$rule" in
      where) violation "a client of the IMAP library outside the two files that open a connection -> $hit" ;;
      imported) violation "the IMAP library's client taken in by an import or an alias -> $hit" ;;
      unguarded) violation "a client of the IMAP library over a stream the guard does not read -> $hit" ;;
    esac
  done <<< "$clients"

  # BODY[...] fetches the section and sets \Seen on the message; BODY.PEEK[...] does neither.
  # The difference is one word and it marks a person's mail as read on their server.
  while IFS= read -r hit; do
    violation "a body fetch that would mark mail read -> $hit"
  done < <(grep -rnE '"[^"]*BODY\[' src-tauri/src 2>/dev/null)
fi

# One connection policy, one place that decides what is worth trying again. An adapter reaching for
# the single attempt keeps its own idea of a retry, and two protocols disagreeing about that is how
# a server that is already struggling gets hammered by the client that noticed.
if [ -d src-tauri/src ]; then
  while IFS= read -r hit; do
    case "$hit" in
      src-tauri/src/infrastructure/net/*) continue ;;
    esac
    violation "connect without the retry policy; use connect_retrying -> $hit"
  done < <(grep -rn '\.connect(&' src-tauri/src 2>/dev/null | cut -d: -f1 | sort -u)
fi

# The guard may be exempt from naming the commands, but only while it is the guard: the exemption
# above is worth nothing if the file stops refusing them.
if [ -f src-tauri/src/infrastructure/imap/read_only.rs ]; then
  if ! grep -q 'CHANGES_THE_SERVER' src-tauri/src/infrastructure/imap/read_only.rs; then
    violation "the read-only guard no longer lists the commands it refuses"
  fi
  if ! grep -q 'ReadOnly::guarding' src-tauri/src/infrastructure/imap/mod.rs; then
    violation "the IMAP session is opened without the read-only guard"
  fi
fi

# A session opened without a deadline is a session that can wait for ever. The wrapper is what puts
# one on every command, including the commands nobody has written yet, so opening a client without
# it has to be impossible to do quietly.
if [ -d src-tauri/src ]; then
  while IFS= read -r hit; do
    case "$hit" in
      # The file that proves what goes on the wire drives a scripted stream, which answers at once
      # and has nothing to be silent about.
      src-tauri/src/transcript_tests.rs:*) continue ;;
      *Deadline*) continue ;;
    esac
    violation "a session opened with no deadline on its commands -> $hit"
  done < <(grep -rn "async_imap::Client::new(" src-tauri/src 2>/dev/null)

  if ! grep -q "\.timeout(" src-tauri/src/infrastructure/smtp/mod.rs; then
    violation "the submission client is opened with no deadline on its commands"
  fi
fi

# A secret may be read only where it leaves for its destination: written into the system store, or
# sent to the server that asks for it. Everywhere else it stays wrapped, which is what keeps it out
# of the database, the logs and error messages.
if [ -d src-tauri/src ]; then
  while IFS= read -r hit; do
    case "$hit" in
      src-tauri/src/infrastructure/secure_storage/*) continue ;;
      src-tauri/src/infrastructure/imap/*) continue ;;
      # The other server that asks for it. Sending signs in the same way reading does, and the
      # password goes the same one-way trip: out of the system store, onto the wire, nowhere else.
      src-tauri/src/infrastructure/smtp/*) continue ;;
    esac
    violation "a secret is unwrapped outside the places allowed to send it -> $hit"
  done < <(grep -rln "expose_secret" src-tauri/src 2>/dev/null)
fi

# An account's passwords are written and taken out through the ones held this run, which every pass,
# push, watch, body fetch and send reads. Handed the store alone, adding, changing or removing an
# account writes past them, and the old password goes on being sent -- to whichever server the
# account was just pointed at, or for an account that is gone -- until the application starts again.
# A command is out of reach of every test, so this holds the ones that do it, and it reads text rather
# than types. A function that names one of those use cases, however the path to it is written, makes
# the call `passwords(&state.credentials, &state.secrets)`, written exactly so, and beside that call
# it does not name a field called `secrets` on anything, take the state apart with `AppState {`, make
# a store with a `...Store::` path, or ask `the_store_for` for one. A `//` comment is not read: the
# call named in one is not made. Each function is read on its own, so what it cannot see is what
# another function or item does for it -- a helper in this file or another that hands over a store,
# an alias that names a use case -- and a `//` inside a string, which it takes for a comment and reads
# no further on that line. Those are left to whoever reviews the change. A function is read from its
# signature at the start of a line to its closing brace at the start of a line, which is where
# rustfmt puts both; a test module's functions are indented, and are not read.
if [ -d src-tauri/src/commands ]; then
  while IFS= read -r hit; do
    violation "an account's passwords written past the ones held this run; hand it passwords(&state.credentials, &state.secrets) -> $hit"
  done < <(find src-tauri/src/commands -name '*.rs' -type f | sort | while IFS= read -r file; do
    awk -v file="$file" '
      /^(pub(\([^)]*\))? )?(async )?fn / { inside = 1; start = FNR; signature = $0; body = "" }
      inside { line = $0; sub(/\/\/.*/, "", line); body = body "\n" line }
      inside && /^}/ {
        inside = 0
        if (body !~ /[^A-Za-z0-9_](AddAccount|ChangeAccount|RemoveAccount)[^A-Za-z0-9_]/) next
        rest = body
        gsub(/passwords\(&state\.credentials, &state\.secrets\)/, "", rest)
        if (rest == body || rest ~ /\.secrets|AppState *\{|Store::|the_store_for/) print file ":" start ": " signature
      }
    ' "$file"
  done)
fi

# Security: prohibitions that must never be violated.
#
# By what they do rather than by how they are spelled: `initialization_script_for_all_frames`,
# `initialization_script_on_all_frames` (tauri-runtime) and `js_init_script_on_all_frames` (the
# plugin builder) are three names for one capability, injecting into the frame holding a message.
# Nothing here legitimately runs anything in every frame, so the capability is refused by any name.
while IFS= read -r hit; do
  violation "forbidden permanently: a script injected into every frame -> $hit"
done < <(grep -rn 'all_frames' src-tauri/src 2>/dev/null)
# The one call the compiler cannot check. AppKit changes the Dock's picture only through a call Rust
# marks `unsafe`, and one file holds it; `unsafe` as Rust spells it — a block, a function, an
# implementation, a trait, an `extern`, an attribute, or the lint allowing any of them — is refused
# in every other Rust file. A word inside a string, like a content policy's 'unsafe-inline', is not.
while IFS= read -r hit; do
  violation "unsafe outside the Dock's picture -> $hit"
done < <(grep -rnE '\bunsafe[[:space:]]*(\{|fn[[:space:]]|impl[[:space:]<]|trait[[:space:]]|extern[[:space:]]|\()|unsafe_code' \
  --include='*.rs' src-tauri/src crates 2>/dev/null |
  grep -v '^src-tauri/src/infrastructure/app_icon/dock\.rs:')
# The frame a message is shown in. There is exactly one in this application, its attributes are
# fixed in the markup, and a test asserts them — so the thing to guard is not the spelling of an
# attribute but the appearance of a second frame, or of the words that would weaken the one there
# is. `srcdoc` and `allow-*` were matched as `srcdoc[[:space:]]*=` and lines containing `sandbox=`,
# and `frame["srcdoc"] = html` and `setAttribute(name, html)` walked past both.
#
# Naming either in a test that asserts it is absent is the opposite of a violation, so tests are
# left out — and the globs now include the JavaScript extensions, because a `.jsx` file carrying an
# iframe matched nothing at all.
SOURCE_GLOBS=(--include='*.ts' --include='*.tsx' --include='*.js' --include='*.jsx' --include='*.mjs' --include='*.cjs')

not_a_test() { grep -vE '\.(test|spec)\.[jt]sx?:'; }

while IFS= read -r hit; do
  violation "forbidden permanently: srcdoc for message bodies -> $hit"
done < <(grep -rn 'srcdoc' src "${SOURCE_GLOBS[@]}" 2>/dev/null | not_a_test)

while IFS= read -r hit; do
  violation "forbidden permanently: allow-* in a sandbox attribute -> $hit"
done < <(grep -rnE 'allow-(scripts|same-origin|popups|forms|modals|top-navigation|downloads|pointer-lock|presentation|orientation-lock)' src "${SOURCE_GLOBS[@]}" 2>/dev/null | not_a_test)

# One frame, and it is the message reader's. A second one is what any way past the rules above
# would need, whatever it called its attributes.
frames=$(grep -rln '<iframe' src "${SOURCE_GLOBS[@]}" 2>/dev/null | grep -vE '\.(test|spec)\.[jt]sx?$' || true)
if [ "$frames" != "src/features/message-reader/MessageBody.tsx" ]; then
  violation "the message frame is not the only iframe in the interface -> ${frames:-none found}"
fi

# Invariant 8: untrusted HTML is never parsed in the window that holds IPC. `dangerouslySetInnerHTML`
# was the only spelling refused, and it is the one nobody reaches for — these need no dependency and
# no React.
while IFS= read -r hit; do
  violation "forbidden permanently: parsing HTML in the main window (invariant 8) -> $hit"
done < <(grep -rnE '\b(DOMParser|insertAdjacentHTML|createContextualFragment|document\.write)\b|\.(inner|outer)HTML[[:space:]]*=' src "${SOURCE_GLOBS[@]}" 2>/dev/null | not_a_test)

# Untrusted HTML is never parsed in the window that holds IPC. For a sanitizer written in JavaScript
# to clean anything, the attacker's markup has to be parsed into the DOM of the privileged window
# first — by the very parser whose bypass is the attack.
if grep -rniE 'dompurify|sanitize-html|xss-filters' src package.json >/dev/null 2>&1; then
  violation "forbidden permanently: an HTML sanitizer in the frontend"
fi

# The panic strategy is a decision, and cargo reads it in exactly one place. A profile in
# a member manifest is ignored — silently, apart from a warning nobody reads — so this checks where
# it actually counts rather than where it looks right.
python3 - <<'PANIC' || fail=1
import re, sys, pathlib

root = pathlib.Path("Cargo.toml").read_text()
profile = re.search(r"\[profile\.release\](.*?)(\n\[|\Z)", root, re.S)

if not profile or 'panic = "unwind"' not in profile.group(1):
    print('LAYERING: [profile.release] panic = "unwind" is not in the workspace root')
    sys.exit(1)

if 'panic = "abort"' in root:
    print('LAYERING: panic = "abort" is forbidden')
    sys.exit(1)

for manifest in sorted(pathlib.Path(".").glob("*/Cargo.toml")) + sorted(
    pathlib.Path(".").glob("*/*/Cargo.toml")
):
    if "target" in manifest.parts or "node_modules" in manifest.parts:
        continue
    if "[profile." in manifest.read_text():
        print(f"LAYERING: {manifest} carries a profile, and cargo ignores it")
        sys.exit(1)
PANIC

# What the window itself is allowed to do, which is a boundary no content policy covers.
#
# A capability granted to the window is a thing the window can do without asking this application —
# so an opener permission is an outbound channel a compromised renderer reaches with no bypass and
# no CSP directive involved. Links in a message are opened by our own command, from Rust, where a
# window's permissions do not reach, so the window needs none of these.
if [ -d src-tauri/capabilities ]; then
  while IFS= read -r hit; do
    violation "the window is granted a way out of the application it does not use -> $hit"
  done < <(grep -rnE '"(opener|shell|fs|http|process|dialog|updater|notification):' src-tauri/capabilities 2>/dev/null)

  # The window never raises a notification. What is shown is decided in Rust, from a count the
  # window never sees, and the sentence it hands over is checked before it is kept — so a window
  # that could call the plugin directly would be a way to put arbitrary text on a locked screen
  # with none of that in the way.
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    violation "the window raising its own notification -> $hit"
  done < <(grep -rnE "plugin-notification|sendNotification|isPermissionGranted|requestPermission" src package.json 2>/dev/null)
fi

# The content security policy, by what it permits rather than by what it spells out.
#
# An absent directive is not an absent rule: it falls back to `default-src`, so a policy with no
# `img-src` and `default-src 'self' https:` permits exactly what naming `img-src https:` would.
# `csp: ""` and `csp: {}` are not null either. And `devCsp` — the one a person edits to get a
# message to render locally, and then copies across — is read like the other.
# Where the bundle configuration is. The real one unless somebody says otherwise, and the only
# thing that says otherwise is the file that proves these checks bite. It plants its violations in
# a copy: planted in the real configuration, a weakened policy would sit on disk for as long as a
# run lasted, and stay there after a run killed outright.
CONFIG="${LYONOYL_BUNDLE_CONFIG:-src-tauri/tauri.conf.json}"

if [ -f "$CONFIG" ]; then
  CONFIG="$CONFIG" python3 - <<'CSP' || fail=1
import json, sys

REACHES_THE_NETWORK = ("https:", "http:", "*")

def resolved(policy, directive, fallback="default-src"):
    """What a browser applies for this directive, following the fallback when it is absent."""
    parts = {}
    for piece in policy.split(";"):
        piece = piece.strip()
        if not piece:
            continue
        name, _, values = piece.partition(" ")
        parts[name.strip()] = values.strip()

    return parts.get(directive, parts.get(fallback))

def written(csp):
    """A policy as one string, whether it was written as a string or as an object."""
    if isinstance(csp, str):
        return csp
    if isinstance(csp, dict):
        return "; ".join(f"{name} {values}" for name, values in csp.items())
    return None

bad = False

def refuse(message):
    global bad
    print(f"LAYERING: {message}")
    bad = True

import os

security = json.load(open(os.environ["CONFIG"])).get("app", {}).get("security", {})

# Both of them. `devCsp` governs the window a developer looks at, which is the window a relaxation
# is made in and the one it gets copied out of.
for name in ("csp", "devCsp"):
    if name not in security:
        continue

    policy = written(security[name])
    if policy is None:
        refuse(f"{name} is null in tauri.conf.json")
        continue
    if not policy.strip():
        refuse(f"{name} is empty, which permits everything")
        continue
    if resolved(policy, "default-src") is None:
        refuse(f"{name} has no default-src, so every directive it omits permits everything")
        continue

    # Remote pictures are fetched by this application, never by the webview: a message that can
    # reach the network is a message that learns it was opened, whatever the frame around it says.
    images = (resolved(policy, "img-src") or "").split()
    if any(source in images for source in REACHES_THE_NETWORK):
        refuse(f"{name} permits remote images: img-src resolves to '{' '.join(images)}'")

    # The frame holding a message is on its own scheme, never on the application's own origin:
    # `frame-src 'self'` would let a message be framed as part of the window that holds IPC.
    frames = (resolved(policy, "frame-src") or "").split()
    if "'self'" in frames or any(source in frames for source in REACHES_THE_NETWORK):
        refuse(f"{name} permits framing from 'self' or the network: frame-src resolves to '{' '.join(frames)}'")

    # Nothing that turns text into code in the window that holds IPC.
    scripts = (resolved(policy, "script-src") or "").split()
    for forbidden in ("'unsafe-inline'", "'unsafe-eval'"):
        if forbidden in scripts:
            refuse(f"{name} permits {forbidden} in script-src")

sys.exit(1 if bad else 0)
CSP
fi

# A file path never arrives from the frontend (invariant 12). The webview is where hostile input
# lives every day, so a command that accepts a path is a command that writes wherever the person
# can. What a command takes is what a message is — an identifier — and the native dialog in Rust
# decides the rest. This reads the parameters of every #[tauri::command] and refuses a path among
# them; the dialog adapter is the one place allowed to know what a path is.
python3 - <<'PATHS' || fail=1
import pathlib, re, sys

# A parameter is a path if it is named like one or typed like one. Both halves matter: `path: String`
# hides the type, and `where_to: PathBuf` hides the name.
NAMED = re.compile(r"^(path|file_path|directory|folder|destination|target_path|to)$")
TYPED = re.compile(r"\bPath\b|\bPathBuf\b|\bOsString\b")

failed = False

for source in sorted(pathlib.Path("src-tauri/src/commands").rglob("*.rs")):
    text = source.read_text()

    for command in re.finditer(r"#\[tauri::command\][^\n]*\n(?:[^\n]*\n)*?\s*(?:pub )?(?:async )?fn\s+(\w+)\s*\(([^)]*)\)", text):
        name, parameters = command.group(1), command.group(2)

        for parameter in parameters.split(","):
            if ":" not in parameter:
                continue
            argument, kind = parameter.split(":", 1)
            argument = argument.strip().removeprefix("mut ").strip()

            if NAMED.match(argument) or TYPED.search(kind):
                print(f"LAYERING: command {name} takes a path: {parameter.strip()} (invariant 12)")
                failed = True

sys.exit(1 if failed else 0)
PATHS

# Any transaction that may write opens as BEGIN IMMEDIATE (invariant 6).
#
# Deferred is the default, and it takes a read lock first and asks to upgrade at the first write.
# Behind another writer that upgrade is refused outright rather than waiting, because waiting there
# is how two transactions deadlock — so the busy timeout, which exists precisely so a second writer
# waits its turn, never gets a chance to help. What a person sees is "the database is unavailable"
# for marking a message read, whenever a background pass happened to be writing.
#
# It stays a rule rather than a judgement call about each transaction: whether a given one reads
# before it writes is exactly the sort of thing that changes under a later edit, quietly.
#
# read_model is the exception, and a deliberate one: it may only SELECT — a rule this same script
# enforces a few lines up — and its transactions exist to read a consistent snapshot. Taking a
# write lock to read would block every writer for the length of a list query.
while IFS= read -r hit; do
  case "$hit" in
    src-tauri/src/read_model/*) continue ;;
  esac
  violation "deferred transaction; use transaction_with_behavior(Immediate) -> $hit"
done < <(grep -rn '\.transaction()' src-tauri/src 2>/dev/null)

# And the file dialog is reached from exactly one place, so what it is can be changed without
# hunting for callers.
while IFS= read -r hit; do
  case "$hit" in
    src-tauri/src/infrastructure/dialog/*) continue ;;
  esac
  violation "rfd used outside the dialog adapter -> $hit"
done < <(grep -rln "rfd::" src-tauri/src 2>/dev/null)

# A PowerShell script is run on Windows, and Windows PowerShell 5.1 reads a file with no byte order
# mark as ANSI rather than UTF-8. An em dash decoded that way ends in a right double quote, which
# PowerShell accepts as a string delimiter: one of them unbalances every quote below it and the
# parse error is reported dozens of lines away from the character that caused it. ASCII decodes the
# same either way, so ASCII is the rule. cmd.exe has the same problem in a different alphabet, so
# the rule covers a batch file too.
# awk rather than grep -P: this runs on macOS too, where grep has no -P and would have reported
# nothing at all while looking like it had checked.
while IFS= read -r script; do
  where=$(LC_ALL=C awk '/[\200-\377]/ { print NR; exit }' "$script")
  if [ -n "$where" ]; then
    violation "non-ASCII byte on line $where; Windows will misread it -> $script"
  fi
done < <(find scripts \( -name '*.ps1' -o -name '*.cmd' -o -name '*.bat' \) -type f)


# One file, one subject, and short enough to find a thing in without already knowing it is there.
# Test modules do not count: a large one sits at the bottom where nobody scrolls through it, and it
# moves to a sibling file the moment it is in the way. Comments do count — they are the design
# record here, and a measure that discounted them would pay for deleting them.
LONGEST=1171

# A file whose length is a list, scanned one entry at a time rather than held at once. Position in
# `index_migrations`'s vec IS the schema version, so it cannot be cut and cannot be composed.
# An entry that has fallen under the limit is a violation: an exemption does not outlive its reason.
# The provider table is the same shape: one line per domain, read by
# eye only when a diff moves a line. It cannot be cut, because half a table answers half the people,
# and it cannot be composed, because a domain is either in it or it is not.
LEDGERS="src-tauri/src/infrastructure/sqlite/migrations/mod.rs crates/lyonoyl-domain/src/provider/table.rs"

# Physical lines with every `#[cfg(test)] mod … { … }` block removed. The closing brace is matched
# at column 0 rather than by counting braces: a top-level module's is always there after rustfmt,
# and counting would need to know which braces are inside a string. This deliberately does not fire
# on `#[cfg(test)]` in front of anything else — cutting at the first attribute instead reported a
# 3775-line file as 479 lines of code.
lines_outside_tests() {
  awk '
    skip     { if ($0 == "}") skip = 0; next }
    pending  { pending = 0
               if ($0 ~ /^(pub[^ ]* )?mod [A-Za-z0-9_]+ \{/) { skip = 1; next }
               code += 2; next }
    $0 == "#[cfg(test)]" { pending = 1; next }
               { code++ }
    END      { print code + pending }
  ' "$1"
}

# Modules compiled only into a test build. The whole file is a test module, so measuring it would be
# measuring tests. Found rather than listed, so a new one needs no edit here.
test_only=$(find src-tauri/src crates -name '*.rs' | while read -r file; do
  awk -v dir="$(dirname "$file")" '
    /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { armed = 1; next }
    armed && /^[[:space:]]*(pub[^ ]* )?mod [A-Za-z0-9_]+;/ {
      name = $0; sub(/;.*/, "", name); sub(/.* /, "", name)
      print dir "/" name ".rs"; print dir "/" name "/"
    }
    { armed = 0 }
  ' "$file"
done | sort -u)

while IFS= read -r file; do
  skip=0
  while IFS= read -r only; do
    case "$file" in "$only"|"$only"*) skip=1 ;; esac
  done <<< "$test_only"
  [ "$skip" -eq 1 ] && continue

  size=$(lines_outside_tests "$file")
  case " $LEDGERS " in
    *" $file "*)
      if [ "$size" -le "$LONGEST" ]; then
        violation "listed as a ledger but is only $size lines; take it off the list -> $file"
      fi
      continue ;;
  esac
  if [ "$size" -gt "$LONGEST" ]; then
    violation "$size lines outside its test modules, over $LONGEST (AGENTS.md section 6) -> $file"
  fi
done < <(find src-tauri/src crates -name '*.rs' | sort)

# The same limit on the frontend, where a test lives in its own file rather than in a module: the
# name says which, so there is nothing to strip and the whole file is counted.
while IFS= read -r file; do
  size=$(wc -l < "$file")
  if [ "$size" -gt "$LONGEST" ]; then
    violation "$size lines, over $LONGEST (AGENTS.md section 6) -> $file"
  fi
done < <(find src -name '*.ts' -o -name '*.tsx' \
  | grep -v -e '\.test\.' -e '\.spec\.' -e '/generated/' | sort)

# Nothing in this repository identifies anybody (AGENTS.md invariant 21). Most of that rule is about
# prose and is caught by reading; these three are the ways an identity gets in by machinery, and a
# machine can refuse them. This covers the shapes whose content is bytes rather than a pattern, and
# the two config fields that name a signer.

# A certificate, a private key or a provisioning profile, tracked. `.gitignore` keeps these out of
# `git status`; a `git add -f` walks past that, and this does not.
while IFS= read -r file; do
  [ -n "$file" ] || continue
  violation "an identity file does not belong in this repository -> $file"
done < <(git ls-files \
  '*.p12' '*.pfx' '*.cer' '*.crt' '*.der' '*.pem' '*.key' '*.p8' \
  '*.mobileprovision' '*.provisionprofile' '*.certSigningRequest' '*.keychain' '*.keychain-db' \
  2>/dev/null || true)

# `authors` in a manifest names whoever wrote it, and ships that name in the crate metadata. The
# field is optional; the repository does without it.
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  violation "a manifest naming its authors -> $hit"
done < <(git ls-files '*Cargo.toml' -z 2>/dev/null | xargs -0 grep -nE '^ *authors *=' 2>/dev/null || true)

# A signing identity written into the bundle configuration. It is read from the environment of
# whoever builds instead, so a plain checkout signs with nothing and names nobody.
#
# Every platform has its own way of asking for one, and each of these is an identity by the letter of
# invariant 21. `certificateThumbprint` is a fingerprint. `signCommand` carries whatever a cloud
# signing service is addressed by -- an endpoint that encodes a region, an account, a certificate
# profile. `publisher` is the name that goes in a Debian Maintainer field and an installer
# Manufacturer property; left out, both fall back to the product name and stay anonymous.
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  violation "a signing identity in the bundle configuration -> $hit"
done < <(grep -rnE '"(signingIdentity|providerShortName|teamId|appleId|certificateThumbprint|signCommand|publisher)"' \
  "${LYONOYL_BUNDLE_CONFIG:-src-tauri/tauri.conf.json}" 2>/dev/null || true)

# The shop window, held to section 7. The rules a script can check are checked here.
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  violation "$hit"
done < <(python3 scripts/check-readme.py 2>&1 || echo "README: its own checker will not run, and a gate that cannot run passes everything")

if [ "$fail" -ne 0 ]; then
  echo
  echo "Boundaries violated. Rules: AGENTS.md, section 1."
  exit 1
fi
echo "Boundaries hold."
