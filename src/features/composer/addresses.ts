/**
 * What a line of recipients holds, and whether it can be sent to.
 *
 * A pure module rather than something inside the form: the same two questions are asked by the
 * button that decides whether a message may go and by the code that hands the addresses over, and
 * two answers to "is this an address" is a Send button that lights up for something the server
 * will refuse after the wait.
 */

/** The addresses written on one line, in the order they were written. */
export function addressesIn(written: string): string[] {
  return written
    .split(",")
    .map((address) => address.trim())
    .filter((address) => address !== "");
}

/**
 * Whether one written address is one at all.
 *
 * Deliberately loose. The rule for what an address may be is long, older than this application and
 * stranger than anybody expects -- quoted local parts, addresses with no dot in the domain, hosts
 * that are only a name on a local network. What is refused here is what is certainly not an
 * address: nothing before the `@`, nothing after it, more than one `@`, or a space anywhere. A
 * check that went further would refuse mail somebody can actually send, and the server is the one
 * that knows.
 */
export function looksLikeAnAddress(written: string): boolean {
  const [local, domain, ...rest] = written.split("@");

  if (rest.length > 0) return false;
  if (local === undefined || domain === undefined) return false;
  if (local === "" || domain === "") return false;

  return !/[\s,<>]/.test(written);
}

/**
 * Whether this line can be sent to: at least one address, and every one of them an address.
 *
 * Every one, not merely the first: a line that ends in a half-typed second address would send the
 * message to the first and fail on the second, which is a message half sent.
 */
export function everyOneAnAddress(written: string): boolean {
  const found = addressesIn(written);

  return found.length > 0 && found.every(looksLikeAnAddress);
}

/** The address being typed on a line of addresses: where it lies, and what of it is typed so far. */
export interface BeingTyped {
  /** Where it starts: after the comma before the caret, and after the spaces past that comma. */
  start: number;
  /** Where it ends: at the next comma, or at the end of the line. */
  end: number;
  /** What is written of it before the caret. */
  typed: string;
}

/**
 * The address the caret stands in, on a line where commas part one address from the next.
 *
 * The same rule `addressesIn` splits by, so what is offered for and what is sent read one line the
 * same way.
 */
export function beingTyped(written: string, caret: number): BeingTyped {
  const at = Math.max(0, Math.min(caret, written.length));
  let start = written.slice(0, at).lastIndexOf(",") + 1;
  while (start < at && /\s/.test(written[start] ?? "")) start += 1;
  const comma = written.indexOf(",", at);
  const end = comma === -1 ? written.length : comma;

  return { start, end, typed: written.slice(start, at) };
}

/**
 * The line with a chosen address in place of the one being typed, and where the caret goes.
 *
 * All of the address being typed goes, what follows the caret up to the next comma included, and
 * every other address stays as it was. At the end of the line a comma and a space follow the
 * chosen one, so the next can be typed straight away.
 */
export function withChosen(
  written: string,
  caret: number,
  chosen: string,
): { written: string; caret: number } {
  const { start, end } = beingTyped(written, caret);
  const before = written.slice(0, start);
  const after = written.slice(end);
  const parted = after === "" ? ", " : "";

  return {
    written: before + chosen + parted + after,
    caret: before.length + chosen.length + parted.length,
  };
}
