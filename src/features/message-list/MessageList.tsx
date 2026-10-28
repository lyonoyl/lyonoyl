import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import {
  listsConversations,
  questionKey,
  type LeftAt,
} from "@/entities/mailbox";
import { listOf, useI18n } from "@/shared/i18n";
import { ListSaysNothing } from "./ListSaysNothing";
import {
  archiveMessage,
  deleteMessage,
  markMessageRead,
  tryActionAgain,
} from "@/shared/ipc/client";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import {
  ACTS_ON_THE_CHOSEN,
  addedBy,
  firstChoice,
  itemAt,
  jumpedTo,
  nextAfterLeaving,
  placeOf,
  rowsOnScreen,
  unfoldedFrom,
  whereItCouldBe,
} from "./places";
import {
  insideId,
  LIST_GUTTER,
  LIST_ID,
  ROW_HEIGHT,
  rowDates,
  rowId,
} from "./rows";

/** A list of nothing, for the rows on a screen before there is a screen. */
const LIST_SHAPE = { places: 0, height: 1, gutter: 0 };
import { InsideRow, LEAVING_MS, Row, rowWords } from "./MessageRow";
import type { ViewScopeDto } from "@/shared/ipc/generated/ViewScopeDto";
import { useMessageWindows } from "./useMessageWindows";


/**
 * The message list.
 *
 * Only the visible rows exist in the DOM, so a hundred thousand messages cost the same as fifty.
 * Focus stays on the list itself and the active row is named by `aria-activedescendant`: a row that
 * scrolls out is unmounted, and focus placed on it would be lost with it.
 */

export function MessageList({
  scope,
  density = "comfortable",
  unreadOnly = false,
  emptyState,
  quietWhenEmpty = false,
  onAnswered,
  onOpen,
  filed = null,
  onHolds,
  generation = 0,
  onFiled,
  returnTo,
  onPlace,
  onMarking,
  chosenElsewhere = 0,
}: {
  scope: ViewScopeDto;
  density?: DensityDto;
  unreadOnly?: boolean;
  /** Shown instead of the plain "folder is empty" line when the caller has something better. */
  emptyState?: ReactNode;
  /**
   * Draws nothing at all when there is nothing, rather than saying so.
   *
   * For a caller that shows this list beside something else and says "empty" once for both: two
   * lines saying a folder is empty, or one saying it over drafts that are right there above it,
   * is a folder contradicting itself.
   */
  quietWhenEmpty?: boolean;
  /**
   * Told whether the list has answered yet, with what it holds or with a refusal.
   *
   * `onHolds` is unsaid for both a list still asking and one that was refused. A caller that waits
   * for this before drawing something beside it has to tell the two apart, or a refusal would keep
   * it waiting for ever.
   */
  onAnswered?: (answered: boolean) => void;
  /**
   * The message the person is now reading, and the folder they are reading it in.
   *
   * The folder travels with it because nothing downstream can work it out: a message may sit in an
   * inbox and in a folder somebody made, and the copy that is acted on has to be the one on the
   * screen. Asked of the message itself, the answer was whichever placement the index listed first.
   */
  onOpen?: (
    opened: { messageId: string; folderId: FolderIdDto | null } | null,
  ) => void;
  /**
   * A message filed away or thrown out somewhere else in the window, and what became of it — or,
   * `unread`, one marked read by being opened and put back from the notice under the list.
   *
   * The buttons that do it live on the message being read, and the list is what has to close up
   * afterwards. Told rather than asked: waiting for the server to say so is the wait a person
   * reads as nothing having happened.
   */
  filed?: {
    messageId: string;
    state: "gone" | "back" | "unread";
    folderId?: FolderIdDto;
  } | null;
  /**
   * How many this list holds, once it knows.
   *
   * Told to whoever draws around it rather than asked for again: the pane beside this one says
   * something different when there is nothing to choose, and the trash offers to empty itself
   * only when it has something in it. Both are the same number this already has.
   */
  onHolds?: (holds: number | null) => void;
  /** Bumped by the caller when the index changed, which is when this reads what it holds again. */
  generation?: number;
  /**
   * Told when a message was thrown away from here, in the same words the reader uses.
   *
   * What comes back through `filed` is what closes the list up over it, so a message thrown away
   * by a key and one thrown away by a button leave in exactly the same way.
   */
  onFiled?: (filed: {
    messageId: string;
    folderId: FolderIdDto;
    what: "deleted" | "read";
  }) => void;
  /**
   * Where the person left this mailbox, read once when the list appears: the message they had open
   * is shown chosen again, and nothing is opened or marked by it; the list is scrolled back.
   */
  returnTo?: LeftAt;
  /** Told where the person is in this mailbox, whenever that changes, so it can be returned to. */
  onPlace?: (left: LeftAt) => void;
  /**
   * Told of a message marked read or unread the moment its dot changes, with the write that says
   * so: the counts around the list move in the same frame, and back if the write fails.
   */
  onMarking?: (
    folderId: FolderIdDto,
    read: boolean,
    written: Promise<number>,
  ) => void;
  /**
   * Bumped by the caller when the person chose something this list does not hold, which the
   * reader then shows. The list lets go of its own choice: a row still drawn as chosen says the
   * reader is showing it.
   */
  chosenElsewhere?: number;
}) {
  const { t, language, dir } = useI18n();
  const query = useMemo(() => ({ scope, unreadOnly }), [scope, unreadOnly]);
  const {
    total,
    opened,
    positionOfConversation,
    toggleConversation,
    notAnswering,
    status,
    error,
    rowAt,
    requestRange,
    showRead,
    readWritten,
    holdsAll,
    reread,
    showGone,
    showBack,
    positionOf,
  } = useMessageWindows(query);

  // What it holds, said outward whenever it changes and unsaid while it is unknown: a pane that
  // was told "nothing" during a round trip says the wrong thing for as long as the round trip
  // takes, on every folder somebody opens. Said before the frame with these rows is painted, so
  // whatever is drawn around the list from that number is drawn in the same frame: the folder for
  // sent mail takes away its own row of a message in the frame the list draws that message's copy.
  useLayoutEffect(() => {
    onHolds?.(status === "ready" ? total : null);
  }, [onHolds, status, total]);

  useEffect(() => {
    onAnswered?.(status !== "loading");
  }, [onAnswered, status]);

  // Read again when the core says the index changed, rather than built again. Building it again
  // would be simpler and would throw away the row somebody is on and the place they had scrolled
  // to — every time a background pass brought in a message, and now on every message they open.
  const firstGeneration = useRef(generation);
  useEffect(() => {
    if (generation === firstGeneration.current) return;
    reread();
  }, [generation, reread]);

  // Every word a row shows, once for the whole list rather than once a row.
  const words = useMemo(() => rowWords(t), [t]);

  // What a person does with the explanation on a row. The window is not told the outcome: the row
  // stops explaining itself when the next window says so, which is when the core has actually put
  // the instruction back.
  const tryAgain = useCallback(
    (messageId: string, folderId: FolderIdDto) => {
      tryActionAgain(messageId, folderId)
        // Back in the queue, which is what the next read shows: on its way rather than stopped.
        .then(reread)
        .catch(() => {
          /* The explanation stays on the row, which is the honest answer to "it did not happen". */
        });
    },
    [reread],
  );

  /**
   * Throwing a message away, wherever it was asked for.
   *
   * One function rather than one per control: the key and the menu item must do the same work,
   * including the guard that stops a second press while the first is still going and the line
   * that plays the row out of the list. Two copies is how a control that looks the same starts
   * behaving differently.
   */
  const throwAway = useCallback(
    (messageId: string, folderId: FolderIdDto) => {
      if (throwing.current) return;
      throwing.current = true;

      deleteMessage(messageId, folderId, false)
        .then(() => onFiled?.({ messageId, folderId, what: "deleted" }))
        .catch(() => {
          /* The row stays. What could not be done is said on the row by the next window. */
        })
        .finally(() => {
          throwing.current = false;
        });
    },
    [onFiled],
  );

  /** The same, for filing a message where the account keeps what it is done with. */
  const archive = useCallback(
    (messageId: string, folderId: FolderIdDto) => {
      archiveMessage(messageId, folderId)
        .then(() => onFiled?.({ messageId, folderId, what: "deleted" }))
        .catch(() => {
          /* Said on the row by the next window, the same way a refused deletion is. */
        });
    },
    [onFiled],
  );

  // Shown before it is written down, and put back if the writing fails. The core answers with the
  // same thing once it has it — it lays what a person asked for over what the server knows — so
  // this only spares the round trip, at the one moment where a delay is felt.
  const toggleRead = useCallback(
    (messageId: string, folderId: FolderIdDto, read: boolean) => {
      showRead(messageId, read);

      const written = markMessageRead(messageId, folderId, read);
      onMarking?.(folderId, read, written);
      written
        .catch(() => showRead(messageId, !read))
        .finally(() => readWritten(messageId));
    },
    [showRead, readWritten, onMarking],
  );

  const viewport = useRef<HTMLDivElement>(null);
  /**
   * The row the person chose, or none — which is where a list starts: coming into a mailbox is not
   * reading anything in it, so nothing is chosen until somebody chooses.
   */
  const [active, setActive] = useState<number | null>(null);
  /**
   * Which message of the conversation open under the selected row is selected, or none.
   *
   * Kept beside the row rather than folded into one number: a conversation opening anywhere above
   * moves every place in the list down without moving a single row, and a selection counted in
   * places would then name a different message than the one it was on.
   */
  const [inside, setInside] = useState<number | null>(null);
  /**
   * Which way the person is going through the list.
   *
   * What decides where the selection lands when the message they are on is filed away: the next
   * one the way they were already going. Going the other way would turn a pass down a folder into
   * a jump back to where they started.
   */
  const heading = useRef<"up" | "down">("down");
  /** Rows playing their way out, kept in the list until the animation has finished. */
  const [going, setGoing] = useState<string[]>([]);
  /**
   * One wait per row on its way out, by message.
   *
   * Not one per effect: filing a second message before the first has finished sliding would then
   * cancel the first one's wait, and that row would never leave the list at all — invisible, still
   * counted, still taking its place.
   */
  const settling = useRef(new Map<string, number>());
  /** Whether a key press is still being answered, so held-down keys throw one message away. */
  const throwing = useRef(false);
  useEffect(
    () => () => {
      for (const wait of settling.current.values()) {
        window.clearTimeout(wait);
      }
    },
    [],
  );

  // Where each open conversation sits, and how many messages it added under it.
  const unfolded = useMemo(
    () => unfoldedFrom(opened, positionOfConversation),
    [opened, positionOfConversation],
  );

  const added = addedBy(unfolded);

  /** The messages showing under a row, and nothing where its conversation is closed. */
  const messagesUnder = useCallback(
    (index: number) => {
      const conversation = rowAt(index)?.conversationId;
      return (
        (conversation === null || conversation === undefined
          ? undefined
          : opened.get(conversation)) ?? []
      );
    },
    [rowAt, opened],
  );

  const virtualizer = useVirtualizer({
    count: total + added,
    getScrollElement: () => viewport.current,
    estimateSize: () => ROW_HEIGHT[density],
    overscan: 12,
    paddingStart: LIST_GUTTER,
    paddingEnd: LIST_GUTTER,
  });

  /**
   * The message the reader shows, as far as this list knows, with the folder it is in and the
   * conversation it was chosen inside, if it was. Announced once, so a re-render does not open the
   * same one again.
   */
  const announced = useRef<string | null>(returnTo?.messageId ?? null);
  const announcedIn = useRef<FolderIdDto | null>(returnTo?.folderId ?? null);
  const announcedUnder = useRef<string | null>(
    returnTo?.conversationId ?? null,
  );
  /**
   * The row the chosen message was last seen at. A list reading its pages again holds none of them
   * for a moment, and a message not found then is only not found yet; once the page holding this
   * row has answered without it, it has gone.
   */
  const seenAt = useRef<number | null>(returnTo?.at ?? null);
  /** How many the list held then, which says how far it can have moved since. */
  const heldThen = useRef<number | null>(returnTo?.held ?? null);
  /**
   * Where a move somebody made was going, while it is still on its way: the difference between a
   * person opening a message and the list moving under them. `null` means nobody moved. A position
   * is not an identity — mail arriving moves every row down one — so what is spent here is the
   * destination, and only by the pass that actually arrives there.
   */
  const went = useRef<{ index: number; at: number | null } | null>(null);
  /** Counts every choice, so one that moves nothing still reaches the effect that spends `went`. */
  const [choices, setChoices] = useState(0);

  /** The rows on the screen now, read from where it is scrolled to. */
  const onScreen = useRef(() => rowsOnScreen(0, 0, LIST_SHAPE, []));
  onScreen.current = () =>
    rowsOnScreen(
      viewport.current?.scrollTop ?? 0,
      viewport.current?.clientHeight ?? 0,
      {
        places: total + added,
        height: ROW_HEIGHT[density],
        gutter: LIST_GUTTER,
      },
      unfolded,
    );

  /** Where the person is in this mailbox, said to whoever keeps it for next time. */
  const tellPlace = useRef(() => {});
  tellPlace.current = () =>
    onPlace?.({
      messageId: announced.current,
      folderId: announcedIn.current,
      conversationId: announcedUnder.current,
      at: seenAt.current,
      held: heldThen.current,
      top: onScreen.current().top,
    });

  /** Nothing chosen and nothing open, because what was open is not in this list any more. */
  const letGo = useCallback(() => {
    went.current = null;
    announced.current = null;
    announcedIn.current = null;
    announcedUnder.current = null;
    seenAt.current = null;
    setActive(null);
    setInside(null);
    onOpen?.(null);
    tellPlace.current();
  }, [onOpen]);

  // Each bump once, whatever else renders this again: a choice made elsewhere is let go of when it
  // is made, and a row chosen here afterwards stays chosen.
  const choiceElsewhere = useRef(chosenElsewhere);
  useEffect(() => {
    if (chosenElsewhere === choiceElsewhere.current) return;
    choiceElsewhere.current = chosenElsewhere;
    letGo();
  }, [chosenElsewhere, letGo]);

  /**
   * Where a message is shown now: its row, or its place under a conversation that is open. The
   * newest of an open conversation is shown in both, and it is found where the person is: inside,
   * when they are walking through the conversation, or they are thrown back out onto its row.
   */
  const whereIs = useCallback(
    (messageId: string, insideFirst: boolean) => {
      const row = positionOf(messageId);
      if (row !== null && !insideFirst) return { index: row, at: null };

      for (const [conversation, messages] of opened) {
        const at = messages.findIndex((one) => one.messageId === messageId);
        if (at === -1) continue;
        const index = positionOfConversation(conversation);
        if (index !== null) return { index, at };
      }
      if (row !== null) return { index: row, at: null };

      // Inside a conversation that has since closed: its row stands for it. One open and read that
      // no longer lists it says the message has left it; one still being read says nothing yet.
      const under =
        messageId === announced.current ? announcedUnder.current : null;
      const index =
        under === null || (opened.get(under)?.length ?? 0) > 0
          ? null
          : positionOfConversation(under);
      return index === null ? null : { index, at: null };
    },
    [positionOf, positionOfConversation, opened],
  );

  useEffect(() => {
    // Somebody moved, and where they went has to be announced; or nobody did, and whatever is open
    // has to be found again wherever the list has put it.
    const arrived =
      went.current !== null &&
      went.current.index === active &&
      went.current.at === inside;

    if (!arrived) {
      if (announced.current === null) return;

      const found = whereIs(announced.current, inside !== null);
      if (found !== null) {
        // Followed by name, never opened again: it is already the one being read.
        seenAt.current = found.index;
        heldThen.current = total;
        if (found.index !== active) setActive(found.index);
        if (found.at !== inside) setInside(found.at);
        return;
      }

      // Not found. Gone, once every row it could have moved to has been read without it; until
      // then only not found yet, and so is one that could have moved further than is worth
      // reading to find out — the reader goes on showing it with nothing chosen.
      if (seenAt.current === null || status !== "ready") return;
      const moved = total - (heldThen.current ?? total);
      const near = whereItCouldBe(seenAt.current, moved, total);
      if (near === null) return;
      if (holdsAll(near.start, near.end)) letGo();
      else requestRange(near.start, near.end);
      return;
    }

    if (active === null) return;
    const row = rowAt(active);
    // A move to a row whose page is still on its way: announced when it arrives.
    if (row === undefined) return;

    // Standing inside a conversation, what is chosen is the message under the row rather than the
    // row: the row stands for the newest of them, and walking through them has to read each one.
    const chosen =
      inside === null ? row : (messagesUnder(active)[inside] ?? row);

    went.current = null;
    seenAt.current = active;
    heldThen.current = total;
    if (announced.current === chosen.messageId) return;

    // A message somebody went to has been read. The dot beside it is how they say otherwise, and
    // it goes on working.
    //
    // The conversation it was chosen in is kept with it, where the row stands for a conversation:
    // a reply arriving makes the row show another message, and the row still stands for this one.
    announced.current = chosen.messageId;
    announcedIn.current = chosen.folderId;
    announcedUnder.current =
      inside !== null || listsConversations(scope) ? row.conversationId : null;
    onOpen?.({ messageId: chosen.messageId, folderId: chosen.folderId });
    tellPlace.current();

    if (chosen.unread && chosen.folderId !== null) {
      toggleRead(chosen.messageId, chosen.folderId, true);
      // Said out loud, because nobody asked for it. The dot beside the row is the way back and it
      // goes on working, but on a message that is read it is drawn as nothing at all — a way back
      // that has to be known about is one most people do not have.
      onFiled?.({
        messageId: chosen.messageId,
        folderId: chosen.folderId,
        what: "read",
      });
    }
  }, [
    active,
    inside,
    choices,
    messagesUnder,
    rowAt,
    whereIs,
    holdsAll,
    requestRange,
    status,
    total,
    scope,
    letGo,
    onOpen,
    onFiled,
    toggleRead,
  ]);

  // A conversation closes for reasons of its own: the row it hangs under was pressed, or mail
  // arrived and every open one was dropped with the epoch. What was selected inside it is then a
  // message the list does not draw, so the selection comes back out onto the row.
  useEffect(() => {
    if (
      inside !== null &&
      (active === null || inside > messagesUnder(active).length - 1)
    )
      setInside(null);
  }, [inside, active, messagesUnder]);

  // A different question is a different set of rows, and every position held here was a position
  // in the answer to the old one. The list is deliberately not remounted while somebody types —
  // that would take the scroll position and the loaded rows on every letter — so what a remount
  // would have cleared is cleared here instead.
  //
  // What is not cleared is which message is being read: the effect above follows it by name into
  // the new answer, and where it is not there nothing is chosen and the reader goes on showing it.
  // The move flag is cleared too: a message somebody never chose must not be read on the server.
  const askedBefore = useRef(questionKey(scope));
  useEffect(() => {
    const asked = questionKey(scope);
    if (asked === askedBefore.current) return;
    askedBefore.current = asked;
    went.current = null;
    seenAt.current = null;
    setActive(null);
    setInside(null);
    setGoing([]);
    settling.current.clear();
    viewport.current?.scrollTo({ top: 0 });
  }, [scope]);

  // The set can shrink under a selection nobody moved. Left alone, the selection names a row that is
  // not drawn; it lets go of the row, and the effect above finds the message again or lets it go.
  useEffect(() => {
    if (active !== null && active > total - 1) {
      went.current = null;
      setActive(null);
      setInside(null);
    }
  }, [active, total]);

  // Scrolled back to the row the person left at the top, once there is a list long enough to scroll,
  // with the gutter above it that the top of the list has. A list left at its top is where it
  // starts, and is not moved at all.
  const returned = useRef(returnTo === undefined);
  useEffect(() => {
    if (returned.current || status !== "ready") return;
    returned.current = true;
    const top = returnTo?.top ?? 0;
    if (top === 0) return;
    const [offset] = virtualizer.getOffsetForIndex(top, "start") ?? [0];
    virtualizer.scrollToOffset(Math.max(0, offset - LIST_GUTTER));
    // Once, on the first answer; `returnTo` is read when the list appears and not after.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status]);

  /** The scroll, said at most once a frame, and only once the list is back where it was left. */
  const scrolling = useRef<number | null>(null);
  const scrolled = useCallback(() => {
    if (!returned.current || scrolling.current !== null) return;
    scrolling.current = window.requestAnimationFrame(() => {
      scrolling.current = null;
      tellPlace.current();
    });
  }, []);
  useEffect(
    () => () => window.cancelAnimationFrame(scrolling.current ?? 0),
    [],
  );

  const items = virtualizer.getVirtualItems();
  // What is on screen is asked for by row of the list, not by place in it: an open conversation
  // pushes places down without adding rows, and a range in places would ask for the wrong page.
  const firstVisible =
    items.length > 0 ? itemAt(items[0].index, unfolded).index : 0;
  const lastVisible =
    items.length > 0
      ? itemAt(items[items.length - 1].index, unfolded).index
      : 0;

  // Depends on the two positions rather than on the array: the virtualizer returns a new array on
  // every render, and asking for the same range again on every frame is what makes scrolling stutter.
  useEffect(() => {
    if (total === 0) return;
    requestRange(firstVisible, lastVisible);
  }, [firstVisible, lastVisible, requestRange, total]);

  const goTo = useCallback(
    (index: number, at: number | null) => {
      if (total === 0) return;
      const clamped = Math.max(0, Math.min(index, total - 1));
      if (active !== null && clamped !== active) {
        heading.current = clamped > active ? "down" : "up";
      }
      // Said here and nowhere else: everything that calls this is somebody acting. Where the move
      // is going, not merely that there was one: a move sets two pieces of state, and the effect
      // that spends this can run between them. Counted, so it runs even where nothing moved: a
      // flag left armed is spent by whatever moves the rows next and read as an opening, and a row
      // chosen again may stand for another message than the one it shows, which this opens.
      went.current = { index: clamped, at };
      setChoices((count) => count + 1);
      setActive(clamped);
      setInside(at);
      virtualizer.scrollToIndex(
        placeOf(clamped, unfolded) + (at === null ? 0 : at + 1),
      );
    },
    [active, total, virtualizer, unfolded],
  );

  /** Moving to a row of the list leaves whatever conversation was being walked through. */
  const moveTo = useCallback((index: number) => goTo(index, null), [goTo]);

  // Everything the answer below needs, kept where changing it cannot restart the answer.
  //
  // Moving the selection is part of answering, and the selection is what these are built from — so
  // an effect that listed them would clear its own timer the moment it moved anything, and the row
  // it was seeing out would stay in the list for ever.
  const parts = useRef({
    positionOf,
    total,
    moveTo,
    showGone,
    showBack,
    letGo,
  });
  parts.current = { positionOf, total, moveTo, showGone, showBack, letGo };

  // A filing answered once. The same message cannot be filed twice, so the identifier plus what
  // became of it is enough to tell a new answer from a re-render.
  const acted = useRef<string | null>(null);
  useEffect(() => {
    const { positionOf, total, moveTo, showGone, showBack, letGo } =
      parts.current;
    // Nothing to answer forgets the last answer: the same message filed again is a new filing.
    if (!filed) {
      acted.current = null;
      return;
    }
    const signal = `${filed.state}:${filed.messageId}`;
    if (acted.current === signal) return;
    acted.current = signal;

    // The way the dot does it, so the dot and the counts beside the folders move in one frame.
    if (filed.state === "unread") {
      if (filed.folderId) toggleRead(filed.messageId, filed.folderId, false);
      return;
    }

    if (filed.state === "back") {
      // Both halves. A way back taken while the row was still sliding never reached the wait that
      // takes it out of `going`, and a row left in there is painted invisible for ever.
      const waiting = settling.current.get(filed.messageId);
      if (waiting !== undefined) {
        window.clearTimeout(waiting);
        settling.current.delete(filed.messageId);
      }
      showBack(filed.messageId);
      setGoing((rows) => rows.filter((one) => one !== filed.messageId));
      return;
    }

    const at = positionOf(filed.messageId);
    if (at === null) {
      // No loaded page holds it, so there is no position to close up over. The row leaves when the
      // server has answered and the list reads itself again, which is the wait this spares. A
      // conversation's row stood in for it no longer does: it has left the conversation too.
      if (filed.messageId === announced.current) announcedUnder.current = null;
      acted.current = null;
      return;
    }

    // Where the selection goes: the one the way they were going, or the other way at an end.
    const next = nextAfterLeaving(at, total, heading.current);

    setGoing((rows) => [...rows, filed.messageId]);

    // Only when the row filed is the row somebody is on. Filing something else — from another
    // window, or with nothing chosen here at all — must not drag them off where they are.
    const theirs = active !== null && at === active;

    // At once, not when the animation ends: the message being read has to change under the finger
    // that filed this one. The row keeps its place for as long as it is on its way out.
    if (!theirs) {
      // Nothing to move: their row is elsewhere and stays where it is.
    } else if (next !== null) {
      moveTo(next);
    } else {
      // The last message of the folder. Leaving it in the reader would show mail from a folder
      // that no longer has any.
      letGo();
    }

    const wait = window.setTimeout(() => {
      settling.current.delete(filed.messageId);
      showGone(filed.messageId);
      setGoing((rows) => rows.filter((one) => one !== filed.messageId));

      // Every position below the row that left moves up by one, so every selection below it does
      // too — not only the one this chose. Somebody who clicked another row while this one was
      // still sliding would otherwise end up one message further down than they picked.
      //
      // The announce effect would put that right on the next render, because it follows the
      // message rather than the number. Doing it here as well is what keeps a wrong row from
      // being drawn at all in between.
      setActive((was) => (was !== null && was > at ? was - 1 : was));
    }, LEAVING_MS);
    settling.current.set(filed.messageId, wait);
    // `active` is read as it is when the filing is answered, and listing it would restart the
    // answer mid-way, which is the mistake `parts` above exists to avoid.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filed]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      // Only keys pressed on the list itself. Every drawn row holds real buttons — the read dot,
      // the conversation count, "try again" — and a key pressed on one of those bubbles up to
      // here. Without this the list answered for them: Enter was cancelled and re-opened whatever
      // the selection happened to be on, and Delete threw away the selected message rather than
      // the one whose button was under the finger. That is a different message, quietly.
      if (event.target !== event.currentTarget) return;

      // Throwing away what is selected, by the key every system throws things away with: Delete
      // where there is one, and Backspace on a keyboard whose Delete key is that — which is what a
      // Mac has, and why both are read here rather than one being chosen by platform.
      if (event.key === "Delete" || event.key === "Backspace") {
        event.preventDefault();

        // A message inside a conversation is not thrown away from here. Filing plays a row out of
        // the list and takes it out of the count, and a message hanging under a row is neither.
        // Nor is anything thrown away when nothing is chosen, nor when the row chosen stands for
        // the message being read and shows another one: that one nobody is looking at.
        if (inside !== null || active === null) return;

        const row = rowAt(active);
        if (row?.folderId == null || row.messageId !== announced.current)
          return;

        throwAway(row.messageId, row.folderId);
        return;
      }

      // Nothing chosen yet: a key that moves chooses, starting from what is on the screen, and a
      // key that acts on what is chosen has nothing to act on.
      if (active === null) {
        const start = firstChoice(event.key, onScreen.current(), total);
        if (start !== undefined) moveTo(start);
        if (start !== undefined || ACTS_ON_THE_CHOSEN.includes(event.key))
          event.preventDefault();
        return;
      }

      const under = messagesUnder(active);

      // Marking read, without the mouse. The dot on the row does it and is now out of the tab
      // order, so without a key it would be a thing only a mouse can do — and the state it sets
      // is the one a person changes most often after opening a message.
      if (event.key === "u" || event.key === "U") {
        event.preventDefault();

        // The message being read, and never another one a row standing for it happens to show.
        const chosen = inside === null ? rowAt(active) : under[inside];
        if (chosen?.folderId == null || chosen.messageId !== announced.current)
          return;

        // The same toggle the dot is: an unread message is marked read, a read one unread.
        toggleRead(chosen.messageId, chosen.folderId, chosen.unread);
        return;
      }

      // A choice like a click, so what it opens is marked read and remembered the same way.
      if (event.key === "Enter") {
        event.preventDefault();
        goTo(active, inside);
        return;
      }

      // Down walks into an open conversation and out of its far end, so a pass down the list reads
      // every message rather than every newest one. Up comes back the same way, and coming up onto
      // a row that is open lands on its last message instead of stepping over the whole of it.
      if (event.key === "ArrowDown") {
        event.preventDefault();
        if (inside === null) {
          if (under.length > 0) goTo(active, 0);
          else moveTo(active + 1);
        } else if (inside + 1 < under.length) {
          goTo(active, inside + 1);
        } else {
          moveTo(active + 1);
        }
        return;
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        if (inside !== null) {
          goTo(active, inside === 0 ? null : inside - 1);
          return;
        }
        const above = active > 0 ? messagesUnder(active - 1) : [];
        if (above.length > 0) goTo(active - 1, above.length - 1);
        else moveTo(active - 1);
        return;
      }

      // Right opens the conversation under the row and steps into it, left comes back out — each
      // the other way round in a mirrored window. Neither key does anything else, as in a tree.
      if (event.key === (dir === "rtl" ? "ArrowLeft" : "ArrowRight")) {
        event.preventDefault();
        if (inside !== null) return;
        const row = rowAt(active);
        if (row === undefined || row.inConversation < 2) return;
        if (under.length > 0) goTo(active, 0);
        else if (row.conversationId !== null) {
          toggleConversation(row.conversationId, row.messageId);
        }
        return;
      }

      if (event.key === (dir === "rtl" ? "ArrowRight" : "ArrowLeft")) {
        event.preventDefault();
        if (inside !== null) {
          goTo(active, null);
          return;
        }
        const row = rowAt(active);
        if (row?.conversationId != null && under.length > 0) {
          toggleConversation(row.conversationId, row.messageId);
        }
        return;
      }

      const next = jumpedTo(event.key, active, total, {
        height: viewport.current?.clientHeight ?? 0,
        row: ROW_HEIGHT[density],
      });
      if (next === undefined) return;
      event.preventDefault();
      moveTo(next);
    },
    [
      active,
      density,
      dir,
      goTo,
      inside,
      messagesUnder,
      moveTo,
      rowAt,
      throwAway,
      toggleConversation,
      toggleRead,
      total,
    ],
  );

  const dates = useMemo(() => rowDates(language), [language]);

  if (status === "failed" && error) {
    return <ListSaysNothing error={error} t={t} />;
  }

  if (status === "ready" && total === 0) {
    if (quietWhenEmpty) return null;

    return (
      <ListSaysNothing unreadOnly={unreadOnly} t={t}>
        {emptyState}
      </ListSaysNothing>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* An account whose mail could not be read is named rather than left out. A list quietly
          short of an account is a list a person trusts and should not. */}
      {notAnswering.length > 0 && (
        <p
          role="status"
          className="border-b px-3 py-2 text-xs leading-snug text-muted-foreground"
        >
          {t("list.accountNotAnswering", {
            accounts: listOf(language, notAnswering),
          })}
        </p>
      )}
      <div
        ref={viewport}
        id={LIST_ID}
        role="listbox"
        tabIndex={0}
        aria-label={t("list.label")}
        aria-activedescendant={
          total === 0 || active === null
            ? undefined
            : inside === null
              ? rowId(active)
              : insideId(active, inside)
        }
        onKeyDown={onKeyDown}
        onScroll={scrolled}
        // No frame round the column, however the focus arrived: the selected row says where a
        // person is, and the list takes the focus by itself after a message is filed away, so a
        // frame would light up the whole column at the press of a button that has nothing to do
        // with it.
        className="no-gutter min-h-0 flex-1 overflow-auto outline-none"
      >
        <div
          className="relative w-full"
          style={{ height: `${virtualizer.getTotalSize()}px` }}
        >
          {items.map((item) => {
            const { index, at } = itemAt(item.index, unfolded);
            const message =
              at === undefined ? undefined : messagesUnder(index)[at];

            // A message of an open conversation: indented under the row it belongs to, and shown
            // as itself rather than as a conversation — it is one message and stands for one.
            if (message && at !== undefined) {
              return (
                <InsideRow
                  key={`inside-${message.messageId}`}
                  index={index}
                  at={at}
                  of={
                    rowAt(index)?.inConversation ?? messagesUnder(index).length
                  }
                  offset={item.start}
                  message={message}
                  chosen={index === active && at === inside}
                  density={density}
                  dates={dates}
                  words={words}
                  onSelect={goTo}
                />
              );
            }

            return (
              <Row
                // Keyed by the message, not by the position it happens to hold. Keyed by position,
                // a row that leaves is not a row that leaves at all: the box at that place simply
                // starts drawing the next message, and the ones below it jump a row's height in a
                // single frame. Keyed by the message, the same box stays and only its place changes
                // — which is a thing that can be moved to, over time, rather than redrawn.
                key={rowAt(index)?.messageId ?? `waiting-${item.index}`}
                index={index}
                of={total}
                offset={item.start}
                row={rowAt(index)}
                active={index === active}
                // A line is drawn between two rows, so it belongs to neither when one of them is the
                // one chosen: a rule running into a rounded corner is what makes a selection look
                // like a mistake. The last row has nothing under it to be separated from.
                separated={
                  index !== active && index + 1 !== active && index < total - 1
                }
                density={density}
                dates={dates}
                onToggleRead={toggleRead}
                onArchive={archive}
                onThrowAway={throwAway}
                words={words}
                onTryAgain={tryAgain}
                onSelect={moveTo}
                going={going.includes(rowAt(index)?.messageId ?? "")}
                onUnfold={toggleConversation}
                unfolded={opened.has(rowAt(index)?.conversationId ?? "")}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}
