/**
 * Where the addresses are in text a person wrote: what the composer marks as a message is typed,
 * and what a message sent from here offers as links before its copy is read back.
 *
 * The rule is the core's, which offers the same addresses as links in plain-text mail, in the
 * markup it sends beside a message's text, and above a message passed on in its own form: an
 * address is written whole, beginning `http://`, `https://` or `mailto:`, or `www.` at the start of
 * a word; it runs to the first space or the first character an address in a sentence does not hold,
 * and leaves behind what a sentence puts after one. Somebody to write to counts written alone too,
 * `zeta@example.com`: a name of letters, digits and `._+-`, an `@`, and a domain of letters,
 * digits, dots and hyphens, read for as long as they last and no further. It starts only where no
 * name could be going on, ends only where nothing could go on with it, and has the shape mail is
 * sent to, which `isSomebodyToWriteTo` describes. Two copies of a rule drift apart, so both answer
 * the same examples, `tests/fixtures/links-in-text.json`, and the tests on both sides read them.
 *
 * Positions are in the string's own units, so a slice of the text is the address as written.
 */

/** One address in a text: from `start`, up to and not including `end`. */
export type TextLink = { start: number; end: number };

/** How an address may begin, as lower-case letters. */
const BEGINNINGS = ["https://", "http://", "mailto:", "www."];

/**
 * What an address never holds in a sentence, besides spaces and control characters: the characters
 * the URL standard asks to be written escaped, and the marks that end a clause in scripts that put
 * no space after an address.
 */
const ENDS = new Set([
  ..."<>\"`{}|\\^",
  "。", "、", "，", "．", "：", "；", "！", "？", "（",
  "）", "「", "」", "『", "』", "【", "】", "《", "》",
  "〈", "〉", "“", "”", "‘", "’", "«", "»",
]);

/** What a sentence puts after an address, and an address almost never ends with. */
const AFTER = new Set([".", ",", ";", ":", "!", "?", "'", '"', "*", "…"]);

/** The characters the core counts as spaces: Unicode's White_Space, and nothing besides. */
const SPACES = new Set([
  ..."\t\n\u000B\f\r \u0085      　",
  ...Array.from({ length: 11 }, (_, at) => String.fromCharCode(0x2000 + at)),
]);

/** What joins on to an address before it, so that a beginning there is inside a word. */
const JOINS = /[A-Za-z0-9@./\-_+%]/;

/**
 * The longest somebody to write to can be, as mail carries one: 254 characters of ASCII, counted
 * without the full stop a sentence puts after it.
 */
const LONGEST_TO_WRITE_TO = 254;

/** What the standard lets a mail name hold besides letters, digits and dots. */
const IN_ANY_NAME = new Set([..."!#$%&'*+-/=?^_`{|}~"]);

/** The kinds of file a picture or a document is named with, that no domain people write to ends with. */
const FILE_KINDS = new Set([
  "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "pdf", "txt", "html", "htm", "css",
  "js", "json",
]);

/** Every address in `text`, in order. One pass, whatever the text is made of. */
export function linksIn(text: string): TextLink[] {
  const found: TextLink[] = [];
  let at = 0;

  while (at < text.length) {
    const beginning = beginningAt(text, at);
    if (beginning === undefined) {
      const alone = writtenAloneAt(text, at);
      if (alone !== undefined) {
        found.push({ start: at, end: alone });
        at = alone;
        continue;
      }
      at += 1;
      continue;
    }

    const end = trimmed(text, at, beginning.length, endOfAddress(text, at));
    if (opens(text.slice(at, end), beginning)) {
      found.push({ start: at, end });
      at = end;
    } else {
      at += beginning.length;
    }
  }

  return found;
}

/** The beginning an address has at `at`, if one starts there and starts a word. */
function beginningAt(text: string, at: number): string | undefined {
  const beginning = BEGINNINGS.find(
    (one) => text.slice(at, at + one.length).toLowerCase() === one,
  );
  if (beginning === undefined) return undefined;

  // A letter in another script does not join on, since several scripts put no space between a
  // word and the address after it.
  return at === 0 || !JOINS.test(text[at - 1] ?? "") ? beginning : undefined;
}

/** Where the address starting at `start` stops: at a space, a control character or one of `ENDS`. */
function endOfAddress(text: string, start: number): number {
  let at = start;
  for (const character of text.slice(start)) {
    const code = character.codePointAt(0) ?? 0;
    const control = code < 0x20 || (code >= 0x7f && code <= 0x9f);
    if (SPACES.has(character) || control || ENDS.has(character)) return at;
    at += character.length;
  }
  return text.length;
}

/**
 * The end of the address at `start` once what a sentence put after it is left behind: the marks in
 * `AFTER`, and a closing bracket the address did not open. Never into its beginning.
 */
function trimmed(
  text: string,
  start: number,
  beginning: number,
  end: number,
): number {
  while (end > start + beginning) {
    const written = text.slice(start, end);
    const characters = Array.from(written);
    const last = characters[characters.length - 1] ?? "";
    const unopened =
      (last === ")" && count(written, ")") > count(written, "(")) ||
      (last === "]" && count(written, "]") > count(written, "["));

    if (!AFTER.has(last) && !unopened) break;
    end -= last.length;
  }
  return end;
}

function count(text: string, character: string): number {
  return text.split(character).length - 1;
}

/**
 * Where somebody to write to, written alone and starting a name at `at`, ends: read for as long as
 * its name and its domain last, and no further, so no other start lies inside what was read.
 */
function writtenAloneAt(text: string, at: number): number | undefined {
  if (!isLetterOrDigit(text, at) || !startsAName(text, at)) return undefined;

  let end = at;
  while (end < text.length && (isLetterOrDigit(text, end) || "._+-".includes(text[end] ?? ""))) {
    end += 1;
  }
  if (text[end] !== "@") return undefined;
  const named = end;
  end += 1;
  while (end < text.length && (isLetterOrDigit(text, end) || ".-".includes(text[end] ?? ""))) {
    end += 1;
  }

  // What follows ends it only where nothing could go on with it: another `@`, or a character a
  // name holds that a sentence does not put after an address.
  const next = text[end];
  if (next === "@" || (next !== undefined && IN_ANY_NAME.has(next) && !AFTER.has(next))) {
    return undefined;
  }
  // A full stop after the domain is the sentence's.
  while (end > named + 1 && text[end - 1] === ".") end -= 1;

  const written = text.slice(at, end);
  return written.length <= LONGEST_TO_WRITE_TO &&
    isSomebodyToWriteTo(written) &&
    openable(`mailto:${written}`) !== null
    ? end
    : undefined;
}

/** An ASCII letter or digit at `at`. */
function isLetterOrDigit(text: string, at: number): boolean {
  return /[A-Za-z0-9]/.test(text[at] ?? "");
}

/**
 * Whether a name of somebody to write to can start at `at`: the start of the text, or after a
 * character no name holds, so a name is never read from inside another; not after a colon either,
 * where an address's own sign-in ends.
 */
function startsAName(text: string, at: number): boolean {
  if (at === 0) return true;
  const before = text[at - 1] ?? "";
  return !(/[A-Za-z0-9.@:]/.test(before) || IN_ANY_NAME.has(before));
}

/**
 * Whether `written` is an address mail is sent to: a name of letters, digits and `._+-` that
 * neither starts nor ends with a dot, one `@`, and a domain of at least two names of letters, digits
 * and inner hyphens, the last of two or more letters and not a kind of file.
 */
function isSomebodyToWriteTo(written: string): boolean {
  const at = written.indexOf("@");
  if (at === -1) return false;
  const name = written.slice(0, at);
  const domain = written.slice(at + 1);

  const named =
    name !== "" &&
    !name.startsWith(".") &&
    !name.endsWith(".") &&
    !name.includes("..") &&
    /^[A-Za-z0-9._+-]+$/.test(name);
  const names = domain.split(".");
  const last = names[names.length - 1] ?? "";
  const placed =
    names.length >= 2 &&
    names.every((label) => /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(label)) &&
    /^[A-Za-z]{2,}$/.test(last) &&
    !FILE_KINDS.has(last.toLowerCase());

  return named && placed;
}

/** Whether a written address opens anything this build opens. */
function opens(written: string, beginning: string): boolean {
  const rest = written.slice(beginning.length);
  if (rest === "") return false;

  if (beginning === "www.") {
    const host = rest.split(/[/?#:]/)[0] ?? "";
    return (
      host.includes(".") &&
      !host.startsWith(".") &&
      !host.endsWith(".") &&
      openable(`https://${written}`) !== null
    );
  }
  if (beginning === "mailto:") {
    return rest.includes("@") && openable(written) !== null;
  }
  return (openable(written)?.host ?? "") !== "";
}

/** The address as the URL standard reads it, when it is one this build opens. */
function openable(written: string): URL | null {
  try {
    const url = new URL(written);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? url : null;
  } catch {
    return null;
  }
}
