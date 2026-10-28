/**
 * What to show about work that may be over before anybody has read it.
 *
 * Two rules, and both exist because the alternative is a line that flickers. Work that finishes
 * quickly is never announced at all — [response-budgets.md](../../../docs/performance/response-budgets.md) puts that
 * at 250 ms, and a round over nine quiet mailboxes can be shorter than the eye takes to move.
 * Work that has been announced stays announced for long enough to read, so the end of a round does
 * not blink the line out of existence a moment after it appeared.
 *
 * Plain arithmetic over two timestamps rather than a hook: nothing here is state, and a function
 * can be asked the same question for every instant a test cares about.
 */

/** Nothing is announced until it has been going this long. */
export const BEFORE_SAYING_ANYTHING = 250;

/** Once said, it stays said at least this long. */
export const ONCE_SAID = 900;

/**
 * How often the core is asked what it is reading, while it is reading something.
 *
 * A round lasts seconds and the bar has to move inside it, so this is the pace the line is drawn
 * at. It buys a number held in memory — no database, nothing that grows with how much mail there
 * is — which is why it can be asked this often at all.
 */
export const WHILE_READING = 250;

/**
 * And how often while nothing is being read.
 *
 * Nineteen answers in twenty are "nothing", and a window open all day at the reading pace is a
 * laptop kept awake for them. A round noticed a second late still has all of itself left to show.
 */
export const WHEN_IDLE = 1000;

export type Work = {
  /** Whether anything is being done at this instant. */
  busy: boolean;
  /** When the current stretch of work began, or `null` when nothing is being done. */
  since: number | null;
  /** When the line last had something to say, or `null` when it has not said anything. */
  saidAt: number | null;
};

/**
 * Whether the line should be showing the work, at this instant.
 *
 * `saidAt` is what the caller remembers about its own last answer: without it, "keep it up for a
 * while" has nothing to measure from.
 */
export function worthSaying(work: Work, now: number): boolean {
  if (work.busy) {
    return work.since !== null && now - work.since >= BEFORE_SAYING_ANYTHING;
  }

  // Finished. Held only if it was ever shown — work that went by under the threshold is not
  // announced after the fact, which would be the flicker with an extra step.
  return work.saidAt !== null && now - work.saidAt < ONCE_SAID;
}

/**
 * How many mailboxes' worth has been read, out of how many there are.
 *
 * Not how many have finished. Every mailbox is read at the same moment as every other, so they
 * finish within a second of each other at the end of a round: counted that way the number is
 * nought for five seconds and then races through every value it has. What is asked here is how far
 * through the work the round is, said in the unit a person thinks in — and the work is folders.
 *
 * The total is never shown before the work is actually over: an eight-of-nine that sits at
 * nine-of-nine while the last mailbox is still being read is the one reading a person would act
 * on, and it would be wrong.
 */
export function asFarAsMailboxes(reading: {
  done: number;
  asked: number;
  through: number;
  of: number;
}): number {
  if (reading.asked === 0) return 0;
  // Everything finished, so say so exactly.
  if (reading.done >= reading.asked) return reading.asked;
  // Nothing said how far along it is yet: fall back to what did finish.
  if (reading.of === 0) return reading.done;

  const through = (reading.through / reading.of) * reading.asked;

  return Math.min(Math.floor(through), reading.asked - 1);
}
