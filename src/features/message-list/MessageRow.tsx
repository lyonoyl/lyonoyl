/**
 * One row of the message list, and the number beside a subject that stands for a conversation.
 *
 * Drawing only. Everything it needs is passed in already decided — which row is selected, what its
 * strings say in this language, whether it is on its way out — so the row stays a thing that draws
 * rather than a thing that decides, and the list beside it stays about behaviour.
 */
import { memo } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/primitives/context-menu";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { ListRowDto } from "@/shared/ipc/generated/ListRowDto";

import { useI18n, type Translator } from "@/shared/i18n";

import { insideId, ROW_HEIGHT, rowId } from "./rows";

/**
 * How long a filed row takes to leave, in milliseconds.
 *
 * The same number twice: the transition it plays, and the wait before the list closes up over it.
 * Read from one place so the row cannot vanish mid-slide or leave a gap standing after it.
 */
export const LEAVING_MS = 200;

/// How many messages a conversation holds, shown only when there is more than one.
///
/// A "1" beside every subject would be noise on every row and information on none. The number is
/// read out for a screen reader, because "3" beside a subject means nothing said aloud.
export function ConversationSize({
  of,
  open,
  onToggle,
}: {
  of: number;
  open: boolean;
  onToggle: () => void;
}) {
  const { t } = useI18n();

  if (of < 2) return null;

  return (
    <button
      type="button"
      // Out of the tab order, not out of reach: the list draws the visible band plus twelve rows
      // either side, so leaving these in it put twenty to forty stops between the list and
      // whatever comes after it — and each one scrolled the list as it took focus, so the wall
      // regrew as somebody walked it. The keyboard opens a conversation with the arrows instead.
      tabIndex={-1}
      aria-expanded={open}
      aria-label={t("list.inConversation", { count: of })}
      // The row underneath opens a message; this opens the conversation. The click must not do both.
      onClick={(event) => {
        event.stopPropagation();
        onToggle();
      }}
      className={`shrink-0 rounded-full px-1.5 text-[0.625rem] leading-[1.125rem] tabular-nums transition-colors ${
        open
          ? "bg-primary text-primary-foreground"
          : "bg-muted text-muted-foreground hover:bg-muted-foreground/20"
      }`}
    >
      {of}
    </button>
  );
}

export const Row = memo(function Row({
  onUnfold,
  unfolded,
  index,
  of,
  offset,
  row,
  active,
  separated,
  density,
  dates,
  words,
  onTryAgain,
  onSelect,
  onToggleRead,
  onArchive,
  onThrowAway,
  going,
}: {
  index: number;
  /** How many rows the list holds, so a row can say where it sits among them. */
  of: number;
  offset: number;
  row: ListRowDto | undefined;
  active: boolean;
  /** Whether this row is on its way out of the list right now. */
  going: boolean;
  /** Whether a line is drawn under this row. */
  separated: boolean;
  density: DensityDto;
  dates: Intl.DateTimeFormat;
  words: RowWords;
  onTryAgain: (messageId: string, folderId: FolderIdDto) => void;
  /**
   * The row was chosen. Choosing is what opens a message, and the list does the opening: it is
   * the one way in, so a message opened is a message marked read, however it was chosen.
   */
  onSelect: (index: number) => void;
  onToggleRead: (
    messageId: string,
    folderId: FolderIdDto,
    read: boolean,
  ) => void;
  /** Opens the conversation this row stands for, or closes it. */
  onUnfold: (conversationId: string, messageId: string) => void;
  /** Whether its messages are showing under it. */
  unfolded: boolean;
  onArchive: (messageId: string, folderId: FolderIdDto) => void;
  onThrowAway: (messageId: string, folderId: FolderIdDto) => void;
}) {
  const {
    placeholder,
    markRead,
    markUnread,
    tryAgain,
    explanations,
    noSubject,
    unknownSender,
    menu,
  } = words;
  const compact = density === "compact";
  // Held in one place because a folder identifier is what the read toggle and the retry link both
  // act on, and reading it again inside a click handler loses what the branch above it proved.
  const inFolder = row?.folderId ?? null;
  const stoppedBecause =
    row?.stopped == null
      ? null
      : (explanations[row.stopped] ?? explanations.theServerRefused);

  const drawn = (
    <div
      id={rowId(index)}
      role="option"
      // Where this row sits, and how many there are. The list draws thirty rows out of forty
      // thousand, so without these a reader counts the ones in the page and says "3 of 30" about
      // a message three thousand rows down — a number that is wrong and sounds authoritative.
      aria-posinset={index + 1}
      aria-setsize={of}
      aria-selected={active}
      aria-busy={row === undefined}
      onClick={() => onSelect(index)}
      style={{
        transform: `translateY(${offset}px)`,
        height: `${ROW_HEIGHT[density]}px`,
      }}
      // The outer box is the virtualizer's: exactly where the row goes and exactly how tall it is.
      // Everything visible lives one level in, so the geometry a hundred thousand rows depend on is
      // never a thing a change of appearance can move.
      //
      // The line between two rows is drawn here, inset to the same width as the highlight and only
      // where one belongs. A pseudo-element rather than an element: this renders per visible row,
      // and a rule is not worth a node.
      data-separated={separated ? "true" : undefined}
      // The place is animated, the drawing is not. Scrolling never changes a row's place — the
      // virtualizer lays every row out against the whole list, and moving the viewport moves no
      // row — so this only ever runs when the list itself gained or lost something above this row.
      className="absolute inset-x-0 top-0 cursor-default px-2 py-px transition-transform duration-200 ease-out motion-reduce:transition-none after:absolute after:inset-x-4 after:bottom-0 after:hidden after:h-px after:bg-border/60 after:content-[''] data-[separated]:after:block"
    >
      <div
        // The highlight: inset from the edges and rounded to the same corner the chosen account in
        // the sidebar has, so "this is the one you are on" looks the same wherever it is said.
        //
        // Two ways of being on the way out. One this window did, which slides off and takes its
        // place with it — the movement is what says "that worked" before any server has answered.
        // One somebody else's window did, or this one before it was last closed: that message is
        // still where it was and cannot be moved out from under the reader, so it only fades.
        className={`flex h-full gap-3 overflow-hidden rounded-md px-2 transition-[opacity,translate] duration-200 ease-out motion-reduce:transition-none ${
          compact ? "items-center py-2" : "items-start pt-3 pb-[0.5625rem]"
        } ${active ? "bg-muted" : ""} ${
          going
            ? "translate-x-12 opacity-0 rtl:-translate-x-12"
            : // On its way out, or on its way in and not carried yet. Faint either way: the row
              // is true about what was asked for and not yet true about where the message is,
              // and that is one state to a person looking at it.
              row?.leaving || row?.arriving
              ? "opacity-40"
              : ""
        }`}
      >
        {row === undefined ? (
          <span className="self-center text-xs text-muted-foreground">
            {placeholder}
          </span>
        ) : (
          <>
            {/* The dot says whether a message has been read, and is also how that is changed:
                the thing showing the state is the thing that sets it, which is one fewer place to
                look. The click is kept off the row, because opening a message is what marks it
                read by accident. */}
            {inFolder === null ? (
              <span
                aria-hidden="true"
                className={`size-2 shrink-0 rounded-full ${compact ? "" : "mt-1.5"} ${
                  row.unread ? "bg-primary" : "bg-transparent"
                }`}
              />
            ) : (
              <button
                type="button"
                tabIndex={-1}
                aria-label={row.unread ? markRead : markUnread}
                onClick={(event) => {
                  event.stopPropagation();
                  onToggleRead(row.messageId, inFolder, row.unread);
                }}
                className={`size-2 shrink-0 rounded-full transition-colors ${
                  compact ? "" : "mt-1.5"
                } ${
                  row.unread
                    ? "bg-primary hover:bg-primary/70"
                    : "bg-transparent hover:bg-muted-foreground/40"
                }`}
              />
            )}

            {compact ? (
              <>
                <span
                  dir="auto"
                  className="w-48 shrink-0 truncate text-xs text-muted-foreground"
                >
                  {row.sender ?? unknownSender}
                </span>
                <span
                  dir="auto"
                  className={`flex-1 truncate text-xs ${row.unread ? "font-semibold" : ""}`}
                >
                  {row.subject ?? noSubject}
                </span>
                <ConversationSize
                  of={row.inConversation}
                  open={unfolded}
                  onToggle={() => {
                    if (row.conversationId !== null) {
                      onUnfold(row.conversationId, row.messageId);
                    }
                  }}
                />
                {/* Something a person asked for could not be carried out. Drawn here as well
                    as in the roomy row, because a row that is 40 pixels tall is still a row
                    somebody pressed a key on — and left out, a failed action was invisible in
                    this density with no way at all to ask again. One line, so it fits: what
                    stopped it, truncated, and the button beside it. */}
                {row.stopped !== null && inFolder !== null && (
                  <span className="flex min-w-0 shrink items-center gap-1.5 text-[0.6875rem] text-destructive">
                    <span className="truncate">{stoppedBecause}</span>
                    <button
                      type="button"
                      tabIndex={-1}
                      onClick={(event) => {
                        event.stopPropagation();
                        onTryAgain(row.messageId, inFolder);
                      }}
                      className="shrink-0 underline underline-offset-2 hover:no-underline"
                    >
                      {tryAgain}
                    </button>
                  </span>
                )}
                <time
                  dateTime={new Date(row.receivedAt * 1000).toISOString()}
                  className="shrink-0 text-[0.6875rem] text-muted-foreground"
                >
                  {dates.format(new Date(row.receivedAt * 1000))}
                </time>
              </>
            ) : (
              // Three sizes, largest first: who wrote, what about, and what it says. The eye should
              // be able to skip the list on senders alone and stop only where it means to.
              //
              // The date sits above the text rather than beside it. A column of its own would take
              // its width from every line of every message, for the sake of eight characters on the
              // first one.
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="flex min-w-0 items-baseline gap-2">
                  <span
                    dir="auto"
                    // Read and unread differ by weight and by how bright they are. Weight alone is a
                    // difference a person has to look for; the pair is one they see.
                    className={`min-w-0 flex-1 truncate text-[0.8125rem] leading-tight ${
                      row.unread
                        ? "font-semibold text-foreground"
                        : "font-medium text-foreground/70"
                    }`}
                  >
                    {row.sender ?? unknownSender}
                  </span>
                  <time
                    dateTime={new Date(row.receivedAt * 1000).toISOString()}
                    className="shrink-0 text-[0.6875rem] text-muted-foreground"
                  >
                    {dates.format(new Date(row.receivedAt * 1000))}
                  </time>
                </div>
                <span className="flex items-center gap-1.5">
                  <span
                    dir="auto"
                    className={`truncate text-xs leading-tight ${
                      row.unread ? "text-foreground" : "text-muted-foreground"
                    }`}
                  >
                    {row.subject ?? noSubject}
                  </span>
                  <ConversationSize
                    of={row.inConversation}
                    open={unfolded}
                    onToggle={() => {
                      if (row.conversationId !== null) {
                        onUnfold(row.conversationId, row.messageId);
                      }
                    }}
                  />
                </span>
                {row.preview && (
                  <span
                    dir="auto"
                    className="line-clamp-3 text-[0.625rem] leading-snug text-muted-foreground"
                  >
                    {row.preview}
                  </span>
                )}

                {/* Something a person asked for could not be carried out. Shown on the row it was
                    about, with what stopped it and a way to ask again — an explanation with no way
                    out is a dead end rather than a message. */}
                {row.stopped !== null && inFolder !== null && (
                  <span className="mt-0.5 flex items-center gap-2 text-[0.625rem] leading-snug text-destructive">
                    <span className="truncate">{stoppedBecause}</span>
                    <button
                      type="button"
                      tabIndex={-1}
                      onClick={(event) => {
                        event.stopPropagation();
                        onTryAgain(row.messageId, inFolder);
                      }}
                      className="shrink-0 underline underline-offset-2 hover:no-underline"
                    >
                      {tryAgain}
                    </button>
                  </span>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );

  // A row still being fetched has nothing to act on, so it is drawn without a menu rather than
  // with one whose every entry is unavailable.
  if (row === undefined || inFolder === null) return drawn;

  return (
    <ContextMenu>
      {/* On the box the virtualizer places, and merged into it rather than wrapped around it: a
          box of its own here would be a second thing at the same coordinates, and the geometry a
          hundred thousand rows depend on is not a place to add one. */}
      <ContextMenuTrigger asChild>{drawn}</ContextMenuTrigger>
      <RowMenu
        messageId={row.messageId}
        folderId={inFolder}
        unread={row.unread}
        onToggleRead={onToggleRead}
        onArchive={onArchive}
        onThrowAway={onThrowAway}
        words={menu}
      />
    </ContextMenu>
  );
});

/**
 * Every word a row and its menu show, translated once for the whole list: the row draws, it does
 * not translate. A reason for stopping that this build has no words for reads as the one that says
 * the least.
 */
export type RowWords = {
  placeholder: string;
  markRead: string;
  markUnread: string;
  tryAgain: string;
  explanations: Record<string, string>;
  noSubject: string;
  unknownSender: string;
  menu: RowMenuWords;
};

export function rowWords(t: Translator): RowWords {
  return {
    placeholder: t("list.loadingRow"),
    markRead: t("list.markRead"),
    markUnread: t("list.markUnread"),
    tryAgain: t("list.tryAgain"),
    explanations: {
      messageIsNotThere: t("list.stopped.messageIsNotThere"),
      folderIsNotThere: t("list.stopped.folderIsNotThere"),
      nowhereToKeepDeletedMail: t("list.stopped.nowhereToKeepDeletedMail"),
      theServerRefused: t("list.stopped.theServerRefused"),
    },
    noSubject: t("list.noSubject"),
    unknownSender: t("list.unknownSender"),
    menu: {
      forMessage: t("list.menuFor"),
      markRead: t("list.markRead"),
      markUnread: t("list.markUnread"),
      archive: t("reading.archive"),
      throwAway: t("reading.delete"),
    },
  };
}

/** The menu's words, handed over whole: the row draws, it does not translate. */
export type RowMenuWords = {
  forMessage: string;
  markRead: string;
  markUnread: string;
  archive: string;
  throwAway: string;
};

/**
 * What can be done to a message, on the row that shows it.
 *
 * On the second button, because the first opens the message. Every entry does exactly what the
 * control elsewhere in the window does — the same call, through the same function — so a message
 * thrown away from here and one thrown away by the key leave the list the same way.
 *
 * Reading is not offered: that is what pressing the row does, and a menu entry for it would be a
 * second answer to a question the row has already answered.
 */
function RowMenu({
  messageId,
  folderId,
  unread,
  onToggleRead,
  onArchive,
  onThrowAway,
  words,
}: {
  messageId: string;
  folderId: FolderIdDto;
  unread: boolean;
  onToggleRead: (
    messageId: string,
    folderId: FolderIdDto,
    read: boolean,
  ) => void;
  onArchive: (messageId: string, folderId: FolderIdDto) => void;
  onThrowAway: (messageId: string, folderId: FolderIdDto) => void;
  words: RowMenuWords;
}) {
  return (
    <ContextMenuContent aria-label={words.forMessage}>
      <ContextMenuItem
        onSelect={() => onToggleRead(messageId, folderId, unread)}
      >
        {unread ? words.markRead : words.markUnread}
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => onArchive(messageId, folderId)}>
        {words.archive}
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        onSelect={() => onThrowAway(messageId, folderId)}
      >
        {words.throwAway}
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

/**
 * One message of an open conversation: indented under the row it belongs to, and shown as itself
 * rather than as a conversation — it is one message and stands for one.
 */
export function InsideRow({
  index,
  at,
  of,
  offset,
  message,
  chosen,
  density,
  dates,
  words: { noSubject, unknownSender },
  onSelect,
}: {
  /** The row it hangs under, and its place under it. */
  index: number;
  at: number;
  /** How many messages the conversation holds, which is the set a person is walking here. */
  of: number;
  offset: number;
  message: ListRowDto;
  chosen: boolean;
  density: DensityDto;
  dates: Intl.DateTimeFormat;
  words: RowWords;
  onSelect: (index: number, at: number) => void;
}) {
  return (
    <div
      // A child of a listbox has to be an option, and an option is what the selection lands on:
      // the arrow keys walk into a conversation, so this carries a name the list can announce and
      // nothing inside it takes focus of its own.
      id={insideId(index, at)}
      role="option"
      aria-posinset={at + 1}
      aria-setsize={of}
      aria-selected={chosen}
      onClick={() => onSelect(index, at)}
      className={`absolute inset-x-0 flex cursor-default items-center gap-3 border-s-2 border-border/60 ps-6 pe-3 text-start animate-in fade-in slide-in-from-top-1 duration-150 motion-reduce:animate-none ${
        chosen ? "bg-muted" : "hover:bg-muted/40"
      }`}
      style={{
        transform: `translateY(${offset}px)`,
        height: `${ROW_HEIGHT[density]}px`,
      }}
    >
      <span
        dir="auto"
        className="w-40 shrink-0 truncate text-xs text-muted-foreground"
      >
        {message.sender ?? unknownSender}
      </span>
      <span
        dir="auto"
        className={`flex-1 truncate text-xs ${message.unread ? "font-semibold" : "text-muted-foreground"}`}
      >
        {message.subject ?? noSubject}
      </span>
      <time
        dateTime={new Date(message.receivedAt * 1000).toISOString()}
        className="shrink-0 text-[0.6875rem] text-muted-foreground"
      >
        {dates.format(new Date(message.receivedAt * 1000))}
      </time>
    </div>
  );
}
