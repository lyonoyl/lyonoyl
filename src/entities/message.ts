import type { Translator } from "@/shared/i18n";
import type { AnswerDto } from "@/shared/ipc/generated/AnswerDto";
import type { AttachingDto } from "@/shared/ipc/generated/AttachingDto";
import type { ForwardDto } from "@/shared/ipc/generated/ForwardDto";
import type { MessageDetailsDto } from "@/shared/ipc/generated/MessageDetailsDto";

/**
 * What the composer is opened with.
 *
 * It lives here rather than beside either screen because both of them need it and neither may
 * import the other: the reader builds one from the message it is showing, the composer is opened
 * with it. What goes into the two headers is decided in Rust — this only carries what the answered
 * message said about itself.
 */
export interface Answer {
  /**
   * Who the answer goes to: the address the message came from, and only them.
   *
   * Never the name shown beside it. `From: Zeta Quorin <zeta@example.com>` is shown as
   * `Zeta Quorin`, and a reply addressed to that is refused before it reaches the outbox — while
   * a display name written to look like an address would send the answer somewhere the message
   * never came from.
   */
  to: string;
  /** Everyone being kept informed of the answer. Empty unless it is an answer to everybody. */
  cc: string;
  subject: string;
  /**
   * What the composer starts with: room to write, the line saying who is being answered, and their
   * message quoted under it. Empty when this device does not hold the message being answered.
   */
  body: string;
  /** The name the answered message carries, or nothing when it arrived without one. */
  answering: string | null;
  /** The conversation the answered message said it belongs to, oldest first. */
  chain: string[];
  /**
   * The account to write from: the one the answered message arrived in.
   *
   * Nothing for a message starting its own conversation, and the composer then picks the first
   * account that can send. Answering from a different account than the message came to is how a
   * thread ends up with two of the same person in it, and the reply lands in the wrong Sent.
   */
  fromAccount: string | null;
  /**
   * The draft this is, when the composer was opened on one that was written down earlier.
   *
   * Kept so that writing it down again replaces it rather than leaving a second copy in the
   * drafts folder beside the first. Nothing for a message that is not one yet.
   */
  draftId?: number;
  /** Copies nobody else sees, carried when a draft is reopened. */
  bcc?: string;
  /**
   * The files it opens with, already held in the core under their tokens: a message passed on
   * carries the files of the one it passes on. Listed, taken off and sent like a file attached a
   * moment ago, and let go of by the composer when it goes.
   */
  files?: AttachingDto[];
  /** The names of the files the passed-on message carries that are too heavy to go with it. */
  leftBehind?: string[];
}

/** A message answering nothing, which is what the compose button opens. */
export const NOTHING_TO_ANSWER: Answer = {
  to: "",
  cc: "",
  subject: "",
  body: "",
  answering: null,
  chain: [],
  fromAccount: null,
};

/**
 * The subject an answer carries.
 *
 * One `Re: ` however many times a message has been round: RFC 5322 §3.6.5 treats the subject as
 * free text, and every client on earth reads a stack of them as somebody who cannot stop replying.
 * The prefixes other languages use are collapsed too — a thread that crossed a German client comes
 * back as `Aw:`, and answering it would otherwise start a second stack beside the first. The last
 * alternative is the Russian for "answer".
 */
export function answerSubject(subject: string | null): string {
  const written = (subject ?? "").trim();
  const already = /^(re|re\[\d+\]|aw|antw|sv|ответ)\s*:\s*/i;

  let left = written;
  while (already.test(left)) {
    left = left.replace(already, "").trim();
  }

  return left === "" ? "Re:" : `Re: ${left}`;
}

/**
 * Everything an answer to this message needs.
 *
 * Who it goes to is decided in Rust and handed in: taking every address the person has out of the
 * answer is one rule, it already lives in the domain, and a second copy of it here would be a
 * second rule that drifts. What is decided here is only what the answer is called.
 */
export function answerTo(
  message: MessageDetailsDto,
  answer: AnswerDto,
  t: Translator,
  dates: Intl.DateTimeFormat,
): Answer {
  return {
    // A line of addresses, which is what the field holds and what the person edits.
    to: answer.to.join(", "),
    cc: answer.cc.join(", "),
    subject: answerSubject(message.subject),
    answering: message.named,
    chain: message.chain,
    fromAccount: message.accountId,
    body: bodyAnswering(message, answer.quoted, t, dates),
  };
}

/**
 * What an answer starts with: room to write, then who is being answered, then their message.
 *
 * The quotation itself is made in Rust, because how a message is quoted is a rule about mail. The
 * line above it is not: it is prose somebody reads, so it is written in the language they are
 * reading in and their own date format — which is what this catalog and this formatter are for.
 *
 * Two empty lines at the top, and the cursor goes there. An answer written under a quotation is one
 * most people never see the beginning of.
 */
function bodyAnswering(
  message: MessageDetailsDto,
  quoted: string,
  t: Translator,
  dates: Intl.DateTimeFormat,
): string {
  if (quoted === "") {
    return "";
  }

  const who = message.sender ?? message.senderAddress;
  const when = dates.format(new Date(Number(message.receivedAt) * 1000));
  // A sentence of its own where nobody is named, rather than a label put where a name goes.
  const said =
    who === null
      ? t("compose.answeringUnknown", { when })
      : t("compose.answering", { who, when });

  return `\n\n${said}\n${quoted}`;
}

/**
 * The subject a message passed on carries.
 *
 * One `Fwd: ` however many times it has been passed on, and `Fw:`, the other spelling in use, taken
 * for it. Not translated, for the reason `Re: ` is not: it is how every client at the other end
 * tells a message passed on from one written fresh. Prefixes other languages use for it are left
 * as they are, since some of them are words a subject can begin with.
 */
export function forwardSubject(subject: string | null): string {
  const written = (subject ?? "").trim();
  const already = /^fwd?\s*:\s*/i;

  let left = written;
  while (already.test(left)) {
    left = left.replace(already, "").trim();
  }

  return left === "" ? "Fwd:" : `Fwd: ${left}`;
}

/**
 * Everything a message passed on starts with.
 *
 * Addressed to nobody: who it goes to now is the person's to say. Answering nothing either, so it
 * starts a conversation of its own with whoever receives it. Written from the account the message
 * arrived in, as an answer is, and carrying its files, which go unless they are taken off.
 */
export function forwardOf(
  message: MessageDetailsDto,
  forward: ForwardDto,
  t: Translator,
  dates: Intl.DateTimeFormat,
): Answer {
  return {
    to: "",
    cc: "",
    subject: forwardSubject(message.subject),
    body: bodyForwarding(message, forward, t, dates),
    answering: null,
    chain: [],
    fromAccount: message.accountId,
    files: forward.files,
    leftBehind: forward.leftBehind,
  };
}

/**
 * Everything a message passed on whole starts with: nothing written yet, and the message itself as
 * one file, an exact copy the core already holds.
 *
 * Addressed to nobody and answering nothing, as a message passed on as text is, and written from
 * the account the message arrived in. No lines say whose it is: the copy says that itself.
 */
export function forwardWholeOf(
  message: MessageDetailsDto,
  forward: ForwardDto,
): Answer {
  return {
    to: "",
    cc: "",
    subject: forwardSubject(message.subject),
    body: "",
    answering: null,
    chain: [],
    fromAccount: message.accountId,
    files: forward.files,
    leftBehind: forward.leftBehind,
  };
}

/**
 * What a message passed on starts with: room to write, then lines saying whose message it is, then
 * the message.
 *
 * The lines are prose somebody reads, so they are written in the language they read in and their
 * own date format; what goes into them is the message's own. A line the message has nothing for is
 * left out rather than written empty. Addresses are joined as a line of addresses is, which is
 * what each of these lines copies.
 */
function bodyForwarding(
  message: MessageDetailsDto,
  forward: ForwardDto,
  t: Translator,
  dates: Intl.DateTimeFormat,
): string {
  const lines = [t("compose.forwarded")];

  const who = writerOf(message);
  if (who !== null) lines.push(t("compose.forwardedFrom", { who }));
  lines.push(
    t("compose.forwardedDate", {
      when: dates.format(new Date(Number(message.receivedAt) * 1000)),
    }),
  );
  const subject = (message.subject ?? "").trim();
  if (subject !== "") lines.push(t("compose.forwardedSubject", { subject }));
  if (forward.to.length > 0)
    lines.push(t("compose.forwardedTo", { who: forward.to.join(", ") }));
  if (forward.cc.length > 0)
    lines.push(t("compose.forwardedCc", { who: forward.cc.join(", ") }));

  return `\n\n${lines.join("\n")}\n\n${forward.text}`;
}

/**
 * Who wrote it, as mail names somebody: the name and the address, or whichever of the two the
 * message gave. A name that is its own address is said once.
 */
function writerOf(message: MessageDetailsDto): string | null {
  const name = message.sender?.trim() || null;
  const address = message.senderAddress;

  if (name !== null && address !== null) {
    return name.toLowerCase() === address.toLowerCase()
      ? address
      : `${name} <${address}>`;
  }
  return address ?? name;
}
