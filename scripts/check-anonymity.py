#!/usr/bin/env python3
"""Nothing in this repository identifies anybody: AGENTS.md, invariants 21 and 22.

Refused are shapes, not names: an address that could reach somebody, somebody at a machine, a path
holding somebody's home directory, a signing identity, an identifier a real system minted for
somebody -- and a commit about to be made under somebody's name, or signed by them. Shapes, because
a list of names holds only the ones somebody already found, and writing a real name into a gate to
refuse it is putting it in the repository.

What is read is everything this repository holds, committed or about to be: tracked files, untracked
ones that are not ignored, the copy staged for the next commit where it differs from both the disk
and the last commit, what a link points at rather than what it leads to, and every file's own name.
A file refused only once it is in the history is refused too late for the one rule where the history
cannot be taken back.

Each file is read the way a person or a mail client would read it, and not only byte for byte:
through string-literal escapes, HTML entities and characters that look like others, terminal colour
codes, percent-encoding, quoted-printable, base64 and encoded words, and a message kept as a fixture
part by part -- one inside another, to a bounded depth. Each of those hides an address from anything
that reads bytes, and none of them takes anybody meaning to hide one: a link to unsubscribe carries
its recipient percent-encoded, a bounce address carries the recipient inside its own name, and a
message pasted into a test carries its body in base64 inside a string.

Each finding is judged on its own. Judged by the line, one that may stand there would excuse every
other beside it.

What no reading reveals is a reviewer's: an address put together in code from pieces, and words
inside a file that is not text beyond its longer printable runs -- a compressed stream, an archive.
A picture or a recording is read only for those runs, which is where a camera or an editor writes a
name; read whole, its random bytes spell an address or a home now and then.

A clone that commits under the product's name commits for the project, at the address the product
has committed under before, or LYONOYL_COMMIT_ADDRESS where that is set outside the tree as invariant
21 asks of anything real. Any other name is a contributor's own, and is not asked about.

The gate's own three files are outside this, and they have to be: they hold the shapes, and what is
planted to prove the shapes bite. Neither the shapes nor the probes name anybody all the same.
"""

import base64
import binascii
import email
import email.policy
import html
import os
import quopri
import re
import subprocess
import sys
import unicodedata
import urllib.parse
from email.header import decode_header, make_header

GATE = {
    "scripts/check-anonymity.py",
    "scripts/check-layering.sh",
    "scripts/check-layering.test.sh",
}

# The licence lists, which are exempt from the address rule and the only exemption: a permissive
# licence asks for exactly one thing, that the notice travels with the code, and a notice names the
# people who wrote it. Removing them would be failing the one condition under which this product may
# be sold at all. Named whole, so a file that only begins like one of them is not one.
LICENCES = re.compile(
    r"src-tauri/licences(?:-(?:npm|rust|vendored))?\.json|src-tauri/licences-copied/.+"
)

# Files that are messages, read part by part as a mail client reads them.
MESSAGES = (".eml", ".emlx", ".mbox", ".mime")

# How deep one encoding inside another is followed.
DEEPEST = 3

# --------------------------------------------------------------------------------------------------
# What is refused.

# An address as it is usually written, and one written partly outside ASCII: a name in another
# script, or a domain under one. The second is judged only where it holds what the first cannot see.
# Each starts where a name can start, so a long run of letters is not tried at every one of them.
ADDRESS = re.compile(r"(?<![A-Za-z0-9._%+-])[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
WIDE = re.compile(
    r"(?<![\w.%+-])[\w.%+-]+@(?:[\w-]+\.)+"
    r"(?:[A-Za-z]{2,}(?![^\W\d_])|xn--[A-Za-z0-9-]+|[^\W\d_A-Za-z]{2,})"
)

# Somebody at a machine, with no dot to make it an address: a shell's prompt as each system draws it
# by default -- zsh on macOS, bash on Debian and on Fedora, Git Bash on Windows -- and where ssh, scp
# and their kind are pointed.
PROMPTS = [
    re.compile(
        r"(?m)^[ \t>]*([\w.-]+)@([\w-]+)(?::[^\s$#%]*)?[ \t]+[^\s%$#]+[ \t]*[%$#](?=[ \t]|$)"
    ),
    re.compile(r"(?m)^[ \t>]*([\w.-]+)@([\w-]+):[~/][^\s]*?[$#](?=[ \t]|$)"),
    re.compile(r"(?m)^[ \t>]*\[([\w.-]+)@([\w-]+)[ \t][^\]\n]*\][$#]"),
    re.compile(r"(?m)^[ \t>]*([\w.-]+)@([\w-]+)[ \t]+(?:MINGW(?:32|64)|MSYS|UCRT64|CLANG64)\b"),
]
REMOTE = re.compile(
    r"\b(?:ssh|scp|sftp|rsync|mosh|ssh-copy-id)\b[^\n]*?[\s:]([A-Za-z0-9._-]+)@([A-Za-z0-9.-]+)"
)
# What every prompt above begins with, looked for once before the four of them are.
PROMPT_START = re.compile(r"(?m)^[ \t>]*\[?[\w.-]+@[\w-]+[ \t:]")

# A home directory however a system or a person writes it: /Users/name and /home/name where they
# start a path rather than sit inside one or inside a web address, as file:// writes them too, and
# in capitals or small letters; Git Bash's /c/Users/name; C:\Users\name with any drive, written as
# C:, as c$ in an administrative share or as %SystemDrive%, with one backslash or several or a
# forward slash, and the older Documents and Settings; \home\name inside a WSL share; /homes/name on
# a network server; Users\name\AppData with no root at all; ~name/ as a shell and an IMAP server
# write it; and -Users-name- as agent tools name a folder after the path of a project.
HOME = re.compile(
    r"(?:"
    r"(?:(?<![\w.:/\\-])|(?<=file://))/(?i:users)/"
    r"|(?:(?<![\w.:/\\-])|(?<=file://))/homes?/"
    r"|/[A-Za-z]/(?i:users)/"
    r"|\\\\wsl(?:\$|\.localhost)\\[^\\\s]+\\home\\"
    r"|(?:[A-Za-z]:|[A-Za-z]\$|%\w+%)(?:\\{1,4}|/)(?i:users|documents and settings)(?:\\{1,4}|/)"
    r"|(?<![\w.:\\-])\\{1,4}(?i:users)\\{1,4}"
    r"|(?<![\w.:/\\-])(?i:users)[\\/]{1,2}"
    r"(?=[\w.-]+[\\/]{1,2}(?:AppData|Library|Desktop|Documents|Downloads)\b)"
    r")([\w.-]+)"
)
TILDE = re.compile(r"(?<![\w~%$\\])~([A-Za-z_][\w.-]{2,})(?=[/\\][\w.-])")
DASHED = re.compile(r"(?<![A-Za-z0-9-])(?:[A-Za-z]-)?-(?:Users|home)-([A-Za-z0-9_]+)-")

# A signing identity as the tools print it and their settings hold it -- the certificate's name, the
# team it belongs to, and the keys App Store Connect issues -- and identifiers a real system minted
# for somebody: a Windows account's SID, a Microsoft tenant, an Exchange recipient's directory name.
# Invariant 22 asks for one of the same shape that counts from zero, and that is what may stand.
# A certificate's kind, and its holder's name: with the team in brackets after it, as the tools print
# it, or in quotes on its own, as a setting holds it -- never a sentence that only mentions the kind.
CERTIFICATE_KIND = (
    r"(?:Developer ID (?:Application|Installer)|Apple (?:Development|Distribution)|Mac Developer"
    r"|3rd Party Mac Developer (?:Application|Installer)|iPhone (?:Developer|Distribution))"
)
CERTIFICATE_NAME = r"([A-Za-z<][^\n()\"'`]*?)"
CERTIFICATE = re.compile(
    CERTIFICATE_KIND + r":[ \t]*" + CERTIFICATE_NAME + r"[ \t]*\(([A-Z0-9]{10})\)"
    r"|[\"']" + CERTIFICATE_KIND + r":[ \t]*" + CERTIFICATE_NAME + r"[ \t]*[\"']"
)
TEAM = re.compile(
    r"(?i)(?:TeamIdentifier|(?:APPLE_)?TEAM_?ID|DEVELOPMENT_TEAM|--team-id|--key-id"
    r"|APPLE_API_KEY(?:_ID)?)[ \t]*[=: ][ \t]*[\"']?([A-Z0-9]{10})\b"
)
SID = re.compile(r"S-1-5-21-(\d+)-(\d+)-(\d+)")
GUID = r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
TENANT = re.compile(
    r"(?i)(?:Tenant-?Id|--issuer|APPLE_API_ISSUER)[^\n:=]*[:= ][ \t\"']*(" + GUID + ")"
)
DIRECTORY_NAME = re.compile(r"(?i)/CN=RECIPIENTS/CN=([^\s/>\"'<]+)")

# --------------------------------------------------------------------------------------------------
# What may stand.

# What nothing answers for: example.com, example.net and example.org and anything under them, a name
# under .test, .invalid or .example, and lyonoyl.local, the development mail server's own. Matched to
# the end, so a domain that only begins like one of these is a domain somebody may own.
NOBODYS_DOMAIN = re.compile(
    r"(?:[\w-]+\.)*example\.(?:com|net|org)|(?:[\w-]+\.)+(?:test|invalid|example)|lyonoyl\.local",
    re.IGNORECASE,
)
# The scenes the build made for screenshots plays, and the one address they may hold beyond the
# reserved ones: an account at the product's own domain, which is what a screenshot shows. In a
# scene's own file and nowhere else, and that domain whole -- not one under it, and not one that
# only begins like it.
SCENE = re.compile(r"mocks/screenshots/[^/]+/scene\.json")
PRODUCTS_DOMAIN = "lyonoyl.com"
# The product's own mailbox for sponsors, in the one file that gives it: the README says where a
# sponsor writes. The address whole, so nothing else at the product's domain stands there with it.
SPONSORS = ("README.md", "patron@lyonoyl.com")
# A machine that is nobody's: this one, and the addresses set aside for documentation.
NOBODYS_MACHINE = re.compile(
    r"localhost|127(?:\.\d{1,3}){3}|192\.0\.2\.\d{1,3}|198\.51\.100\.\d{1,3}|203\.0\.113\.\d{1,3}"
)
# What follows an @ in a package's name is its version or its tag, not a machine.
VERSION = re.compile(r"\d+(?:\.\d+){0,2}|latest|next|beta|alpha|canary|rc|nightly|stable")

# The cast, at any domain and in the Greek its names come from, with the surname the scheme
# gives it and with a tag after a plus: a test about recognising a provider cannot be written against
# example.com. Nothing else stands at a domain that answers, generic words included -- a word this
# repository did not build names whoever holds it there.
CAST = re.compile(
    "zeta|kappa|sigma"
    "|\u03b6[\u03ae\u03b7]\u03c4\u03b1|\u03ba[\u03ac\u03b1]\u03c0\u03c0\u03b1|\u03c3[\u03af\u03b9]\u03b3\u03bc\u03b1",
    re.IGNORECASE,
)
CAST_RECIPIENT = re.compile(
    r"(?:" + CAST.pattern + r")(?:[._-](?:quorin|vandel|ordrin))?(?:\+[\w.-]*)?", re.IGNORECASE
)

# Shaped like an address and not one: a picture's density name, an algorithm as the SSH protocol
# spells it, the user a forge's SSH transport signs in as, and the address Microsoft publishes in
# the example the XOAUTH2 test checks its bytes against -- a test that invented one would be checking
# this build against itself.
PICTURE = re.compile(r"[0-9]+(?:\.[0-9]+)?x\.(?:png|jpe?g|webp|gif|avif|svg)", re.IGNORECASE)
ALGORITHM = re.compile(r"[a-z0-9]+(?:-[a-z0-9.]+)+")
ALGORITHMS_AT = {"openssh.com", "libssh.org"}
PUBLISHED = {("crates/lyonoyl-domain/src/xoauth2/tests.rs", "test@contoso.onmicrosoft.com")}

# Folders, and the accounts behind them, that belong to nobody: shared ones, the accounts systems,
# virtual machines and containers make for themselves, and the cast.
NOBODY_AT_HOME = {
    "public", "example", "shared", "default", "defaultuser0", "all", "guest", "linuxbrew", "root",
    "administrator", "admini", "admin", "user", "ubuntu", "debian", "fedora", "vagrant", "node",
    "runner", "pi", "vmail", "nobody", "ec2-user", "wdagutilityaccount",
}

# A certificate's name that stands for nobody.
NOBODYS_CERTIFICATE = re.compile(
    r"|<[^>]*>|Something|Lyonoyl Mail|.*example.*|(?:" + CAST.pattern + r")(?: (?:Quorin|Vandel|Ordrin))?",
    re.IGNORECASE,
)

# --------------------------------------------------------------------------------------------------
# How a text is read.

# A bounce or a forward that carries the recipient inside its own name: VERP, as mailing lists write
# it into the address a bounce goes to (bounces+name=domain@list), and SRS, as forwarders do
# (SRS0=hash=time=domain=name@forwarder, and SRS1 with the first forwarder in front). Read only from
# where a bounce address is written -- the fields it goes to, the envelope a server logs -- because
# key=value@domain is also how a flag or a setting is written.
BOUNCE_ADDRESS = re.compile(
    r"(?mi)(?:^(?:Return-Path|Sender|Errors-To|Delivered-To|X-Original-To)[ \t]*:[ \t]*<?"
    r"|^List-[\w-]+[ \t]*:.*?mailto:"
    r"|smtp\.mailfrom=|envelope-from=|(?:MAIL FROM|RCPT TO):[ \t]*<|\b(?:from|to|orig_to)=<)"
    r"([^\s<>;,\"']+@[A-Za-z0-9.-]+)"
)
VERP = re.compile(r"(?<=\w[+-])([A-Za-z0-9._%]+)=((?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,})$")
SRS = re.compile(
    r"SRS[01][=+-](?:[^=@\s]*=)+?([A-Za-z0-9.-]+\.[A-Za-z]{2,})=([A-Za-z0-9._%+-]+)@", re.IGNORECASE
)

# Quoted-printable: a soft break that cuts a line, written as it is in a message or inside a string
# literal, and a byte written as =XX. An address that a soft break cuts is judged where it is whole.
SOFT_BREAK = re.compile(r"=(?:\r?\n|\\r\\n|\\n)")
CUT = re.compile(r"[A-Za-z0-9.-]*=(?:\r?\n|\\r\\n|\\n)")
QUOTED_PRINTABLE = re.compile(r"(?i)quoted-printable")

# An encoded word, as RFC 2047 writes a header outside ASCII.
ENCODED_WORD = re.compile(r"=\?[\w.-]+\?[BbQq]\?[^?\s]*\?=")

# Base64, a run in either alphabet, and lines of it one under another.
BASE64_RUN = re.compile(r"(?<![A-Za-z0-9+/_-])[A-Za-z0-9+/]{16,}={0,2}(?![A-Za-z0-9+/=_-])")
BASE64_URL_RUN = re.compile(r"(?<![A-Za-z0-9+/_-])[A-Za-z0-9_-]{16,}(?![A-Za-z0-9+/=_-])")
BASE64_LINE = re.compile(r"[A-Za-z0-9+/]{16,}={0,2}")
BASE64_LAST_LINE = re.compile(r"[A-Za-z0-9+/]+={0,2}")

# Escapes inside a string literal, bytes written as \xNN one after another, a backslash that
# continues a line, and two literals joined by a +.
ESCAPE = re.compile(r"\\(?:u\{([0-9A-Fa-f]{1,6})\}|u([0-9A-Fa-f]{4})|([rnt0\"'\\/]))")
ESCAPED_BYTES = re.compile(r"(?:\\x[0-9A-Fa-f]{2})+")
CONTINUATION = re.compile(r"\\\r?\n[ \t]*")
JOINED_LITERALS = re.compile(r"(?<!\\)\"[ \t]*\+[ \t]*\r?\n?[ \t]*\"")

# HTML tags: a block or a cell ends a word, and so does one element beside another; a tag laid
# inside a word does not. Named, so a type's parameters in code -- Vec<String> -- are not taken for
# one.
BLOCK_TAG = re.compile(
    r"(?i)</?(?:td|th|tr|table|p|div|li|ul|ol|br|h[1-6]|section|article|header|footer)\b[^<>]*>"
)
INLINE_TAGS = re.compile(
    r"(?:</?(?:a|abbr|b|bdi|bdo|big|cite|del|dfn|em|font|i|ins|kbd|mark|q|s|small|span|strike"
    r"|strong|sub|sup|tt|u|var|wbr)\b[^<>]{0,200}>\s*)+",
    re.IGNORECASE,
)
SIBLINGS = re.compile(r"</[^>]*>\s*<[A-Za-z]")
JOINS = re.compile(r"[\w@.%+-]")

# A terminal's colour codes.
ANSI = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

# HTML entities, and what NFKC folds into the letters it looks like or nobody sees.
# The entities that write an address's own characters, or nothing a reader sees: a numeric one,
# and the few named ones that do. Any name followed by a semicolon would be every `&x;` in code.
ENTITY = re.compile(
    r"&(?:#\d+|#[xX][0-9A-Fa-f]+|commat|period|lowbar|hyphen|plus|percnt|sol|bsol|tilde|nbsp|shy"
    r"|zwj|zwnj|amp);"
)
# The characters Unicode files as formatting: they change nothing a reader sees.
INVISIBLE = re.compile(
    "[\u00ad\u0600-\u0605\u061c\u06dd\u070f\u180e\u200b-\u200f\u202a-\u202e\u2060-\u2064"
    "\u2066-\u206f\ufeff\ufff9-\ufffb]"
)
TAG_START = re.compile(
    r"(?i)</?(?:a|abbr|b|bdi|bdo|big|cite|del|dfn|em|font|i|ins|kbd|mark|q|s|small|span|strike"
    r"|strong|sub|sup|tt|u|var|wbr|td|th|tr|table|p|div|li|ul|ol|br|h[1-6]|section|article|header"
    r"|footer)\b"
)
LOOKALIKE = re.compile(r"[\u00ad\u200b-\u200f\u2024\u2060-\u2064\ufe50-\ufe6b\uff00-\uffef\ufeff]")
PERCENT_BYTE = re.compile(r"%[0-9A-Fa-f]{2}")

# The start of a message: two header fields and the blank line after them.
LOOKS_LIKE_A_MESSAGE = re.compile(rb"(?:[!-9;-~]+:[^\r\n]*\r?\n(?:[ \t][^\r\n]*\r?\n)*){2,}\r?\n")

# Printable runs inside what is not text, the way `strings` reads it: long enough that random bytes
# do not spell one.
PRINTABLE_RUN = re.compile(rb"[\x20-\x7e]{16,}")


def may_stand(path, address):
    recipient, domain = address.split("@", 1)
    return (
        NOBODYS_DOMAIN.fullmatch(domain) is not None
        or CAST_RECIPIENT.fullmatch(recipient) is not None
        or PICTURE.fullmatch(domain) is not None
        or (domain.lower() in ALGORITHMS_AT and ALGORITHM.fullmatch(recipient) is not None)
        or recipient == "git"
        or (path, address) in PUBLISHED
        or (SCENE.fullmatch(path) is not None and domain.lower() == PRODUCTS_DOMAIN)
        or (path, address.lower()) == SPONSORS
    )


def nobody(user):
    return CAST_RECIPIENT.fullmatch(user) is not None or user.lower() in NOBODY_AT_HOME


def at_a_machine_may_stand(path, user, machine):
    return (
        nobody(user)
        or NOBODYS_DOMAIN.fullmatch(machine) is not None
        or NOBODYS_MACHINE.fullmatch(machine) is not None
        or VERSION.fullmatch(machine) is not None
        or ("." in machine and may_stand(path, f"{user}@{machine}"))
    )


def at_home_may_stand(name):
    name = name.rstrip(".")
    return not name or nobody(name)


def counts_from_zero(*parts):
    """An identifier made up the way invariant 22 asks: zeros, or one character over and over."""
    return all(len(set(part.lower())) == 1 for part in parts)


def guid_counts_from_zero(guid):
    """A GUID made up the same way: zeros but for the version and variant a UUID must carry, and the
    last few digits that count."""
    digits = list(guid.replace("-", "").lower())
    digits[12] = digits[16] = "0"
    return counts_from_zero("".join(digits[:-4]))


def as_text(path, data):
    """The file as text, or nothing for one that is not text at all.

    Text a Windows tool wrote is UTF-16, with or without the mark that says so: one byte in two is
    zero, which is what makes `grep` call it binary and walk past it.
    """
    if data.startswith((b"\xff\xfe", b"\xfe\xff")):
        return data.decode("utf-16", "replace")
    if b"\0" not in data or path.lower().endswith(MESSAGES):
        return data.decode("utf-8", "replace")

    half = len(data) / 2
    zeros_even, zeros_odd = data[0::2].count(0), data[1::2].count(0)
    if zeros_odd > half * 0.3 and zeros_even < half * 0.05:
        return data.decode("utf-16-le", "replace")
    if zeros_even > half * 0.3 and zeros_odd < half * 0.05:
        return data.decode("utf-16-be", "replace")
    return None


def printable_runs(data):
    return "\n".join(run.decode("ascii") for run in PRINTABLE_RUN.findall(data))


def readable(raw):
    """Decoded bytes, as text, when they are text -- in UTF-8, or in a single-byte charset of old
    where most of it is ASCII as such text is -- and nothing for what was never text to begin with.
    Random bytes are a third ASCII, which is how a long name in code that happens to be base64's
    alphabet is told from base64 somebody wrote."""
    if not raw:
        return None
    text = raw.decode("utf-8", "replace")
    good = sum((char.isprintable() and char != "\ufffd") or char in "\r\n\t" for char in text)
    if good >= len(text) * 0.85:
        return text
    plain = sum(32 <= byte < 127 or byte in (9, 10, 13) for byte in raw)
    if plain >= len(raw) * 0.7:
        return raw.decode("latin-1")
    return None


def from_base64(run):
    run = run.rstrip("=")
    # A run cut where a message was cut ends in a character that cannot stand alone.
    if len(run) % 4 == 1:
        run = run[:-1]
    # The URL alphabet where the run is written in it: the standard decoder drops what it does not
    # know rather than refusing it, and would hand back other bytes.
    decode = base64.urlsafe_b64decode if "-" in run or "_" in run else base64.b64decode
    try:
        return decode(run + "=" * (-len(run) % 4))
    except (binascii.Error, ValueError):
        return None


def base64_runs(text):
    """Every run of base64 in the text: lines of it one under another, joined, the shorter last line
    included, and each run alone."""
    block = []
    for line in re.split(r"\r?\n", text):
        piece = line.strip().strip("\"',;+\\")
        if BASE64_LINE.fullmatch(piece):
            block.append(piece)
            continue
        if len(block) > 1:
            if BASE64_LAST_LINE.fullmatch(piece):
                block.append(piece)
            yield "".join(block)
        block = []
    if len(block) > 1:
        yield "".join(block)
    yield from BASE64_RUN.findall(text)
    yield from BASE64_URL_RUN.findall(text)


def unliteral(text):
    """A text as the strings in it read once a compiler has read them."""

    def as_bytes(found):
        raw = bytes.fromhex(found.group(0).replace("\\x", ""))
        return raw.decode("utf-8") if _is_utf8(raw) else raw.decode("latin-1")

    def escape(found):
        braced, four, plain = found.groups()
        if plain:
            return {"r": "\r", "n": "\n", "t": "\t", "0": "\0"}.get(plain, plain)
        try:
            return chr(int(braced or four, 16))
        except (ValueError, OverflowError):
            return found.group(0)

    joined = JOINED_LITERALS.sub("", CONTINUATION.sub("", text))
    return ESCAPE.sub(escape, ESCAPED_BYTES.sub(as_bytes, joined))


def _is_utf8(raw):
    try:
        raw.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def decoded_words(text):
    """Each line with encoded words in it decoded as a header is: words next to each other joined."""

    def line(found):
        try:
            return str(make_header(decode_header(found.group(0))))
        except Exception:
            return found.group(0)

    return re.sub(r"[^\n]*=\?[^\n]*", line, text)


def inside_bounces(text):
    inside = []
    for address in BOUNCE_ADDRESS.findall(text):
        local = address.rsplit("@", 1)[0]
        inside += [f"{user}@{domain}" for user, domain in VERP.findall(local)]
    inside += [f"{user}@{domain}" for domain, user in SRS.findall(text)]
    return "\n".join(inside)


def quoted_printable(text):
    """Soft breaks joined and bytes written as =XX read as the bytes they are, in UTF-8."""
    joined = SOFT_BREAK.sub("", text)
    try:
        return quopri.decodestring(joined.encode("utf-8")).decode("utf-8", "replace")
    except (ValueError, binascii.Error):
        return joined


def unescaped(text):
    return INVISIBLE.sub("", unicodedata.normalize("NFKC", html.unescape(text)))


def untagged(text):
    def tags(found):
        run = found.group(0)
        before = text[found.start() - 1] if found.start() else ""
        after = text[found.end()] if found.end() < len(text) else ""
        joined = JOINS.match(before) and JOINS.match(after) and not SIBLINGS.search(run)
        return "" if joined else " "

    return INLINE_TAGS.sub(tags, BLOCK_TAG.sub(" ", text))


# Each way of reading a text again, by the name the report gives it, with what has to be in the text
# for the reading to say anything new -- so a file that holds none of it is read once.
TRANSFORMS = (
    ("as a string", lambda text: "\\" in text, unliteral),
    ("without colour codes", lambda text: "\x1b" in text, lambda text: ANSI.sub("", text)),
    (
        "unescaped",
        lambda text: ENTITY.search(text) is not None or LOOKALIKE.search(text) is not None,
        unescaped,
    ),
    ("untagged", lambda text: "<" in text and TAG_START.search(text) is not None, untagged),
    (
        "unquoted",
        lambda text: PERCENT_BYTE.search(text) is not None,
        lambda text: urllib.parse.unquote(urllib.parse.unquote(text)),
    ),
    (
        "quoted-printable",
        lambda text: QUOTED_PRINTABLE.search(text) is not None and "=" in text,
        quoted_printable,
    ),
    ("encoded words", lambda text: "=?" in text, decoded_words),
    (
        "inside a bounce address",
        lambda text: "SRS" in text.upper() or BOUNCE_ADDRESS.search(text) is not None,
        inside_bounces,
    ),
)


def messages_in(path, data):
    """The messages a file holds: one, or every message of an mbox, or the one an .emlx wraps."""
    lowered = path.lower()
    if lowered.endswith(".mbox"):
        return [chunk for chunk in re.split(rb"(?m)^From [^\r\n]*\r?\n", data) if chunk.strip()]
    if lowered.endswith(".emlx"):
        first, _, rest = data.partition(b"\n")
        length = first.strip()
        return [rest[: int(length)] if length.isdigit() else rest]
    return [data]


def message_parts(data):
    """A message part by part: every header with its encoded words decoded, every attachment's
    name, and every body -- text in its own character set, anything else for its printable runs, as
    a picture's are -- with the bytes of each body for what is inside it."""
    if data.startswith(b"\xef\xbb\xbf"):
        data = data[3:]
    try:
        parts = list(email.message_from_bytes(data, policy=email.policy.default).walk())
    except Exception:
        return

    for part in parts:
        for name, value in part.raw_items():
            try:
                said = f"{name}: {make_header(decode_header(str(value)))}"
            except Exception:
                said = f"{name}: {value}"
            yield said, said.encode("utf-8", "replace")
        try:
            attached = part.get_filename()
        except Exception:
            attached = None
        if attached:
            yield attached, attached.encode("utf-8", "replace")
        if part.is_multipart():
            continue

        try:
            payload = part.get_payload(decode=True)
        except Exception:
            continue
        if not payload:
            continue
        if part.get_content_maintype() not in ("text", "message"):
            runs = printable_runs(payload)
            yield runs, runs.encode("ascii")
            continue
        try:
            body = payload.decode(part.get_content_charset() or "utf-8", "replace")
        except Exception:
            body = payload.decode("utf-8", "replace")
        yield body, payload


def derivations(path, data, text, depth, decoded):
    """Every reading one step on from this one: as a message, through each transform, and what
    base64 inside it holds."""
    if (depth == 0 and path.lower().endswith(MESSAGES)) or LOOKS_LIKE_A_MESSAGE.match(data):
        for message in messages_in(path, data) if depth == 0 else [data]:
            for said, raw in message_parts(message):
                yield "decoded", said, raw

    for name, worth_it, transform in TRANSFORMS:
        if not worth_it(text):
            continue
        read_again = transform(text)
        if read_again and read_again != text:
            yield name, read_again, read_again.encode("utf-8", "replace")

    for run in base64_runs(text):
        if run not in decoded:
            raw = from_base64(run)
            decoded[run] = (readable(raw), raw) if raw else (None, None)
        inner, raw = decoded[run]
        if inner is not None:
            yield "base64", inner, raw


def readings(path, data, text, depth=0, how=(), seen=None, decoded=None):
    """Every way the text can be read, each with the steps that led to it -- each text from the
    shallowest depth it is reached at, and each run of base64 decoded once, however many readings
    hold it."""
    seen = {} if seen is None else seen
    decoded = {} if decoded is None else decoded
    if seen.get(text, DEEPEST + 1) <= depth:
        return
    seen[text] = depth

    yield how, text
    if depth >= DEEPEST:
        return
    for name, said, raw in derivations(path, data, text, depth, decoded):
        yield from readings(path, raw, said, depth + 1, (*how, name), seen, decoded)


def near(said, lowered, words, shape, before=48, after=256):
    """Every match of a costly shape, looked for only on the lines around the words it needs: none
    of these shapes runs across a line, and a whole file is a long way to try one at every place."""
    places = []
    for word in words:
        at = lowered.find(word)
        while at != -1:
            places.append((at, len(word)))
            at = lowered.find(word, at + 1)

    looked_to = 0
    for at, length in sorted(places):
        if at + length <= looked_to:
            continue
        line_start = said.rfind("\n", 0, at) + 1
        start = max(looked_to, line_start, at - before)
        line_end = said.find("\n", at + length + after)
        end = len(said) if line_end == -1 else line_end
        yield from shape.finditer(said, start, end)
        looked_to = end


class Judge:
    """What one file says, and everything in it that names somebody, each said once."""

    def __init__(self, path):
        self.path = path
        self.said = set()
        self.refused = False

    def refuse(self, what, where, found):
        if (what, found) in self.said:
            return
        self.said.add((what, found))
        self.refused = True
        print(f"LAYERING: {what} -> {self.path}{where}: {found}")

    def read(self, copy, said, how):
        def where(found):
            line = " → ".join(how) if how else said.count("\n", 0, found.start()) + 1
            return f"{copy}:{line}"

        lowered = said.lower()

        # Each shape is looked for only where what it needs is in the text at all.
        if "@" in said and LICENCES.fullmatch(self.path) is None:
            # A soft break cuts an address only where the text is quoted-printable, and there the
            # reading that joins the lines judges all of it.
            cut_is_soft = QUOTED_PRINTABLE.search(said) is not None
            wide = () if said.isascii() else (
                found for found in WIDE.finditer(said) if not found.group(0).isascii()
            )
            for found in [*ADDRESS.finditer(said), *wide]:
                if cut_is_soft and CUT.match(said, found.end()):
                    continue
                if not may_stand(self.path, found.group(0)):
                    self.refuse(
                        "an address that could reach somebody", where(found), found.group(0)
                    )

            shapes = [*PROMPTS] if PROMPT_START.search(said) else []
            if any(command in lowered for command in ("ssh", "scp", "sftp", "rsync", "mosh")):
                shapes.append(REMOTE)
            for shape in shapes:
                for found in shape.finditer(said):
                    if not at_a_machine_may_stand(self.path, found.group(1), found.group(2)):
                        self.refuse(
                            "somebody at a machine",
                            where(found),
                            f"{found.group(1)}@{found.group(2)}",
                        )

        # A tag taken out of a path is the one place a reading invents a home that is not there:
        # C:\Users\<you>\AppData would read as somebody called AppData.
        if "untagged" not in how:
            homes = (
                *near(said, lowered, ("users", "home", "settings"), HOME),
                *(TILDE.finditer(said) if "~" in said else ()),
                *near(said, lowered, ("-users-", "-home-"), DASHED),
            )
            for found in homes:
                if not at_home_may_stand(found.group(1)):
                    self.refuse("a path holding a home directory", where(found), found.group(0))

        if any(sign in lowered for sign in ("developer", "apple", "iphone", "team", "key-id")):
            kinds = ("developer", "apple", "iphone")
            for found in near(said, lowered, kinds, CERTIFICATE, before=32):
                name = (found.group(1) or found.group(3)).strip()
                team = found.group(2)
                named = NOBODYS_CERTIFICATE.fullmatch(name) is None
                if named or (team and not counts_from_zero(team)):
                    self.refuse("a signing identity", where(found), found.group(0).strip())
            settings = ("teamidentifier", "team", "--key-id", "apple_api_key")
            for found in near(said, lowered, settings, TEAM, before=16):
                if not counts_from_zero(found.group(1)):
                    self.refuse("a signing identity", where(found), found.group(0))
        if "s-1-5-21-" in lowered:
            for found in SID.finditer(said):
                if not counts_from_zero(*found.groups()):
                    self.refuse("an identifier a real system minted", where(found), found.group(0))
        if "tenant" in lowered or "issuer" in lowered:
            for found in near(said, lowered, ("tenant", "issuer"), TENANT, before=24):
                if not guid_counts_from_zero(found.group(1)):
                    self.refuse("an identifier a real system minted", where(found), found.group(1))
        if "cn=" in lowered:
            for found in DIRECTORY_NAME.finditer(said):
                name = found.group(1)
                zeroed = set(name.replace("-", "").lower()) <= {"0"}
                if not (zeroed or CAST.search(name) or "example" in name.lower()):
                    self.refuse("an identifier a real system minted", where(found), found.group(0))

    def read_bytes(self, copy, data):
        text = as_text(self.path, data)
        if text is None:
            self.read(copy, printable_runs(data), ("strings",))
            return
        for how, said in readings(self.path, data, text):
            self.read(copy, said, how)


class Unreadable(Exception):
    """Git could not answer, and a check that reads nothing must not pass."""


def git(*arguments):
    try:
        answer = subprocess.run(["git", *arguments], capture_output=True, check=False)
    except OSError as error:
        raise Unreadable(f"git {arguments[0]}: {error}") from error
    if answer.returncode != 0:
        shown = " ".join(argument if isinstance(argument, str) else "…" for argument in arguments)
        raise Unreadable(f"git {shown} answered {answer.returncode}")
    return answer.stdout


def listed(*arguments):
    return [name for name in git(*arguments, "-z").split(b"\0") if name]


def staged(name):
    """The copy of a file the next commit takes, found by its object rather than by a path git might
    read as something else."""
    entry = git("ls-files", "-s", "-z", "--", name).split(b"\0")[0]
    if not entry:
        return None
    return git("cat-file", "blob", entry.split()[1].decode())


def reaches_no_mailbox(address):
    _, _, domain = address.partition("@")
    return domain == "users.noreply.github.com" or NOBODYS_DOMAIN.fullmatch(domain) is not None


def who_commits():
    """Whether the next commit is right for the clone it is made in. A clone that commits under the
    product's name, Lyonoyl Mail, commits for the project: there the address is the one the product
    commits under -- LYONOYL_COMMIT_ADDRESS where it is set outside the tree, else the one it has
    committed under before, and either way one that reaches no mailbox -- and there is no signature,
    which names whoever signs it. Any other name is a contributor's own, which lands only as the
    commit a maintainer makes, and is not asked about. Nothing about the person is ever printed."""
    history = subprocess.run(
        ["git", "log", "-1", "--format=%ae", "--author=^Lyonoyl Mail <"],
        capture_output=True,
        text=True,
        check=False,
    )
    # A repository with no commit yet has no history to ask, and that is not a failure to read it.
    before = os.environ.get("LYONOYL_COMMIT_ADDRESS") or history.stdout.strip()
    refused = False
    the_products = False

    for role, variable in (("authored", "GIT_AUTHOR_IDENT"), ("committed", "GIT_COMMITTER_IDENT")):
        answer = subprocess.run(
            ["git", "var", variable], capture_output=True, text=True, check=False
        )
        name, _, rest = answer.stdout.strip().partition(" <")
        address = rest.partition(">")[0]
        if answer.returncode == 0 and name != "Lyonoyl Mail":
            continue
        the_products = True
        its_own = reaches_no_mailbox(address) and (not before or address == before)
        if answer.returncode != 0 or not its_own:
            print(
                f"LAYERING: the next commit would be {role} under the product's name at an address "
                "the product does not commit under -> git config user.email, or a name of your own "
                "(invariant 21)"
            )
            refused = True

    if not the_products:
        return refused
    for setting in ("commit.gpgsign", "tag.gpgsign", "tag.forcesignannotated"):
        answer = subprocess.run(
            ["git", "config", "--type=bool", "--get", setting],
            capture_output=True,
            text=True,
            check=False,
        )
        if answer.stdout.strip() == "true":
            print(
                f"LAYERING: {setting} is on, and a signature names whoever signs -> "
                f"git config {setting} false (invariant 21)"
            )
            refused = True
    return refused


def main():
    sys.stdout.reconfigure(errors="backslashreplace")
    try:
        os.chdir(git("rev-parse", "--show-toplevel").decode().strip())
        tracked = listed("ls-files")
        names = sorted(set(tracked + listed("ls-files", "--others", "--exclude-standard")))
        if not names:
            raise Unreadable("git listed no files at all")
        # What the next commit takes where it is neither what the disk holds nor what is already
        # there: staged, and changed again since. A copy that only matches the last commit adds
        # nothing to it.
        differs = set(listed("diff", "--cached", "--name-only"))
        differs &= set(listed("diff", "--name-only"))

        refused = False
        for name in names:
            path = name.decode("utf-8", "surrogateescape")
            if path in GATE:
                continue

            judge = Judge(path)
            judge.read(" (its name)", path, ())
            if os.path.islink(name):
                link = os.readlink(name).decode("utf-8", "surrogateescape")
                judge.read(" (where it links)", link, ())
            else:
                try:
                    with open(name, "rb") as file:
                        judge.read_bytes("", file.read())
                except OSError:
                    pass
            if name in differs:
                copy = staged(name)
                if copy is not None:
                    judge.read_bytes(" (staged)", copy)
            refused = refused or judge.refused

        refused = who_commits() or refused
    except Unreadable as reason:
        print(f"LAYERING: the anonymity check could not read the repository -> {reason}")
        return 1
    return 1 if refused else 0


if __name__ == "__main__":
    sys.exit(main())
