import type { Translator } from "@/shared/i18n";

/**
 * Bytes, as somebody would say them.
 *
 * Here rather than in a screen because two of them now have to agree: the settings say what each
 * mailbox takes, and the reader says what one file does. Two copies would be two answers to the
 * same question, and the day one gained a unit the other would not.
 *
 * A string in and a string out: a mailbox can hold more than a JavaScript number holds exactly, so
 * the count crosses as text and is turned into a figure only once it has been made small.
 *
 * Powers of a thousand, not of 1024, and the units named for them. What the operating system shows
 * beside the same file is what a person will compare this against, and on two of the three systems
 * that is the decimal one.
 */
export function inRoundNumbers(
  bytes: string,
  language: string,
  t: Translator,
): string {
  const held = Number(BigInt(bytes));
  // The unit is a word somebody reads, so it comes from the catalog rather than from here.
  const units = [
    "size.b",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ] as const;

  let at = 0;
  let shown = held;
  while (shown >= 1000 && at < units.length - 1) {
    shown /= 1000;
    at += 1;
  }

  return t(units[at], {
    amount: new Intl.NumberFormat(language, {
      maximumFractionDigits: at === 0 ? 0 : 1,
    }).format(shown),
  });
}
