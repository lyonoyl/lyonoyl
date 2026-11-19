/**
 * Where a mailbox's signature stands in what the composer opens with, and how it changes places when
 * another mailbox is chosen to write from.
 *
 * Under the room left to write in and over whatever the message opened with — the line saying who
 * is being answered and their message, or the message passed on — which is the order a person reads
 * mail in: what somebody wrote, who they are, and then what they were answering.
 */

/**
 * The line saying a signature begins, as mail has written it for decades: two dashes and a space on
 * a line of their own. Clients at the other end know it, and leave what follows it out of an answer.
 */
const DASHES = "-- ";

/** The signature as it stands in a message: under the dashes, unless it opens with them itself. */
function underDashes(signature: string): string {
  const [first = ""] = signature.split("\n", 1);
  return first.trim() === "--" ? signature : `${DASHES}\n${signature}`;
}

/**
 * Everything after what somebody writes: the signature, when there is one, then what the message
 * opened with.
 */
function foot(opening: string, signature: string): string {
  return signature === "" ? opening : `\n\n${underDashes(signature)}${opening}`;
}

/** What the composer opens with: room to write, the signature, and what the message opened with. */
export function signed(opening: string, signature: string): string {
  return foot(opening, signature);
}

/**
 * Where a message's signature stands: as it was put there, under what was written and over what the
 * message opened with; nowhere, with no line of dashes before what it opened with; or changed —
 * edited, written under, or the quotation under it edited — and then it is the person's own text.
 */
export type Standing = "signed" | "unsigned" | "changed";

/** The line of dashes a signature begins under, with or without its space. */
const A_LINE_OF_DASHES = /(^|\n)-- ?(\n|$)/;

export function standing(
  body: string,
  opening: string,
  signature: string,
): Standing {
  if (signature !== "" && body.endsWith(foot(opening, signature)))
    return "signed";
  if (!body.endsWith(opening)) return "changed";

  const written = body.slice(0, body.length - opening.length);
  return A_LINE_OF_DASHES.test(written) ? "changed" : "unsigned";
}

/**
 * The message signed with `after` instead of `before`: in its place where `before` stands as it
 * was put, and under what was written where there is no signature at all.
 *
 * A signature somebody changed or wrote under is theirs, and so is a message whose quotation was
 * edited, since there is no telling any more where a signature would end: those are left as they
 * are, rather than given a second signature or cut where somebody wrote.
 */
export function withSignatureOf(
  body: string,
  opening: string,
  before: string,
  after: string,
): string {
  switch (standing(body, opening, before)) {
    case "signed":
      return (
        body.slice(0, body.length - foot(opening, before).length) +
        foot(opening, after)
      );
    case "unsigned":
      return body.slice(0, body.length - opening.length) + foot(opening, after);
    case "changed":
      return body;
  }
}

/**
 * What the menu calls a signature, since it has no name of its own: its first line with something
 * on it besides the dashes, which is usually the name of whoever signs.
 */
export function signatureName(signature: string): string {
  return (
    signature
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line !== "" && line !== "--") ?? ""
  );
}
