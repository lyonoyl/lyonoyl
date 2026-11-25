import type { CommandError } from "@/shared/ipc/generated/CommandError";

/**
 * What was asked of a message's files, for as long as the window runs rather than as long as one
 * row of them is drawn.
 *
 * The invariant it holds: a file is never asked for twice while it is on its way, and what an
 * opening comes to is said wherever its message's files are drawn next. A row drawn again for the
 * same message — after the person looked at another, or after the settings stood over the reader —
 * would otherwise show a file on its way as idle, ask for it again, and lose what it came to.
 *
 * A place is a message and the place of a file in it.
 */

/** A file that did not open or was not saved: its message, its name, and what was said. */
export type Failure = { messageId: string; name: string; error: CommandError };

const onItsWay = new Set<string>();
const failures = new Map<string, Failure>();
const listeners = new Set<() => void>();
let version = 0;

function changed() {
  version += 1;
  for (const listener of listeners) listener();
}

export function placeOf(messageId: string, at: number): string {
  return `${messageId}/${at}`;
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** What a row reads to know it is drawn from the latest of all this. */
export function versionNow(): number {
  return version;
}

export function isOnItsWay(place: string): boolean {
  return onItsWay.has(place);
}

export function failureAt(place: string): Failure | undefined {
  return failures.get(place);
}

/**
 * Starts an opening, unless one is already on its way at that place, and answers whether it
 * started. What was said about that place before goes: this is the answer that matters now.
 */
export function began(place: string): boolean {
  if (onItsWay.has(place)) return false;
  onItsWay.add(place);
  failures.delete(place);
  changed();
  return true;
}

/** Ends an opening, with what it came to where it did not open. */
export function ended(place: string, failure?: Failure) {
  onItsWay.delete(place);
  if (failure) failures.set(place, failure);
  changed();
}

/** Says what saving a file came to, or, with nothing, takes back what was said about it. */
export function said(place: string, failure: Failure | null) {
  if (failure) failures.set(place, failure);
  else failures.delete(place);
  changed();
}

/**
 * Forgets what was said about every other message's files, as a row is drawn for this one: a line
 * about the last message is not one to come back to. What is still on its way is kept, and says
 * what it came to whenever it does, even while another message is on screen.
 */
export function forgetAllBut(messageId: string) {
  let forgot = false;
  for (const [place, failure] of failures) {
    if (failure.messageId !== messageId) {
      failures.delete(place);
      forgot = true;
    }
  }
  if (forgot) changed();
}

/** Everything back to nothing, so that each test starts from it. */
export function forgetEverything() {
  onItsWay.clear();
  failures.clear();
  changed();
}
