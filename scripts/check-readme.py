#!/usr/bin/env python3
"""The shop window, held to the rules in AGENTS.md section 7.

Prints one line per violation and exits 0 either way: `check-layering.sh` counts the lines. Only the
rules a script can check are here. The ones marked *by eye* in section 7 — a fact stated once, and
mechanism dressed as a feature — are the reviewer's, and no grep for them stays honest.
"""

import re
import sys
from pathlib import Path

README = Path("README.md")

MOST_LINES = 200
MOST_WORDS = 1600
MOST_LINES_IN_A_SECTION = 25
MOST_WORDS_IN_A_BULLET = 25
MOST_WORDS_IN_A_LEAD = 5

SECTIONS = [
    "What it protects you from",
    "Sponsors",
    "Install and build",
    "Documentation",
    "Contributing",
    "Security",
    "Donations",
    "License",
]

# The one section that is split, and how. Nothing else has a heading under it.
SUBSECTIONS = {"Install and build": ["Install", "Build"]}

# The flag shown for each catalogue, and the English name its place in the line is sorted by.
LANGUAGE_FLAGS = {
    "ar": ("SA", "Arabic"),
    "bn": ("BD", "Bengali"),
    "de": ("DE", "German"),
    "en": ("US", "English"),
    "es": ("ES", "Spanish"),
    "fr": ("FR", "French"),
    "hi": ("IN", "Hindi"),
    "id": ("ID", "Indonesian"),
    "ja": ("JP", "Japanese"),
    "ko": ("KR", "Korean"),
    "pt-BR": ("BR", "Portuguese"),
    "ru": ("RU", "Russian"),
    "th": ("TH", "Thai"),
    "tr": ("TR", "Turkish"),
    "ur": ("PK", "Urdu"),
    "vi": ("VN", "Vietnamese"),
    "zh-Hans": ("CN", "Chinese"),
}
CATALOGUES = Path("src/shared/i18n/locales")

# Words the file does not say. Speed without a measurement, a limit that is really a gap, and the
# self-praise that costs the words a shown fact would have spent better.
REFUSED = {
    "an unmeasured speed claim": r"\b(instant|instantly|snappy|blazing|seamless|effortless|stutter)\b",
    "a gap wearing a limit's clothes": r"\b(yet|for now|currently|today|soon|planned|upcoming)\b|\bwill\s",
    "the first person": r"\b(we|us|our|ours|ourselves)\b",
    "self-praise": r"\b(honest|proven|battle-tested|enterprise|robust|powerful|simply|just)\b",
}

# A sentence that opens by apologising is a caveat, and the row above forbids caveats.
APOLOGY = r"^(Note|However|Unfortunately|Keep in mind|Please note)\b"



def words_in(text):
    """The words of a piece of prose. A dash written closed, word—word, stands between two words."""
    return re.findall(r"[^\s\u2014]+", text)

def fences(lines):
    """Which line numbers sit inside a fenced code block."""
    inside, opened = set(), False
    for at, line in enumerate(lines, 1):
        if line.startswith("```"):
            opened = not opened
            inside.add(at)
            continue
        if opened:
            inside.add(at)
    return inside


def main() -> int:
    if not README.exists():
        print("README.md is missing")
        return 0

    text = README.read_text(encoding="utf-8")
    lines = text.splitlines()
    in_a_fence = fences(lines)
    say = lambda message: print(f"README: {message}")

    if len(lines) > MOST_LINES:
        say(f"{len(lines)} lines, and the shop window holds {MOST_LINES}")
    words = len(words_in(text))
    if words > MOST_WORDS:
        say(f"{words} words, and the shop window holds {MOST_WORDS}")

    headings = [line[3:].strip() for line in lines if line.startswith("## ")]
    if headings != SECTIONS:
        say(f"the sections are not the ones section 7 names, in that order -> {headings}")
    for at, line in enumerate(lines, 1):
        if "<!--" in line:
            say(f"{at}: an HTML comment, which is a placeholder by another name")
        if line.startswith("> [!"):
            say(f"{at}: an admonition, which is a warning box by construction")

    # Subsections, under the sections that have them, and no heading below the fourth level.
    subsections, above = {}, {}
    for at, line in enumerate(lines, 1):
        level = len(line) - len(line.lstrip("#"))
        if at in in_a_fence or level < 2 or not line[level:].startswith(" "):
            continue
        if level > 4:
            say(f"{at}: a heading below the fourth level, and there are none")
            continue
        above[level] = line[level:].strip()
        if level > 2:
            subsections.setdefault(above.get(level - 1), []).append(above[level])
    if subsections != SUBSECTIONS:
        say(f"the subsections are not the ones section 7 names -> {subsections}")

    # Length, from a heading to the next one: a subsection is measured on its own.
    opened_at, opened_name = None, None
    for at, line in enumerate(lines + ["## end"], 1):
        if at in in_a_fence or not line.startswith(("## ", "### ", "#### ")):
            continue
        if opened_at is not None:
            span = at - opened_at
            if span > MOST_LINES_IN_A_SECTION:
                say(f"{opened_at}: '{opened_name}' runs {span} lines, and a section holds {MOST_LINES_IN_A_SECTION}")
        opened_at, opened_name = at, line.lstrip("#").strip()

    # A bullet is one sentence: a short bold lead, then a short sentence, and nothing carrying a
    # second thought. The same bound holds inside a table cell — section 7 says so in as many words
    # ("a table cell is the same sentence with the same bound"), and reading bullets only left six
    # cells over the limit with the gate green.
    def one_sentence(at, body, what):
        lead = re.match(r"\*\*(.+?)\*\*", body)
        if lead:
            if len(words_in(lead.group(1))) > MOST_WORDS_IN_A_LEAD:
                say(f"{at}: the bold lead runs past {MOST_WORDS_IN_A_LEAD} words")
            body = body[lead.end() :]
        if len(words_in(body)) > MOST_WORDS_IN_A_BULLET:
            say(f"{at}: {what} of {len(words_in(body))} words, and one holds {MOST_WORDS_IN_A_BULLET}")
        if ";" in body:
            say(f"{at}: a semicolon in {what}, which is a second sentence wearing a disguise")
        # A full stop with a word after it. Not an abbreviation and not the end: what the rule
        # refuses is the second thought, and the second thought starts with a capital.
        if re.search(r"[a-z)`]\.\s+[A-Z`]", body):
            say(f"{at}: a second sentence in {what}, where the rule allows one")

    for at, line in enumerate(lines, 1):
        if at in in_a_fence:
            continue
        if line.startswith("- "):
            one_sentence(at, line[2:].strip(), "a bullet")
        elif line.startswith("|") and not re.fullmatch(r"\|[\s|:-]+\|", line.strip()):
            for cell in line.strip().strip("|").split("|"):
                cell = cell.strip()
                # A heading row and a one-word cell are not sentences to hold to the rule.
                if len(words_in(cell)) > 1:
                    one_sentence(at, cell, "a table cell")

    # A paragraph is one line. Tables, fences, headings, bullets, links, pictures and markup are not
    # prose, and a run of them is not a paragraph broken in two.
    run = 0
    for at, line in enumerate(lines + [""], 1):
        prose = line.strip() and at not in in_a_fence and not line.startswith(("|", "- ", "#", "[", "!", "<"))
        if prose:
            run += 1
            continue
        if run > 1:
            say(f"{at - run}: a paragraph broken over {run} lines, and a paragraph is one line")
        run = 0

    # Every link inside the repository leads to a file it holds, and into a heading that exists.
    def anchors(path):
        names = re.findall(r"^#{1,6} (.+)$", path.read_text(encoding="utf-8", errors="replace"), re.M)
        plain = (re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", name).strip().lower() for name in names)
        return {re.sub(r"[^\w\- ]", "", name).replace(" ", "-") for name in plain}

    for at, line in enumerate(lines, 1):
        if at in in_a_fence:
            continue
        for target in re.findall(r"\]\(([^)\s]+)\)", line) + re.findall(r'(?:src|srcset|href)="([^"]+)"', line):
            if re.match(r"^[a-z]+:", target):
                continue
            path, _, heading = target.partition("#")
            file = Path(path) if path else README
            if not file.is_file():
                say(f"{at}: {target} leads to nothing the repository holds")
            elif heading and heading not in anchors(file):
                say(f"{at}: {target} names a heading {file} does not have")

    # The line of languages: a flag for each catalogue, in the order of the languages' English names.
    shown = next((line for line in lines if line.startswith("**Available languages:**")), None)
    held = sorted(path.stem for path in CATALOGUES.glob("*.json"))
    unflagged = [code for code in held if code not in LANGUAGE_FLAGS]
    if unflagged:
        say(f"a catalogue with no flag in this checker: {', '.join(unflagged)}")
    elif shown is None:
        say("no line of languages under the screenshots")
    else:
        flag = lambda country: "".join(chr(0x1F1E6 + ord(letter) - ord("A")) for letter in country)
        expected = [flag(LANGUAGE_FLAGS[code][0]) for code in sorted(held, key=lambda code: LANGUAGE_FLAGS[code][1])]
        found = re.findall("[\U0001F1E6-\U0001F1FF]{2}", shown)
        if found != expected:
            say(f"the languages show {' '.join(found)}, and the catalogues call for {' '.join(expected)}")

    # A table of documents or of networks runs in the alphabetical order of what its rows name: a
    # document by its link, a network by its first cell. A table's first row is its heading.
    def tables(of):
        found, run = [], []
        for line in of + [""]:
            if line.startswith("|"):
                run.append(line)
            elif run:
                found.append([row for row in run[1:] if not re.fullmatch(r"\|[\s|:-]+\|", row.strip())])
                run = []
        return found

    def named(rows):
        cells = (row.strip().strip("|").split("|")[0].strip() for row in rows)
        return [re.sub(r"^\[([^\]]+)\]\(.*\)$", r"\1", cell) for cell in cells]

    def section(title):
        inside, body = False, []
        for line in lines:
            if line.startswith("## "):
                inside = line[3:].strip() == title
            elif inside:
                body.append(line)
        return body

    for rows in tables(section("Documentation")):
        if named(rows) != sorted(named(rows), key=str.lower):
            say(f"the documents are not in alphabetical order -> {named(rows)}")

    # The donation page Donations points to: its networks in alphabetical order, and every QR code it
    # shows a file beside it.
    donations = Path("docs/donations/README.md")
    if not donations.is_file():
        say(f"the donation page {donations} is missing")
    else:
        page = donations.read_text(encoding="utf-8").splitlines()
        # The rows are written in HTML, so that a code block, and the copy button GitHub gives it,
        # can stand in a cell: a row's first cell is the network's name.
        networks = re.findall(r"^<tr><td>([^<]+)</td>", "\n".join(page), re.M)
        if not networks:
            say(f"{donations}: no table of networks")
        elif networks != sorted(networks, key=str.lower):
            say(f"{donations}: the networks are not in alphabetical order -> {networks}")
        for picture in re.findall(r'src="([^"]+)"', "\n".join(page)):
            if not (donations.parent / picture).is_file():
                say(f"{donations}: {picture} is not there")

    # One fenced block, and it is the one that gets somebody running.
    opens = sum(1 for line in lines if line.startswith("```"))
    if opens > 2:
        say(f"{opens // 2} code blocks, and the file holds one")

    # The vocabulary, and the numbers.
    docs = "\n".join(
        one.read_text(encoding="utf-8", errors="replace") for one in Path("docs").rglob("*.md")
    )
    for at, line in enumerate(lines, 1):
        if at in in_a_fence:
            continue
        # A tag and its attributes are markup rather than prose: a picture's width is not a figure.
        bare = re.sub(r"`[^`]*`|\[[^\]]*\]\([^)]*\)|https?://\S+|<[^>]*>", " ", line)
        for what, pattern in REFUSED.items():
            found = re.search(pattern, bare, re.I)
            if found:
                say(f"{at}: {what} -> {found.group(0).strip()!r}")
        if re.match(APOLOGY, bare.strip()):
            say(f"{at}: a sentence that opens by apologising")
        # A version is named only in the notes of a release that shipped, never here, and no row
        # marks a step towards one.
        if re.search(r"(?<![\d.])\d+\.\d+\.\d+(?![\d.])", line) or re.search(
            r"(?i)\b(?:lyonoyl mail|version|release)\s+`?v?\d+\.\d+", line
        ) or (
            line.startswith("|") and re.search(r"\*\*\d+\.\d+\*\*", line)
        ):
            say(f"{at}: a version, or a step towards one")
            continue
        # Every figure has a home under docs/.
        for number in re.findall(r"\b\d[\d,.]*\b", bare):
            number = number.rstrip(".")
            if re.fullmatch(r"\d+\.\d+(\.\d+)?", number) or number in {"1", "2", "3"}:
                continue
            if number not in docs:
                say(f"{at}: {number} appears nowhere under docs/, so nobody can check it")

    return 0


if __name__ == "__main__":
    sys.exit(main())
