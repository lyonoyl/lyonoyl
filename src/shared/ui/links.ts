/**
 * Where the addresses are in text a person wrote: what the composer marks as a message is typed,
 * and what a message sent from here offers as links before its copy is read back.
 *
 * The rule is the core's, which offers the same addresses as links in plain-text mail: an address
 * is written whole, beginning `http://`, `https://` or `mailto:`, or `www.` at the start of a word;
 * it runs to the first space or the first character an address in a sentence does not hold, and
 * leaves behind what a sentence puts after one. Two copies of a rule drift apart, so both answer
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

/** Every address in `text`, in order. One pass, whatever the text is made of. */
export function linksIn(text: string): TextLink[] {
  const found: TextLink[] = [];
  let at = 0;

  while (at < text.length) {
    const beginning = beginningAt(text, at);
    if (beginning === undefined) {
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
