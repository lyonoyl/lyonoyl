import { PAGE_SIZE } from "./useMessageWindows";

/**
 * Where a row of the list sits once the conversations above it have opened, and what sits at a
 * place in it.
 *
 * In a file of its own for the reason `rows.ts` is: four things ask this question — the virtualizer
 * asks how many places there are, the visible range asks which rows they cover, moving the
 * selection asks where to scroll, and the draw asks what to put at each one. All four have to get
 * the same answer, and the way they stop agreeing is an off-by-one that shows up as an arrow key
 * landing on the wrong message.
 *
 * Nothing here is React and nothing here reads a row. A place maps to a row and back by counting
 * the messages that opened above it, which is arithmetic over one small sorted array; who the
 * message at that place is stays with the caller, which is the only part that needs the cache.
 */

/** A conversation open in the list: the row it hangs under, and how many messages it added. */
export interface Unfolded {
  index: number;
  messages: number;
}

/**
 * What sits at a place: a row of the list, and the message under it when the place is inside an
 * open conversation.
 */
export interface Place {
  index: number;
  at?: number;
}

/**
 * Where each open conversation sits, and how many messages it added under it, from the open ones
 * alone and sorted: a hundred thousand rows cost nothing to draw, and finding the place a position
 * lands in walks a handful of entries.
 *
 * Where it sits is asked for rather than remembered, and one whose row no loaded page holds adds
 * nothing: it is still open, and draws again as soon as its row comes back. A remembered position
 * would draw a conversation's messages under whatever row moved into its place.
 */
export function unfoldedFrom(
  opened: ReadonlyMap<string, readonly unknown[]>,
  positionOf: (conversation: string) => number | null,
): Unfolded[] {
  return [...opened.entries()]
    .map(([conversation, messages]) => ({
      index: positionOf(conversation),
      messages: messages.length,
    }))
    .filter((one): one is Unfolded => one.index !== null)
    .sort((left, right) => left.index - right.index);
}

/**
 * Where the selection goes when the row it is on leaves: the next one the way the person was
 * going, and the one the other way when there is nothing that way — at the top of a folder going
 * up, at the bottom going down. Nothing when the row was the last one there is.
 */
export function nextAfterLeaving(
  at: number,
  total: number,
  heading: "up" | "down",
): number | null {
  const above = at > 0 ? at - 1 : null;
  const below = at < total - 1 ? at + 1 : null;
  return heading === "up" ? (above ?? below) : (below ?? above);
}

/** How many places the open conversations add to the list. */
export function addedBy(unfolded: readonly Unfolded[]): number {
  return unfolded.reduce((sum, one) => sum + one.messages, 0);
}

/**
 * What sits at a place in the list.
 *
 * `unfolded` is sorted by `index`, so this walks it once and stops at the first conversation that
 * opens at or below the place asked about.
 */
export function itemAt(place: number, unfolded: readonly Unfolded[]): Place {
  let passed = 0;
  for (const one of unfolded) {
    const opensAt = one.index + passed;
    if (place <= opensAt) break;
    if (place <= opensAt + one.messages) {
      return { index: one.index, at: place - opensAt - 1 };
    }
    passed += one.messages;
  }
  return { index: place - passed };
}

/**
 * Where a row of the list sits once the conversations above it have opened.
 *
 * A conversation open on the row asked about does not move it: its messages hang underneath.
 */
export function placeOf(index: number, unfolded: readonly Unfolded[]): number {
  let passed = 0;
  for (const one of unfolded) {
    if (one.index >= index) break;
    passed += one.messages;
  }
  return index + passed;
}

/**
 * The row at the top of the screen, and the first and the last rows wholly on it.
 *
 * The top is what a list is scrolled back to. The other two are where a first key press starts
 * from in a list where nothing has been chosen yet: down starts at the top of what is on the screen
 * and up at the bottom of it, so the first press lands on something the person can see.
 */
export function rowsOnScreen(
  top: number,
  height: number,
  list: { places: number; height: number; gutter: number },
  unfolded: readonly Unfolded[],
): { top: number; first: number; last: number } {
  const within = (place: number) =>
    Math.max(0, Math.min(list.places - 1, place));
  const place = (offset: number) =>
    within(Math.floor((offset - list.gutter) / list.height));

  return {
    // The row whose top is nearest the top of the screen — half of it still there is still it —
    // which is the row a list scrolled back to it puts there with the gutter above it. Read as the
    // row the top of the screen cuts through, the scroll that restores it reads as the row above,
    // and every visit creeps up by one.
    top: itemAt(
      within(Math.ceil((top - list.gutter) / list.height - 0.5)),
      unfolded,
    ).index,
    first: itemAt(place(top + list.height - 1), unfolded).index,
    last: itemAt(place(top + height - list.height), unfolded).index,
  };
}

/**
 * The row a key press chooses in a list where nothing is chosen yet, or nothing for a key that
 * does not choose: down starts at the top of what is on the screen and up at the bottom of it, so
 * the first press lands on something the person can see.
 */
export function firstChoice(
  key: string,
  screen: { first: number; last: number },
  total: number,
): number | undefined {
  return {
    ArrowDown: screen.first,
    PageDown: screen.first,
    ArrowUp: screen.last,
    PageUp: screen.last,
    Home: 0,
    End: total - 1,
  }[key];
}

/**
 * The row a key that jumps lands on from the one chosen, or nothing for a key that does not jump:
 * a screen's worth down or up, less a row so the one left behind is still in sight, or either end.
 */
export function jumpedTo(
  key: string,
  active: number,
  total: number,
  screen: { height: number; row: number },
): number | undefined {
  const page = Math.floor(
    Math.max(1, Math.floor(screen.height) / screen.row - 1),
  );
  return {
    PageDown: active + page,
    PageUp: active - page,
    Home: 0,
    End: total - 1,
  }[key];
}

/** Keys that act on what is chosen, and so do nothing at all while nothing is. */
export const ACTS_ON_THE_CHOSEN = [
  "Enter",
  "u",
  "U",
  "ArrowLeft",
  "ArrowRight",
];

/**
 * The rows a chosen message could now be on, having last been seen at `seenAt`, or nothing when
 * that is more than is worth reading to find out.
 *
 * Mail arriving above it moves it down and mail leaving moves it up, by as many as came and went —
 * which the change in the total says only in part, since some can come while others go. So a
 * page's worth either way, and the change in the total on top of that. A message that is on none
 * of those rows once they have all been read has left the list.
 */
export function whereItCouldBe(
  seenAt: number,
  moved: number,
  total: number,
): { start: number; end: number } | null {
  const reach = PAGE_SIZE + Math.abs(moved);
  if (reach > PAGE_SIZE * 3) return null;
  return {
    start: Math.max(0, seenAt - reach),
    end: Math.max(0, Math.min(total - 1, seenAt + reach)),
  };
}
