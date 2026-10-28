import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
  type FocusEvent,
  type KeyboardEvent,
} from "react";

import { useI18n, type Translator } from "@/shared/i18n";
import { isCommandError, sentHere } from "@/shared/ipc/client";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";
import { isPlainKey } from "@/shared/ui/keys";

import { ListSaysNothing } from "./ListSaysNothing";
import { MessageList } from "./MessageList";
import { LIST_ID, ROW_HEIGHT, rowDates } from "./rows";

/**
 * How often what was sent from here is read again while a message is still on its way, in
 * milliseconds: often enough that "Sending…" goes within a second of a server taking it.
 */
export const WHILE_ON_ITS_WAY_MS = 1000;

/**
 * How long a row read back from the server is still drawn here at most, in milliseconds, when the
 * server's part never draws its copy: a copy filed somewhere this folder does not show.
 */
export const READ_BACK_MS = 2000;

/** Everything the window hands a folder's list, which is what the server's part of this is. */
type FolderList = Omit<
  ComponentProps<typeof MessageList>,
  "quietWhenEmpty" | "onAnswered" | "chosenElsewhere"
>;

/**
 * What is known of the messages sent from this device: the outbox's last answer, and the rows
 * read back from the server since, whose copies the server's part has not drawn yet.
 *
 * One state rather than two, because each answer is compared with what is drawn when it lands,
 * and both halves of that have to be the ones that will be drawn.
 */
type Local =
  | { status: "reading" }
  | { status: "read"; sent: SentHereDto[]; readBack: SentHereDto[] }
  | { status: "failed"; error: CommandError };

/** Nothing held, one array for every time, so an unchanged hold is an unchanged state. */
const NONE_HELD: SentHereDto[] = [];

/** The same, with nothing held any more. */
function letGoOfHeld(local: Local): Local {
  return local.status === "read" && local.readBack.length > 0
    ? { ...local, readBack: NONE_HELD }
    : local;
}

/**
 * A new answer from the outbox, laid over what is drawn now.
 *
 * A message no server had taken, gone, came back to the drafts and goes at once. One that was sent
 * is held until the server's part draws its copy, unless that part has said what it holds since the
 * question was asked, which is when it holds the copy already. The same rows held already keep the
 * moment they were given: read again every second while another message waits for a connection, a
 * row given a new one each time would be held for as long as that wait.
 */
function answered(
  before: Local,
  sent: SentHereDto[],
  serverSaidSince: boolean,
): Local {
  const held = before.status === "read" ? before.readBack : NONE_HELD;
  const drawn = before.status === "read" ? [...before.sent, ...held] : [];
  const went = drawn.filter(
    (row) => !row.onItsWay && !sent.some((one) => one.id === row.id),
  );

  if (serverSaidSince || went.length === 0) {
    return { status: "read", sent, readBack: NONE_HELD };
  }
  const same =
    went.length === held.length &&
    went.every((row) => held.some((one) => one.id === row.id));
  return { status: "read", sent, readBack: same ? held : went };
}

/** Newest first, the order the outbox answers in and the list below draws by date. */
function newestFirst(rows: SentHereDto[]): SentHereDto[] {
  return [...rows].sort(
    (left, right) => right.writtenAt - left.writtenAt || right.id - left.id,
  );
}

/**
 * The folder for sent mail, which shows a message from the moment send is pressed.
 *
 * What the server's folder holds is mail like any other, brought in by a sync, and a message sent
 * from here joins it once a server has taken it, its copy is filed and the account is read back —
 * seconds, or longer with no connection. Until then it is shown from the outbox on this device,
 * above the server's part and drawn as the row its copy will be, with "Sending…" where the date
 * goes while no server has taken it. Once the copy is read, the row from here goes and the
 * server's stands in its place.
 *
 * Read again whenever the window says the mail changed, and every second while anything here is on
 * its way and somebody can see it: taking a message is not a change to the mail, and it is the
 * moment "Sending…" has to go.
 *
 * The two parts are read by two questions, and the one about this device usually answers first.
 * A row read back from the server is drawn here until the server's part has drawn its copy, and
 * goes in that frame: gone a moment before, every row below it would jump up and back down.
 *
 * Nothing is drawn until both places have answered, and the folder says it is empty once, only
 * when neither has anything; a place that could not be read says so. The drafts folder does the
 * same, for the same reasons.
 */
export function SentInFolder({
  accountId,
  list,
  opened,
  onOpen,
}: {
  accountId: string;
  /** The server's folder for sent mail, as the list of mail any folder is. */
  list: FolderList;
  /** The message from here the reader shows, if it shows one. */
  opened: number | null;
  /** Opens one sent from here in the reader, as the message it is. */
  onOpen: (sent: SentHereDto) => void;
}) {
  const { t, language } = useI18n();
  const [local, setLocal] = useState<Local>({ status: "reading" });
  /** How many the server's folder holds, once its list knows. */
  const [onServer, setOnServer] = useState<number | null>(null);
  /** Whether the server's list has answered at all, which a refusal also is. */
  const [serverAnswered, setServerAnswered] = useState(false);
  /** Bumped when a message from here is chosen, so the server's list lets go of its choice. */
  const [chosenHere, setChosenHere] = useState(0);
  /** How many times the server's part has said what it holds. */
  const heardFromServer = useRef(0);
  const generation = list.generation ?? 0;
  const tellWindow = list.onHolds;
  /** The newest question asked; an answer to any older one is not what is here now. */
  const asked = useRef(0);
  const column = useRef<HTMLDivElement>(null);
  /** Where among the rows from here the focus is, while it is on one of them. */
  const focusedAt = useRef<number | null>(null);

  const read = useCallback(() => {
    const mine = ++asked.current;
    const heardThen = heardFromServer.current;

    sentHere(accountId)
      .then((sent) => {
        if (asked.current !== mine) return;
        const serverSaidSince = heardFromServer.current !== heardThen;
        setLocal((before) => answered(before, sent, serverSaidSince));
      })
      .catch((reason: unknown) => {
        if (asked.current !== mine) return;
        setLocal({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }, [accountId]);

  useEffect(() => {
    read();
    return () => {
      asked.current += 1;
    };
  }, [read, generation]);

  const onItsWay =
    local.status === "read" && local.sent.some((sent) => sent.onItsWay);
  useEffect(() => {
    if (!onItsWay) return;

    // A window nobody can see is not asked: a message waiting all night for a connection would
    // otherwise be read every second of it.
    const ticking = window.setInterval(() => {
      if (document.visibilityState !== "hidden") read();
    }, WHILE_ON_ITS_WAY_MS);
    return () => window.clearInterval(ticking);
  }, [onItsWay, read]);

  const readBack = local.status === "read" ? local.readBack : NONE_HELD;
  useEffect(() => {
    if (readBack.length === 0) return;
    const waiting = window.setTimeout(
      () => setLocal(letGoOfHeld),
      READ_BACK_MS,
    );
    return () => window.clearTimeout(waiting);
  }, [readBack]);

  // In the order the outbox answers in, and among them, while there are any, the rows read back.
  const drawn =
    local.status !== "read"
      ? []
      : readBack.length === 0
        ? local.sent
        : newestFirst([...local.sent, ...readBack]);
  const here = drawn.length;
  const settled = local.status !== "reading" && serverAnswered;
  const localShows = here > 0 || local.status === "failed";
  const serverShows =
    (onServer ?? 0) > 0 || (serverAnswered && onServer === null);

  // The pane beside this says there is nothing to read only when this list has nothing.
  useEffect(() => {
    tellWindow?.(
      local.status === "read" && onServer !== null ? onServer + here : null,
    );
  }, [tellWindow, local.status, onServer, here]);

  // A row read back from the server goes from here, and the focus may have been on it: it goes to
  // the row now in that place, or the one before when the last went; with none left here, to the
  // server's list, and with nothing anywhere, to the search. Only when the focus had nowhere left
  // to be, which is where a browser leaves it when what held it is taken off the page.
  useLayoutEffect(() => {
    const rows = [
      ...(column.current?.querySelectorAll<HTMLElement>("[data-sent-here]") ??
        []),
    ];
    const had = document.activeElement;
    const on = rows.findIndex((row) => row === had);
    if (on !== -1) {
      focusedAt.current = on;
      return;
    }

    const at = focusedAt.current;
    if (at === null) return;
    if (had !== null && had !== document.body) return;
    focusedAt.current = null;

    const next =
      rows[Math.min(at, rows.length - 1)] ??
      document.getElementById(LIST_ID) ??
      document.querySelector<HTMLElement>("[data-search]");
    next?.focus();
  }, [local]);

  // Told before the frame with the server's rows is painted, so a row read back goes from here in
  // the frame its copy arrives in there.
  const heardServer = useCallback((holds: number | null) => {
    heardFromServer.current += 1;
    setOnServer(holds);
    setLocal(letGoOfHeld);
  }, []);

  /** The focus left a row: for good when it went somewhere, or when the row is still there. */
  const leftRow = useCallback((event: FocusEvent<HTMLButtonElement>) => {
    const row = event.currentTarget;
    if (event.relatedTarget !== null) {
      focusedAt.current = null;
      return;
    }
    // Nowhere: a click on something that takes no focus, or the row taken off the page. Only the
    // first is the person going somewhere else.
    queueMicrotask(() => {
      if (row.isConnected) focusedAt.current = null;
    });
  }, []);

  return (
    <div ref={column} className="flex min-h-0 flex-1 flex-col">
      {settled && localShows && (
        // At most half the column while the server's part has something to show, as the drafts
        // kept here are given: a hundred messages waiting for a connection would otherwise push
        // the server's part out of reach below them.
        <div
          className={`flex min-h-0 flex-col ${serverShows ? "max-h-[50%] shrink-0" : "flex-1"}`}
        >
          {local.status === "failed" ? (
            <ListSaysNothing error={local.error} t={t} />
          ) : local.status === "read" ? (
            <SentHere
              sent={drawn}
              opened={opened}
              density={list.density ?? "comfortable"}
              dates={rowDates(language)}
              t={t}
              onFocusRow={(index) => {
                focusedAt.current = index;
              }}
              onLeaveRow={leftRow}
              onOpen={(sent) => {
                setChosenHere((count) => count + 1);
                onOpen(sent);
              }}
            />
          ) : null}
        </div>
      )}
      {/* Always there, even with nothing in it, because it is what says how much the server holds
          -- and out of sight rather than out of the page until the other place has answered. */}
      <div
        aria-hidden={settled ? undefined : true}
        className={`flex min-h-0 flex-col ${settled ? "" : "invisible"} ${
          !settled || !localShows || serverShows ? "flex-1" : ""
        }`}
      >
        <MessageList
          {...list}
          onHolds={heardServer}
          onAnswered={setServerAnswered}
          quietWhenEmpty={local.status !== "read" || here > 0}
          chosenElsewhere={chosenHere}
        />
      </div>
    </div>
  );
}

/**
 * The messages sent from this device and not yet read back, newest first.
 *
 * Drawn the way the list below draws a row — the same inset rule between rows, the same
 * highlight, the same type and the same date — so that when the server's copy takes a row's place
 * nothing on the screen moves. One stop for the keyboard however many there are, walked with the
 * arrows, Home and End, as the list below is.
 */
function SentHere({
  sent,
  opened,
  density,
  dates,
  t,
  onFocusRow,
  onLeaveRow,
  onOpen,
}: {
  sent: SentHereDto[];
  opened: number | null;
  density: DensityDto;
  dates: Intl.DateTimeFormat;
  t: Translator;
  onFocusRow: (index: number) => void;
  onLeaveRow: (event: FocusEvent<HTMLButtonElement>) => void;
  onOpen: (sent: SentHereDto) => void;
}) {
  const [current, setCurrent] = useState(0);
  const standingOn = Math.min(current, sent.length - 1);
  const rows = useRef<HTMLUListElement>(null);
  const compact = density === "compact";

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    if (!isPlainKey(event)) return;

    const last = sent.length - 1;
    const next =
      event.key === "ArrowDown"
        ? Math.min(standingOn + 1, last)
        : event.key === "ArrowUp"
          ? Math.max(standingOn - 1, 0)
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    if (next === null) return;

    event.preventDefault();
    rows.current
      ?.querySelectorAll<HTMLElement>("[data-sent-here]")
      [next]?.focus();
  }

  return (
    <ul
      ref={rows}
      onKeyDown={onKeyDown}
      className="no-gutter min-h-0 overflow-y-auto"
    >
      {sent.map((one, index) => {
        const subject = one.subject.trim() || t("list.noSubject");
        const when = one.onItsWay ? (
          <span className="shrink-0 text-[0.6875rem] text-muted-foreground">
            {t("compose.sending")}
          </span>
        ) : (
          <time
            dateTime={new Date(one.writtenAt * 1000).toISOString()}
            className="shrink-0 text-[0.6875rem] text-muted-foreground"
          >
            {dates.format(new Date(one.writtenAt * 1000))}
          </time>
        );

        return (
          <li
            key={one.id}
            data-separated={index < sent.length - 1 ? "true" : undefined}
            style={{ height: `${ROW_HEIGHT[density]}px` }}
            className="relative px-2 py-px after:absolute after:inset-x-4 after:bottom-0 after:hidden after:h-px after:bg-border/60 after:content-[''] data-[separated]:after:block"
          >
            <button
              type="button"
              data-sent-here
              tabIndex={index === standingOn ? 0 : -1}
              aria-current={opened === one.id ? "true" : undefined}
              onFocus={() => {
                onFocusRow(index);
                setCurrent(index);
              }}
              onBlur={onLeaveRow}
              onClick={() => onOpen(one)}
              // On its way out, faint, as a row on its way anywhere is in the list below: true
              // about what was asked for and not yet true about where the message is.
              className={`flex h-full w-full gap-3 overflow-hidden rounded-md px-2 text-start outline-none transition-opacity duration-200 ease-out hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 motion-reduce:transition-none ${
                compact ? "items-center py-2" : "items-start pt-3 pb-[0.5625rem]"
              } ${opened === one.id ? "bg-muted hover:bg-muted" : ""} ${
                one.onItsWay ? "opacity-40" : ""
              }`}
            >
              {/* Where the list below draws whether a message was read: a message somebody sent
                  has been, so the place is kept and left empty. */}
              <span
                aria-hidden="true"
                className={`size-2 shrink-0 ${compact ? "" : "mt-1.5"}`}
              />
              {compact ? (
                <>
                  <span
                    dir="auto"
                    className="w-48 shrink-0 truncate text-xs text-muted-foreground"
                  >
                    {one.sender}
                  </span>
                  <span dir="auto" className="flex-1 truncate text-xs">
                    {subject}
                  </span>
                  {when}
                </>
              ) : (
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="flex min-w-0 items-baseline gap-2">
                    <span
                      dir="auto"
                      className="min-w-0 flex-1 truncate text-[0.8125rem] leading-tight font-medium text-foreground/70"
                    >
                      {one.sender}
                    </span>
                    {when}
                  </span>
                  <span className="flex items-center gap-1.5">
                    <span
                      dir="auto"
                      className="truncate text-xs leading-tight text-muted-foreground"
                    >
                      {subject}
                    </span>
                  </span>
                  {one.preview !== null && (
                    <span
                      dir="auto"
                      className="line-clamp-3 text-[0.625rem] leading-snug text-muted-foreground"
                    >
                      {one.preview}
                    </span>
                  )}
                </span>
              )}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
