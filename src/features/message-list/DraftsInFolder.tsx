import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ComponentProps,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import { NOTHING_TO_ANSWER, type Answer } from "@/entities/message";
import { useI18n, type Translator } from "@/shared/i18n";
import { discardDraft, isCommandError, keptDrafts } from "@/shared/ipc/client";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { KeptDraftDto } from "@/shared/ipc/generated/KeptDraftDto";
import { isPlainKey } from "@/shared/ui/keys";
import { Button } from "@/shared/ui/primitives/button";

import { ListSaysNothing } from "./ListSaysNothing";
import { MessageList } from "./MessageList";
import { LIST_ID, ROW_HEIGHT } from "./rows";

/** Everything the window hands a folder's list, which is what the server's part of this is. */
type FolderList = Omit<
  ComponentProps<typeof MessageList>,
  "quietWhenEmpty" | "onAnswered"
>;

/** What is known of the drafts kept on this device. */
type Local =
  | { status: "reading" }
  | { status: "read"; drafts: KeptDraftDto[] }
  | { status: "failed"; error: CommandError };

/**
 * The drafts folder, which holds drafts from two places.
 *
 * What is being written here lives on this device, in the profile — not on the server and not in
 * the index, so no listing of mail will ever find it. What the server's drafts folder holds —
 * saved by another client, or by webmail — is mail like any other, brought in by a sync. Both are
 * shown, the ones written here first: those open in the composer, the server's open in the reader
 * and take every action any folder's mail does. The count beside the folder adds the two, so it
 * counts exactly what is here, and the window is told the same sum for the pane beside this.
 *
 * Nothing is drawn until both places have answered. Drawn as each answers, whichever came second
 * arrived above or between what was already on the screen, and the rows moved under the pointer.
 *
 * The folder says it is empty once, and only when neither place has anything. A place that could
 * not be read says so, and is never taken for a place with nothing in it.
 */
export function DraftsInFolder({
  accountId,
  list,
  onOpenDraft,
  onChanged,
}: {
  accountId: string;
  /**
   * The server's drafts folder, as the list of mail any folder is. Its `generation` is bumped by
   * the window when something was written down, which is when the drafts kept here are read too.
   */
  list: FolderList;
  /** Opens one kept here in the composer, as the draft it is rather than as a new message. */
  onOpenDraft: (answer: Answer) => void;
  /** Told when one was thrown away, so the count beside the folder is read again. */
  onChanged: () => void;
}) {
  const { t } = useI18n();
  const [local, setLocal] = useState<Local>({ status: "reading" });
  /** How many the server's folder holds, once its list knows. */
  const [onServer, setOnServer] = useState<number | null>(null);
  /** Whether the server's list has answered at all, which a refusal also is. */
  const [serverAnswered, setServerAnswered] = useState(false);
  const generation = list.generation ?? 0;
  const tellWindow = list.onHolds;
  const column = useRef<HTMLDivElement>(null);
  /** Where a draft was thrown away from, until the drafts without it have been read. */
  const refocus = useRef<number | null>(null);

  const read = useCallback(() => {
    let looking = true;

    keptDrafts(accountId)
      .then((drafts) => {
        if (looking) setLocal({ status: "read", drafts });
      })
      .catch((reason: unknown) => {
        if (!looking) return;
        setLocal({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });

    return () => {
      looking = false;
    };
  }, [accountId]);

  useEffect(read, [read, generation]);

  const here = local.status === "read" ? local.drafts.length : 0;
  const settled = local.status !== "reading" && serverAnswered;
  const localShows = here > 0 || local.status === "failed";
  // A refusal from the server's list is something it shows, and it stands under a heading of its
  // own when the drafts kept here are above it.
  const serverShows =
    (onServer ?? 0) > 0 || (serverAnswered && onServer === null);
  const both = localShows && serverShows;
  const hereHeading = useId();
  const thereHeading = useId();

  // The pane beside this says there is nothing to read only when this list has nothing, so it is
  // told what the list shows rather than what the server's half of it holds. Unknown while either
  // place is unknown or could not be read: nothing is the one thing that is not known then.
  useEffect(() => {
    tellWindow?.(
      local.status === "read" && onServer !== null ? onServer + here : null,
    );
  }, [tellWindow, local.status, onServer, here]);

  // A draft thrown away takes the focus with it when the focus was on it. It goes to the draft now
  // in that place, or to the one before when the last went; with none left here, to the server's
  // list, and with nothing anywhere, to the search. Only when the focus had nowhere left to be: a
  // click that did not take it left the person somewhere else, and that is theirs.
  useEffect(() => {
    const at = refocus.current;
    if (at === null || local.status === "reading") return;
    refocus.current = null;

    const had = document.activeElement;
    const elsewhere =
      had !== null && had !== document.body && !column.current?.contains(had);
    if (elsewhere) return;

    const rows =
      column.current?.querySelectorAll<HTMLElement>("[data-kept-draft]") ?? [];
    const next =
      rows[Math.min(at, rows.length - 1)] ??
      document.getElementById(LIST_ID) ??
      document.querySelector<HTMLElement>("[data-search]");
    next?.focus();
  }, [local]);

  return (
    <div ref={column} className="flex min-h-0 flex-1 flex-col">
      {settled && localShows && (
        // At most half the column while the server's part has something to show: a device with a
        // hundred drafts written here would otherwise push that part out of reach below them.
        <section
          aria-labelledby={both ? hereHeading : undefined}
          className={`flex min-h-0 flex-col ${both ? "max-h-[50%] shrink-0" : "flex-1"}`}
        >
          {both && <Part id={hereHeading}>{t("list.draftsHere")}</Part>}
          {local.status === "failed" ? (
            <ListSaysNothing error={local.error} t={t} />
          ) : local.status === "read" ? (
            <KeptHere
              drafts={local.drafts}
              accountId={accountId}
              density={list.density ?? "comfortable"}
              t={t}
              onOpenDraft={onOpenDraft}
              onDiscarded={(index) => {
                refocus.current = index;
                read();
                onChanged();
              }}
            />
          ) : null}
        </section>
      )}
      {/* Always there, even with nothing in it, because it is what says how much the server holds
          -- and out of sight rather than out of the page until the other place has answered. It
          says the folder is empty only once the drafts kept here are known to be none. */}
      <section
        aria-labelledby={both ? thereHeading : undefined}
        aria-hidden={settled ? undefined : true}
        className={`flex min-h-0 flex-col ${settled ? "" : "invisible"} ${
          !settled || !localShows || serverShows ? "flex-1" : ""
        }`}
      >
        {both && <Part id={thereHeading}>{t("list.draftsOnServer")}</Part>}
        <MessageList
          {...list}
          onHolds={setOnServer}
          onAnswered={setServerAnswered}
          quietWhenEmpty={local.status !== "read" || here > 0}
        />
      </section>
    </div>
  );
}

/** A quiet heading over one of the two parts, drawn only when both are there to tell apart. */
function Part({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2
      id={id}
      className="px-4 pt-3 pb-1 text-xs font-medium text-muted-foreground"
    >
      {children}
    </h2>
  );
}

/**
 * The drafts kept on this device, newest first.
 *
 * Drawn the way the list of mail under it draws a row — the same inset rule between rows and none
 * under the last, the same highlight, the same type, one line in the compact density — so the
 * seam between the two parts is a heading and not two kinds of list.
 *
 * Two stops for the keyboard however many there are: the draft being stood on and its discard.
 * The arrows, Home and End walk between drafts, as they walk the list below; a stop per draft put
 * the list of mail two presses of Tab further away for every draft kept here.
 */
function KeptHere({
  drafts,
  accountId,
  density,
  t,
  onOpenDraft,
  onDiscarded,
}: {
  drafts: KeptDraftDto[];
  accountId: string;
  density: DensityDto;
  t: Translator;
  onOpenDraft: (answer: Answer) => void;
  /** Told with the place of the one thrown away, once the core says it is gone. */
  onDiscarded: (index: number) => void;
}) {
  const [current, setCurrent] = useState(0);
  const standingOn = Math.min(current, drafts.length - 1);
  const rows = useRef<HTMLUListElement>(null);
  const compact = density === "compact";

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    if (!isPlainKey(event)) return;

    const last = drafts.length - 1;
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
      ?.querySelectorAll<HTMLElement>("[data-kept-draft]")
      [next]?.focus();
  }

  return (
    <ul
      ref={rows}
      onKeyDown={onKeyDown}
      className="no-gutter min-h-0 overflow-y-auto"
    >
      {drafts.map((draft, index) => {
        const subject = draft.subject.trim() || t("list.noSubject");
        const recipients =
          draft.recipients.trim() || t("compose.noRecipientYet");
        const stop = index === standingOn ? 0 : -1;

        return (
          <li
            key={draft.id}
            data-separated={index < drafts.length - 1 ? "true" : undefined}
            style={compact ? { height: `${ROW_HEIGHT.compact}px` } : undefined}
            className="relative px-2 py-px after:absolute after:inset-x-4 after:bottom-0 after:hidden after:h-px after:bg-border/60 after:content-[''] data-[separated]:after:block"
          >
            <div className="flex h-full items-center gap-2 rounded-md px-2 hover:bg-muted/40">
              <button
                type="button"
                data-kept-draft
                tabIndex={stop}
                onFocus={() => setCurrent(index)}
                className={`flex min-w-0 flex-1 gap-3 self-stretch rounded-md text-start outline-none focus-visible:ring-3 focus-visible:ring-ring/50 ${
                  compact ? "items-center" : "items-start pt-3 pb-[0.5625rem]"
                }`}
                onClick={() =>
                  onOpenDraft({
                    ...NOTHING_TO_ANSWER,
                    draftId: draft.id,
                    to: draft.recipients,
                    cc: draft.copies,
                    bcc: draft.blind,
                    subject: draft.subject,
                    body: draft.body,
                    answering: draft.answering,
                    chain: draft.chain,
                    fromAccount: accountId,
                  })
                }
              >
                {/* Where the list below draws whether a message was read: kept empty, so the words
                    of both parts start in one column. */}
                <span aria-hidden="true" className="size-2 shrink-0" />
                {compact ? (
                  <>
                    <span
                      dir="auto"
                      className="w-48 shrink-0 truncate text-xs text-muted-foreground"
                    >
                      {recipients}
                    </span>
                    <span dir="auto" className="flex-1 truncate text-xs">
                      {subject}
                    </span>
                  </>
                ) : (
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span
                      dir="auto"
                      className="truncate text-[0.8125rem] leading-tight font-medium text-foreground/70"
                    >
                      {subject}
                    </span>
                    <span
                      dir="auto"
                      className="truncate text-xs leading-tight text-muted-foreground"
                    >
                      {recipients}
                    </span>
                  </span>
                )}
              </button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                tabIndex={stop}
                onFocus={() => setCurrent(index)}
                aria-label={t("compose.discardThisDraft", { subject })}
                onClick={() => {
                  // Gone from the screen when the core says it is gone, not before: a row that
                  // vanishes on a write that failed is a draft a person believes they have lost.
                  discardDraft(draft.id)
                    .then(() => onDiscarded(index))
                    .catch(() => {
                      /* It stays where it is, which is the honest answer to "it did not happen". */
                    });
                }}
              >
                {t("list.discardDraft")}
              </Button>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
