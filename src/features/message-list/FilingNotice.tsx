import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MutableRefObject,
} from "react";

import { folderName } from "@/entities/folder";
import { getMailboxTree, undoAction } from "@/shared/ipc/client";
import type { Translator } from "@/shared/i18n";

/** A message just filed away or thrown out, as the pane that did it describes it. */
export interface Filed {
  messageId: string;
  folderId: FolderIdDto;
  what: "archived" | "deleted" | "moved" | "read";
}

/**
 * What the key takes back: the last message somebody filed away or threw out themselves, or
 * `"forGood"` once the trash has been emptied after it, which nothing brings back.
 *
 * A message marked read by being opened is never one. Nobody asked for that, and the next message
 * opens and is marked the moment the one being read is thrown away: counted, it would stand between
 * the key and the message somebody wants back.
 */
export type PutAway = Filed | "forGood";

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
  takeBackAsked = 0,
  inFlight,
  onTakenBack,
  onGone,
  onUnreadAgain,
  t,
}: {
  /** What the line is about: the message just filed, or what the key has asked for back since. */
  filed: PutAway | null;
  /**
   * Bumped when the key asks for what this is about back, which is answered here exactly as the
   * button is. A line the key raised is not drawn until the answer has come: until then it would
   * say the message was put away, the opposite of what was asked, and the answer is all there is to
   * say.
   */
  takeBackAsked?: number;
  /**
   * The line whose way back is still being answered, held by whoever shows this so it outlives
   * the line: a screen opened while the answer is on its way takes the line out of the tree, and
   * the line drawn after it closed must not ask a second time. Kept here when nobody holds it.
   */
  inFlight?: MutableRefObject<PutAway | null>;
  /**
   * Told when the message is back where it was, so the list can show it again, and whether this
   * line goes on to say where it went back to — the line the key raised, which then goes by itself.
   */
  onTakenBack: (messageId: string, sayingWhere: boolean) => void;
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
  /**
   * `asking` is a line the key raised, drawn as nothing while its answer is on its way, and `back`
   * is that line once the message is back, saying where.
   */
  const [answered, setAnswered] = useState<
    "offered" | "asking" | "tooLate" | "gone" | "back"
  >("offered");
  /** The folder the message went back to, as the sidebar names it, for a line that says so. */
  const [backIn, setBackIn] = useState("");
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
  /** Not on screen yet, so not counting down either. */
  const quiet = answered === "asking";

  /** Whether somebody is looking at this offer right now, by pointer or by focus. */
  const [held, setHeld] = useState(false);

  /**
   * The line being drawn, so an answer that comes once another line has taken its place is not
   * drawn over that one.
   */
  const about = useRef(filed);
  /**
   * What the key had asked when this line came, which tells a press that raised it from one made
   * while it showed.
   */
  const cameWith = useRef(takeBackAsked);
  /** What the key had asked the last time this looked, so each press is answered once. */
  const askedBefore = useRef(takeBackAsked);
  /**
   * The line whose way back is still being answered, so a second press about it asks nothing. One
   * line's, so an answer that never comes holds back no other line's way back.
   */
  const ownInFlight = useRef<PutAway | null>(null);
  const asking = inFlight ?? ownInFlight;
  /** What the line has come to, and whom to tell it went, as they are when it leaves the tree. */
  const leftAs = useRef({ answered, filed, onGone });
  useLayoutEffect(() => {
    leftAs.current = { answered, filed, onGone };
  });

  // Taken out of the tree once it has its answer -- a screen opened over the list while it said
  // where the message went back to, that it could not be put back, or that emptying the trash
  // cannot be undone, or while the answer was on its way -- the line is done with, and is
  // forgotten. Held, the screen closed again drew the offer it had answered, and its Undo asked a
  // second time about a message already back. One still only offered comes back with the list,
  // the way it went: it still means what it says. And an answer arriving after it went is about no
  // line on the screen, so it says nothing of where the message went: the list shows it back.
  useEffect(
    () => () => {
      const { answered, filed, onGone } = leftAs.current;
      about.current = null;
      if (
        answered === "asking" ||
        answered === "back" ||
        answered === "tooLate" ||
        filed === "forGood"
      ) {
        onGone?.();
      }
    },
    [],
  );

  // The clock is the countdown itself rather than a timer beside it. A timer that was restarted
  // whenever somebody stopped hovering gave them five fresh seconds while the circle said one --
  // two clocks disagreeing about the same offer.
  //
  // It stops while somebody is on the notice: reading it, or standing on its button. Five seconds
  // is enough to notice a mistake and not enough to reach the offer by keyboard, since the list is
  // between them and Tab has to cross it -- so without this the offer was taken away from under
  // the very person who had got to it.
  useEffect(() => {
    if (filed === null || held || leaving || left <= 0 || quiet) return;

    const tick = window.setTimeout(() => setLeft((was) => was - 1), 1000);
    return () => window.clearTimeout(tick);
  }, [filed, held, leaving, left, quiet]);

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

  // Before the paint, so a line is never drawn with what the one before it said, and one the key
  // raised is never drawn before its answer.
  useLayoutEffect(() => {
    about.current = filed;
    if (filed === null) return;
    cameWith.current = takeBackAsked;
    setAnswered("offered");
    setLeft(NOTICE_MS / 1000);
    // Only a new line starts over; a press is weighed below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filed]);

  const takeItBack = useCallback(
    (quietly = false) => {
      if (filed === null || filed === "forGood" || asking.current === filed)
        return;

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
      //
      // Asked once, however often it is asked for: a second way back while the first is on its way
      // would be a second instruction about a message the first is already bringing home.
      const asked = filed;
      asking.current = asked;
      if (quietly) setAnswered("asking");
      // Raised by the key, the line is about a message that may be in a folder nobody is looking
      // at, and back there changes nothing on the screen: so it says where it went back to. The
      // folder is named while the core answers; a name that cannot be had says nothing at all.
      const named = quietly
        ? nameOfFolder(asked.folderId, t)
        : Promise.resolve(null);
      Promise.all([undoAction(asked.messageId, asked.folderId), named])
        .then(([takenBack, name]) => {
          const here = about.current === asked;
          const sayingWhere = takenBack && here && name !== null;
          // Back, whatever this line says by now: the list shows it again either way.
          if (takenBack) onTakenBack(asked.messageId, sayingWhere);
          if (!here) return;
          if (sayingWhere) {
            setBackIn(name);
            setAnswered("back");
            return;
          }
          setAnswered(takenBack ? "gone" : "tooLate");
        })
        .catch(() => {
          // It stays as it was: an offer that failed must not look like one that worked. A line the
          // key raised was not drawn yet, and is drawn now as the offer that still stands.
          if (about.current === asked && quietly) setAnswered("offered");
        })
        .finally(() => {
          if (asking.current === asked) asking.current = null;
        });
    },
    [filed, onTakenBack, onUnreadAgain, t],
  );

  // A press of the key: raised by it, the line is answered before anything of it is drawn; pressed
  // while the line showed, it is the button.
  useLayoutEffect(() => {
    if (takeBackAsked === askedBefore.current) return;
    askedBefore.current = takeBackAsked;

    const raised = cameWith.current === takeBackAsked;
    if (raised || answered === "offered") takeItBack(raised);
    // Only a press is answered here; the line it is about is read as it stands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [takeBackAsked]);

  if (filed === null || answered === "gone" || quiet) {
    return null;
  }

  const WORD = {
    archived: "list.archived",
    deleted: "list.deleted",
    moved: "list.moved",
    read: "list.markedRead",
  } as const;
  // Nothing brings back what emptying the trash deleted, so that is said at once, and nothing is
  // offered. It names the emptying rather than a message: the key may have been pressed for one
  // moved before, which is still wherever it went.
  const said =
    filed === "forGood"
      ? t("list.emptyingCannotBeUndone")
      : answered === "tooLate"
        ? t("list.cannotBringBack")
        : answered === "back"
          ? t("list.movedBackTo", { folder: backIn })
          : t(WORD[filed.what]);

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
      {/* Wrapped rather than cut short: the box is sized to the text. A line saying a message could
          not be put back names the product, and in some languages, or in a list made narrow, it
          is longer than the notice is wide. */}
      <span className="min-w-0 flex-1">{said}</span>
      {answered === "offered" && filed !== "forGood" && (
        <button
          type="button"
          onClick={() => takeItBack()}
          className="shrink-0 rounded-md px-2 py-1 font-medium text-foreground hover:bg-muted"
        >
          {t("list.undo")}
        </button>
      )}
    </div>
  );
}

/**
 * What the folder is called where the person sees it, beside the list: its role's word, or the
 * name it has on the server. Nothing when it cannot be found, or the folders cannot be read.
 */
function nameOfFolder(
  folderId: FolderIdDto,
  t: Translator,
): Promise<string | null> {
  return getMailboxTree()
    .then(({ accounts }) => {
      for (const account of accounts) {
        const folder = account.folders.find((one) => one.folderId === folderId);
        if (folder) return folderName(folder, t);
      }
      return null;
    })
    .catch(() => null);
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
