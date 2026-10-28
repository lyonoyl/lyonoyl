import { useCallback, useEffect, useRef, useState } from "react";

import { questionKey } from "@/entities/mailbox";
import {
  getConversation,
  getViewWindow,
  isCommandError,
} from "@/shared/ipc/client";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { ListRowDto } from "@/shared/ipc/generated/ListRowDto";
import type { ViewScopeDto } from "@/shared/ipc/generated/ViewScopeDto";

/**
 * Loads the message list in windows, on demand.
 *
 * The list addresses rows by position, so the virtualizer can jump anywhere and this hook fetches
 * only the pages that became visible. Rows already fetched are kept until the set changes.
 */

/** Rows per request. Large enough that scrolling rarely waits, small enough to stay responsive. */
/**
 * The messages of an open conversation, under the name the conversation keeps.
 *
 * Not under the row's position and not under the message the row shows: a reply arriving moves the
 * row and makes a different message its newest, and both of those were the thing being held on to.
 * Where the row sits is asked for when it is drawn, from the rows already loaded.
 */
export type OpenConversation = ListRowDto[];

export const PAGE_SIZE = 100;

export interface ViewQuery {
  scope: ViewScopeDto;
  unreadOnly: boolean;
}

export interface MessageWindows {
  /** Size of the whole set, not of what is loaded. */
  total: number;
  /**
   * Accounts whose mail could not be read for the page that arrived last.
   *
   * Their mail is missing from the rows and from the total. Saying nothing would be a list quietly
   * short of an account, which is worse than a list that says which one.
   */
  notAnswering: string[];
  status: "loading" | "ready" | "failed";
  error: CommandError | null;
  /** The row at that position, or `undefined` while its page is still on the way. */
  rowAt: (index: number) => ListRowDto | undefined;
  /**
   * The messages of a conversation, once it has been opened. Empty while they are on the way.
   *
   * Kept here rather than in the list, so opening a conversation, scrolling away and coming back
   * does not ask for it again.
   */
  opened: Map<string, OpenConversation>;
  /** Where the row standing for a conversation sits, or nothing while no loaded row names it. */
  positionOfConversation: (conversationId: string) => number | null;
  /** Opens the conversation the row at `at` stands for, or closes it if it is already open. */
  toggleConversation: (conversationId: string, messageId: string) => void;
  /** Asks for the pages covering `[start, end]`. Safe to call on every scroll frame. */
  requestRange: (start: number, end: number) => void;
  /**
   * Shows a message as read, or unread, without asking the core again.
   *
   * The core already answers with what a person has asked for laid over what the server knows, so
   * a reload would show the same thing — this only spares the round trip, and spares it at the one
   * moment where a delay is felt: right under the finger that just clicked.
   *
   * Called again with the previous value when the write turns out to have failed.
   */
  showRead: (messageId: string, read: boolean) => void;
  /**
   * The read state `showRead` last showed for this message has been written down, or turned out
   * not to be.
   *
   * Until then every answer is corrected to it: an answer the core gave before the write landed
   * says what the message was, and laid over the rows as it came it would put the dot back under
   * a message just opened. An answer asked for after this is the core's word, taken as it is.
   */
  readWritten: (messageId: string) => void;
  /**
   * Whether every page covering these rows has come back and none is still on its way — which is
   * when a message not among them is not among them rather than not yet read.
   */
  holdsAll: (start: number, end: number) => boolean;
  /** Asks for what is held to be read again, for when the index changed under it. */
  reread: () => void;
  /**
   * Takes a message out of the list at once, before any server has been asked.
   *
   * Filing a message has to close the list up under the finger that filed it. Waiting for the
   * server would leave the message sitting there for as long as the round takes, which is the one
   * thing a person reads as "that did nothing".
   *
   * The position is remembered rather than the cache rewritten: pages are addressed by position,
   * and rewriting one would move every row of every page after it.
   */
  showGone: (messageId: string) => void;
  /** Puts one back, for a filing that turned out not to have happened. */
  showBack: (messageId: string) => void;
  /** Where a message sits now, or `null` if no loaded page holds it. */
  positionOf: (messageId: string) => number | null;
}

export function useMessageWindows(query: ViewQuery): MessageWindows {
  // The scope is an object rebuilt on every render; what identifies it is its contents — all of
  // them, the words being looked for included, or a search would be asked once and never again.
  const asked = questionKey(query.scope);
  const [total, setTotal] = useState(0);
  const [notAnswering, setNotAnswering] = useState<string[]>([]);
  const [opened, setOpened] = useState<Map<string, OpenConversation>>(
    new Map(),
  );
  const [status, setStatus] = useState<MessageWindows["status"]>("loading");
  const [error, setError] = useState<CommandError | null>(null);
  /** Bumped whenever loaded rows change, so the list re-renders without copying the cache. */
  const [revision, setRevision] = useState(0);

  const pages = useRef(new Map<number, ListRowDto[]>());
  /** Conversations already read, by conversation, so opening one a second time costs nothing. */
  const alreadyRead = useRef(new Map<string, ListRowDto[]>());
  /** Which conversations are open, by conversation, readable the instant a row is pressed. */
  const open = useRef(new Map<string, OpenConversation>());
  /**
   * Rows taken out of the list here and not yet out of the index, by the position they had.
   *
   * Sorted, so working out which stored row a position now stands for is a walk along a list that
   * almost always has one thing in it. Emptied when the server's answer changes shape, which is
   * the moment its own answer stops counting them.
   */
  const gone = useRef<{ at: number; messageId: string }[]>([]);
  const inFlight = useRef(new Set<number>());
  /** How many pages have been asked for, which orders an answer against a write. */
  const asks = useRef(0);
  /**
   * What `showRead` showed, by message, with the ask from which the core's answer is to be taken
   * instead — `null` while the write is still on its way.
   */
  const shownRead = useRef(
    new Map<string, { unread: boolean; trustFrom: number | null }>(),
  );
  const epoch = useRef<string | null>(null);
  /** Which set the cache belongs to. A request that returns after this changed is discarded. */
  const generation = useRef(0);

  /**
   * Settles what has been filed here against what the index now says.
   *
   * A row filed here stays in the index until the server answers, and the epoch moves for any
   * change in the folder at all — so emptying this on a changed epoch would put the row somebody
   * just archived straight back into the list, in the second before the server catches up.
   *
   * So each answer is asked about its own rows instead: a message still in them has only moved,
   * and keeps its place here under its new position; a message the answer no longer holds at the
   * place it had is one the server has dealt with, and is forgotten. Anything outside the rows this
   * answer covers is not this answer's business.
   */
  const settle = useCallback((page: number, rows: ListRowDto[]) => {
    const from = page * PAGE_SIZE;
    const to = from + rows.length;

    gone.current = gone.current
      .flatMap((one) => {
        const at = rows.findIndex((row) => row.messageId === one.messageId);
        if (at !== -1) return [{ at: from + at, messageId: one.messageId }];
        return one.at >= from && one.at < to ? [] : [one];
      })
      .sort((one, other) => one.at - other.at);
  }, []);

  /**
   * Rows as an answer to the ask numbered `ask` gives them, with what a person marked and the core
   * had not yet written down when it was asked corrected to what they marked.
   */
  const asMarked = useCallback((rows: ListRowDto[], ask: number) => {
    if (shownRead.current.size === 0) return rows;
    return rows.map((row) => {
      const shown = shownRead.current.get(row.messageId);
      if (shown === undefined) return row;
      if (shown.trustFrom !== null && ask >= shown.trustFrom) {
        shownRead.current.delete(row.messageId);
        return row;
      }
      return row.unread === shown.unread
        ? row
        : { ...row, unread: shown.unread };
    });
  }, []);

  /**
   * Reads a conversation and keeps it under the name the conversation keeps.
   *
   * Asked by the message its row shows, because that is the only way in — the conversation's own
   * name is what the answer is filed under here, not what the index is asked by.
   */
  const readConversation = useCallback(
    (conversationId: string, messageId: string) => {
      asks.current += 1;
      const ask = asks.current;
      getConversation(messageId)
        .then((answer) => {
          const messages = asMarked(answer, ask);
          alreadyRead.current.set(conversationId, messages);
          if (!open.current.has(conversationId)) return;
          open.current.set(conversationId, messages);
          setOpened(new Map(open.current));
        })
        .catch(() => {
          // Nothing to show under a row that says it is open. Closing it again is the honest answer
          // — but one already showing its messages keeps them: a refresh that failed is not a reason
          // to take away what was read.
          if ((open.current.get(conversationId) ?? []).length > 0) return;
          open.current.delete(conversationId);
          setOpened(new Map(open.current));
        });
    },
    [asMarked],
  );

  const loadPage = useCallback(
    async (page: number, again = false) => {
      if ((pages.current.has(page) && !again) || inFlight.current.has(page))
        return;
      inFlight.current.add(page);
      const requested = generation.current;
      const known = epoch.current;
      asks.current += 1;
      const ask = asks.current;

      try {
        const window = await getViewWindow({
          scope: query.scope,
          unreadOnly: query.unreadOnly,
          offset: page * PAGE_SIZE,
          limit: PAGE_SIZE,
        });

        // The answer describes a set the caller has already left; storing it would show one
        // folder's messages under another folder's heading.
        if (requested !== generation.current) return;

        // An answer about a set that has already moved on. This happens when a page asked for
        // before the change lands after one asked for since: storing it would put the old rows
        // back and set the epoch backwards, and nothing would ask for the good ones again.
        if (
          known !== null &&
          known !== epoch.current &&
          window.viewEpoch !== epoch.current
        ) {
          return;
        }

        // A changed epoch means rows moved. What is cached describes positions that no longer
        // hold, so it is dropped and the visible range is asked for again.
        //
        // What is open is not dropped with it. A conversation is held by its own name, which the
        // rows carry and which surviving mail does not change — so a reply arriving redraws the
        // conversation it landed in instead of closing every conversation on the screen. What was
        // read of each is kept too: the rows say how many messages each conversation holds now, and
        // only one whose count has moved is asked for again.
        if (epoch.current !== null && epoch.current !== window.viewEpoch) {
          pages.current.clear();
        }
        epoch.current = window.viewEpoch;

        const rows = asMarked(window.rows, ask);
        pages.current.set(page, rows);

        // A conversation open on the screen whose row now counts a different number of messages
        // has gained or lost one, and what is showing under it is an answer out of date. That one
        // is read again and the others are left alone — a hundred accounts moving their epochs
        // between them must not turn every arrival into a question about every open conversation.
        for (const row of rows) {
          const conversation = row.conversationId;
          if (conversation === null) continue;
          const showing = open.current.get(conversation);
          if (!showing?.length || showing.length === row.inConversation)
            continue;
          readConversation(conversation, row.messageId);
        }

        settle(page, rows);
        setTotal(window.total);
        setNotAnswering(window.notAnswering);
        setStatus("ready");
        setError(null);
      } catch (reason: unknown) {
        if (requested !== generation.current) return;
        setStatus("failed");
        setError(
          isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        );
      } finally {
        if (requested === generation.current) {
          inFlight.current.delete(page);
          setRevision((value) => value + 1);
        }
      }
    },
    [asked, query.unreadOnly, settle, readConversation, asMarked],
  );

  // A different query is a different set: nothing cached applies to it.
  useEffect(() => {
    generation.current += 1;
    pages.current.clear();
    alreadyRead.current.clear();
    open.current.clear();
    setOpened(new Map());
    gone.current = [];
    inFlight.current.clear();
    shownRead.current.clear();
    epoch.current = null;
    setStatus("loading");
    setRevision((value) => value + 1);
    void loadPage(0);
  }, [loadPage]);

  const requestRange = useCallback(
    (start: number, end: number) => {
      const firstPage = Math.max(0, Math.floor(start / PAGE_SIZE));
      const lastPage = Math.floor(Math.max(start, end) / PAGE_SIZE);
      for (let page = firstPage; page <= lastPage; page += 1) {
        void loadPage(page);
      }
    },
    [loadPage],
  );

  /**
   * The stored position a shown position stands for.
   *
   * Every row taken out at or before it moves it on by one, which is the same arithmetic as
   * closing a gap in an array — done here rather than in the pages, because a page is addressed by
   * where it starts and rewriting one would move every page after it.
   */
  const storedAt = useCallback((shown: number) => {
    let stored = shown;
    for (const { at } of gone.current) {
      if (at > stored) break;
      stored += 1;
    }
    return stored;
  }, []);

  const rowAt = useCallback(
    (index: number) => {
      // `revision` is a dependency so that callers re-render when a page arrives.
      void revision;
      const stored = storedAt(index);
      return pages.current.get(Math.floor(stored / PAGE_SIZE))?.[
        stored % PAGE_SIZE
      ];
    },
    [revision, storedAt],
  );

  /**
   * Reads the pages already held again, because something changed in the index behind them.
   *
   * Held rather than dropped first: dropping them would empty the list for as long as the answer
   * takes, and what a person is looking at would turn to placeholders under them. What the answer
   * carries is the epoch, and a changed epoch is what clears the cache — so a pass that changed
   * nothing here costs a read and nothing else.
   */
  const reread = useCallback(() => {
    const held = [...pages.current.keys()];

    // Nothing held means the first read never landed — a failed one, or one still on the wire.
    // Re-reading nothing would leave the failure on screen for the rest of the session.
    if (held.length === 0) {
      void loadPage(0, true);
      return;
    }

    for (const page of held) {
      void loadPage(page, true);
    }
  }, [loadPage]);

  /** Where the first row answering to `is` sits among the rows that are shown. */
  const positionWhere = useCallback((is: (row: ListRowDto) => boolean) => {
    for (const [page, rows] of pages.current) {
      const at = rows.findIndex(is);
      if (at === -1) continue;

      const stored = page * PAGE_SIZE + at;
      const removed = gone.current.filter((one) => one.at <= stored).length;
      return stored - removed;
    }

    return null;
  }, []);

  /** Where a message sits among the rows that are shown, counting the ones already taken out. */
  const positionOf = useCallback(
    (messageId: string) => {
      // `revision` is a dependency so that callers re-render when a page arrives.
      void revision;
      return positionWhere((row) => row.messageId === messageId);
    },
    [revision, positionWhere],
  );

  /**
   * Where the row standing for a conversation sits, or nothing while no loaded row names it.
   *
   * Asked on every draw rather than remembered: a position is not a name. Mail arriving moves every
   * row under it, filing one here closes the gap, and a conversation held by the place it had
   * would draw its messages under whatever moved into that place.
   */
  const positionOfConversation = useCallback(
    (conversationId: string) => {
      void revision;
      return positionWhere((row) => row.conversationId === conversationId);
    },
    [revision, positionWhere],
  );

  const showGone = useCallback((messageId: string) => {
    if (gone.current.some((one) => one.messageId === messageId)) return;

    for (const [page, rows] of pages.current) {
      const at = rows.findIndex((row) => row.messageId === messageId);
      if (at === -1) continue;

      const stored = page * PAGE_SIZE + at;
      gone.current = [...gone.current, { at: stored, messageId }].sort(
        (one, other) => one.at - other.at,
      );
      setRevision((value) => value + 1);
      return;
    }
  }, []);

  const showBack = useCallback((messageId: string) => {
    const left = gone.current.filter((one) => one.messageId !== messageId);
    if (left.length === gone.current.length) return;

    gone.current = left;
    setRevision((value) => value + 1);
  }, []);

  const showRead = useCallback((messageId: string, read: boolean) => {
    shownRead.current.set(messageId, { unread: !read, trustFrom: null });

    for (const [page, rows] of pages.current) {
      const at = rows.findIndex((row) => row.messageId === messageId);
      if (at === -1) continue;

      const changed = [...rows];
      changed[at] = { ...rows[at], unread: !read } as ListRowDto;
      pages.current.set(page, changed);
    }

    // And wherever it hangs under a conversation. A message opened inside one is drawn from what
    // was read of that conversation, not from a page, and its dot goes there too or it stays
    // until the conversation is read again.
    const marked = (messages: ListRowDto[]) =>
      messages.map((row) =>
        row.messageId === messageId ? { ...row, unread: !read } : row,
      );
    for (const [conversation, messages] of alreadyRead.current) {
      if (messages.some((row) => row.messageId === messageId))
        alreadyRead.current.set(conversation, marked(messages));
    }
    let opening = false;
    for (const [conversation, messages] of open.current) {
      if (!messages.some((row) => row.messageId === messageId)) continue;
      open.current.set(conversation, marked(messages));
      opening = true;
    }
    if (opening) setOpened(new Map(open.current));

    setRevision((value) => value + 1);
  }, []);

  const readWritten = useCallback((messageId: string) => {
    const shown = shownRead.current.get(messageId);
    if (shown !== undefined) shown.trustFrom = asks.current + 1;
  }, []);

  const holdsAll = useCallback(
    (start: number, end: number) => {
      void revision;
      for (
        let page = Math.floor(storedAt(start) / PAGE_SIZE);
        page <= Math.floor(storedAt(end) / PAGE_SIZE);
        page += 1
      ) {
        if (!pages.current.has(page) || inFlight.current.has(page))
          return false;
      }
      return true;
    },
    [revision, storedAt],
  );

  /**
   * Opens a conversation, or closes one that is open.
   *
   * The messages are asked for once and kept: a conversation opened, scrolled past and come back to
   * is the same conversation, and asking again would redraw rows that never changed.
   */
  const toggleConversation = useCallback(
    (conversationId: string, messageId: string) => {
      // Whether it is open is read from the ref rather than from inside the state update: React
      // runs that update later, and by then this has already had to decide whether to ask.
      if (open.current.has(conversationId)) {
        open.current.delete(conversationId);
        setOpened(new Map(open.current));
        return;
      }

      // A conversation read once opens again with no wait and no flicker: it is shown from what is
      // remembered, and the fresh answer replaces it when it lands. Opened holding nothing when
      // there is nothing remembered — the row still turns the moment it is pressed.
      const remembered = alreadyRead.current.get(conversationId);
      open.current.set(conversationId, remembered ?? []);
      setOpened(new Map(open.current));

      if (remembered === undefined) readConversation(conversationId, messageId);
    },
    [readConversation],
  );

  return {
    // What is shown is what the index holds, less what has just been filed out of it.
    total: Math.max(0, total - gone.current.length),
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
  };
}
