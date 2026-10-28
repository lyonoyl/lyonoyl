import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import { useCallback, useEffect, useState } from "react";

import { undoAction } from "@/shared/ipc/client";
import type { Translator } from "@/shared/i18n";

/** A message just filed away or thrown out, as the pane that did it describes it. */
export interface Filed {
  messageId: string;
  folderId: FolderIdDto;
  what: "archived" | "deleted" | "moved" | "read";
}

/** How long it stays before it takes itself away, in milliseconds. */
export const NOTICE_MS = 5000;

/** How long it takes to arrive and to go, in milliseconds. */
const MOVING_MS = 200;

/**
 * What just happened to a message, said under the list it happened in.
 *
 * Under the list rather than on the message: by the time this is read the reader has moved on to
 * the next one, and a line there would be about mail nobody is looking at any more.
 *
 * It takes itself away. An offer that has to be dismissed is one more thing to do about a message
 * somebody has already finished with.
 */
export function FilingNotice({
  filed,
  onTakenBack,
  onGone,
  onUnreadAgain,
  t,
}: {
  filed: Filed | null;
  /** Told when the message is back where it was, so the list can show it again. */
  onTakenBack: (messageId: string) => void;
  /**
   * Told once this is off the screen.
   *
   * The caller holds what was filed, and without this it holds it for ever: what it thinks is "a
   * notice is showing" stays true long after the notice went, and anything else that wanted the
   * same corner never got it.
   */
  onGone?: () => void;
  /**
   * Told when a message marked read is to be unread again, which the list then marks.
   *
   * Not marked here. The list marks a message the way its dot does — the dot and the counts beside
   * the folders move in one frame, and back if the write fails. Marked from here, the counts would
   * follow the core's word while the dot waited for the list to read itself again.
   */
  onUnreadAgain?: (messageId: string, folderId: FolderIdDto) => void;
  t: Translator;
}) {
  const [answered, setAnswered] = useState<"offered" | "tooLate" | "gone">(
    "offered",
  );
  /** Whole seconds left, which is what the ring drains by and what the circle says. */
  const [left, setLeft] = useState(NOTICE_MS / 1000);
  /**
   * On its way out, and still on screen while it goes.
   *
   * Read from the clock rather than kept beside it. Kept, it was a second thing to set -- and
   * setting it changed what the effect that had just scheduled the going depended on, so the
   * cleanup cancelled the timer that same effect had started a moment before. It never went.
   */
  const leaving = left <= 0;

  /** Whether somebody is looking at this offer right now, by pointer or by focus. */
  const [held, setHeld] = useState(false);

  // The clock is the countdown itself rather than a timer beside it. A timer that was restarted
  // whenever somebody stopped hovering gave them five fresh seconds while the circle said one --
  // two clocks disagreeing about the same offer.
  //
  // It stops while somebody is on the notice: reading it, or standing on its button. Five seconds
  // is enough to notice a mistake and not enough to reach the offer by keyboard, since the list is
  // between them and Tab has to cross it -- so without this the offer was taken away from under
  // the very person who had got to it.
  useEffect(() => {
    if (filed === null || held || leaving || left <= 0) return;

    const tick = window.setTimeout(() => setLeft((was) => was - 1), 1000);
    return () => window.clearTimeout(tick);
  }, [filed, held, leaving, left]);

  // Out rather than away: taken out of the tree at once it would blink out, and what a person is
  // being told is that something can still be undone.
  useEffect(() => {
    if (filed === null || left > 0) return;

    const gone = window.setTimeout(() => {
      setAnswered("gone");
      onGone?.();
    }, MOVING_MS);
    return () => window.clearTimeout(gone);
  }, [filed, left, onGone]);

  useEffect(() => {
    if (filed === null) return;
    setAnswered("offered");
    setLeft(NOTICE_MS / 1000);
  }, [filed]);

  const takeItBack = useCallback(() => {
    if (filed === null) return;

    // A message marked read by being opened is put back by marking it unread, which always works.
    // The way back for the others is a queued instruction that may already have gone through, and
    // that is why they can be told it is too late; this one never can.
    if (filed.what === "read") {
      setAnswered("gone");
      onUnreadAgain?.(filed.messageId, filed.folderId);
      return;
    }

    // Taking it back is one action and two things underneath: while the instruction is waiting it
    // is simply removed, and once it has gone through the message is asked for back — a second
    // instruction, which the list shows the same way it showed the first. Either way the message
    // is coming home, and the answer here says only whether anything could be done at all.
    undoAction(filed.messageId, filed.folderId)
      .then((takenBack) => {
        setAnswered(takenBack ? "gone" : "tooLate");
        if (takenBack) onTakenBack(filed.messageId);
      })
      .catch(() => {
        /* It stays as it was: an offer that failed must not look like one that worked. */
      });
  }, [filed, onTakenBack, onUnreadAgain]);

  if (filed === null || answered === "gone") {
    return null;
  }

  const WORD = {
    archived: "list.archived",
    deleted: "list.deleted",
    moved: "list.moved",
    read: "list.markedRead",
  } as const;
  const said =
    answered === "tooLate" ? t("list.cannotBringBack") : t(WORD[filed.what]);

  return (
    // Floating over the list rather than sitting under it. Under it, it pushed the rows up by its
    // own height the moment it appeared and let them drop when it went -- a list that jumps every
    // time a message is filed. Over them, nothing moves but the notice.
    <div
      aria-live="polite"
      onMouseEnter={() => setHeld(true)}
      onMouseLeave={() => setHeld(false)}
      onFocusCapture={() => setHeld(true)}
      onBlurCapture={() => setHeld(false)}
      className={`pointer-events-auto absolute inset-x-3 bottom-3 z-10 flex items-center gap-3 rounded-lg border border-border bg-popover/95 px-3 py-2 text-sm text-muted-foreground shadow-lg backdrop-blur duration-200 motion-reduce:animate-none ${
        leaving
          ? "animate-out fade-out slide-out-to-bottom-2"
          : "animate-in fade-in slide-in-from-bottom-2"
      }`}
    >
      <Countdown left={left} of={NOTICE_MS / 1000} />
      <span className="min-w-0 flex-1 truncate">{said}</span>
      {answered === "offered" && (
        <button
          type="button"
          onClick={takeItBack}
          className="shrink-0 rounded-md px-2 py-1 font-medium text-foreground hover:bg-muted"
        >
          {t("list.undo")}
        </button>
      )}
    </div>
  );
}

/**
 * How long is left, as a ring that drains and a number inside it.
 *
 * The notice takes itself away, and a thing that vanishes on its own without saying when is a
 * thing a person cannot plan around: they either hurry for no reason or lose the offer mid-reach.
 * The ring is the same clock the notice closes on, so what it shows cannot disagree with what
 * happens -- and when the clock stops, because somebody is reading this, the ring stops with it.
 */
function Countdown({ left, of }: { left: number; of: number }) {
  const around = 2 * Math.PI * 9;

  return (
    <span className="relative grid size-7 shrink-0 place-items-center">
      <svg
        aria-hidden="true"
        viewBox="0 0 24 24"
        className="absolute inset-0 size-7 -rotate-90"
      >
        <circle
          cx="12"
          cy="12"
          r="9"
          fill="none"
          strokeWidth="2"
          className="stroke-border"
        />
        <circle
          cx="12"
          cy="12"
          r="9"
          fill="none"
          strokeWidth="2"
          strokeLinecap="round"
          strokeDasharray={around}
          strokeDashoffset={around * (1 - Math.max(0, left) / of)}
          className="stroke-muted-foreground transition-[stroke-dashoffset] duration-1000 ease-linear motion-reduce:transition-none"
        />
      </svg>
      <span className="text-[0.6875rem] tabular-nums">{Math.max(0, left)}</span>
    </span>
  );
}
