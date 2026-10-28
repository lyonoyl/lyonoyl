import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { ShowPicturesDto } from "@/shared/ipc/generated/ShowPicturesDto";
import {
  Archive,
  ChevronDown,
  Download,
  FolderInput,
  Forward,
  Reply,
  ReplyAll,
  Trash2,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { folderName } from "@/entities/folder";
import {
  answerTo,
  forwardOf,
  forwardWholeOf,
  type Answer,
} from "@/entities/message";
import { describeError, listOf, useI18n } from "@/shared/i18n";
import { inRoundNumbers } from "@/shared/ui/bytes";
import {
  answerAudience,
  archiveMessage,
  deleteMessage,
  forgetAttachment,
  forwardMessage,
  getMailboxTree,
  moveMessage,
  getMessage,
  isCommandError,
  saveMessage,
} from "@/shared/ipc/client";
import { Button } from "@/shared/ui/primitives/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/primitives/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/primitives/tooltip";
import { MessageBody } from "./MessageBody";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { FolderDto } from "@/shared/ipc/generated/FolderDto";
import type { MessageDetailsDto } from "@/shared/ipc/generated/MessageDetailsDto";

/**
 * The reading pane.
 *
 * Shows what the index holds, and below it the body — which is read from the server when the
 * message is opened, sanitized in Rust, and shown in a frame of its own rather than here.
 */

/**
 * What the save button is doing.
 *
 * There is no "cancelled": closing the panel puts this back to idle and says nothing. A person who
 * decided not to save does not need to be told what they just decided.
 */
type Saving =
  | { status: "idle" }
  | { status: "asking" }
  | { status: "saved"; name: string }
  | { status: "failed"; error: CommandError };

/**
 * A filing that could not be done, which is the only thing about it this pane still says.
 *
 * What went through is said by the list: the row slides out of it and a line underneath offers the
 * way back. Saying it here as well would put the news on a message that is no longer open.
 */
type JustDone =
  { status: "nothing" } | { status: "failed"; error: CommandError };

type State =
  | { status: "empty" }
  | { status: "loading" }
  | { status: "ready"; message: MessageDetailsDto }
  | { status: "missing" }
  | { status: "failed"; error: CommandError };

export function MessageReader({
  messageId,
  folderId,
  onReply,
  askedToAnswer = 0,
  askedToAnswerAll = 0,
  onForward,
  askedToForward = 0,
  askedToThrowAway = 0,
  onFiled,
  nothingToChoose = false,
  showPictures = "whenAsked",
}: {
  messageId: string | null;
  /**
   * The folder the message is being read in, which is what anything done to it is done in.
   *
   * Given by the list rather than read off the message: a message may sit in an inbox and in a
   * folder somebody made, and taking the first placement the index lists acted on whichever of
   * them the server happened to name first.
   */
  folderId: FolderIdDto | null;
  /**
   * Whether the list beside this holds nothing at all.
   *
   * Then "select a message" is asking somebody to do something they cannot, and what is true is
   * that there is nothing here.
   */
  nothingToChoose?: boolean;
  /**
   * Whether a message fetches what it points at elsewhere without being asked.
   *
   * Handed down rather than read here: the settings screen writes it, the window holds it, and a
   * second reader of the same setting is a second answer to the same question.
   */
  showPictures?: ShowPicturesDto;
  /**
   * Told when somebody wants to answer this message, with everything an answer needs.
   *
   * Built here because this is where the message is: who wrote it, what it is called, and the
   * conversation it said it belongs to. The window opens the composer with it.
   */
  onReply?: (to: Answer) => void;
  /**
   * Bumped by the shell when somebody pressed the key for answering.
   *
   * A number rather than a call into here, because who an answer goes to is asked for and can
   * fail: the shell has no way to say why nothing opened, and this reader already does.
   */
  askedToAnswer?: number;
  /** Bumped by the shell when somebody pressed the key for answering everybody on it. */
  askedToAnswerAll?: number;
  /**
   * Told when somebody wants to pass this message on, with everything the composer opens with:
   * the message and the files it carries, which the core already holds for it.
   */
  onForward?: (forward: Answer) => void;
  /** Bumped by the shell when somebody pressed the key for passing a message on. */
  askedToForward?: number;
  /** Bumped by the shell when somebody pressed the key for throwing it away. */
  askedToThrowAway?: number;
  /**
   * Told when this message has been filed away or thrown out.
   *
   * The list is what closes up over it, and it cannot know: what happens here is written down in
   * the queue and only shows up in a list window once a server has answered.
   */
  onFiled?: (filed: {
    messageId: string;
    folderId: FolderIdDto;
    what: "archived" | "deleted" | "moved";
  }) => void;
}) {
  const { t, language } = useI18n();
  const [state, setState] = useState<State>({ status: "empty" });
  const [saving, setSaving] = useState<Saving>({ status: "idle" });
  const [justDone, setJustDone] = useState<JustDone>({ status: "nothing" });
  /** Whether a filing started here is still outstanding, which is what shuts the buttons. */
  const [filing, setFiling] = useState(false);
  /** Whether the core is still being asked who an answer goes to. */
  const [answering, setAnswering] = useState(false);
  /** Whether the core is still reading the message to pass it on. */
  const [forwarding, setForwarding] = useState(false);
  /**
   * Whether this reader is still on the screen, for a message passed on to find somebody to give
   * its files to. Gone, it lets go of them: the core holds them for a composer that will never
   * open.
   */
  const here = useRef(true);
  useEffect(() => {
    here.current = true;
    return () => {
      here.current = false;
    };
  }, []);
  /** Where this message could be filed, once somebody has asked. Null until then. */
  const [elsewhere, setElsewhere] = useState<FolderDto[] | null>(null);

  useEffect(() => {
    if (messageId === null) {
      setState({ status: "empty" });
      return;
    }

    let active = true;
    setState({ status: "loading" });
    // Forgotten with the message it belonged to: the next one may be in another account, and a
    // menu offering that account's folders would offer somewhere this message cannot go.
    setElsewhere(null);
    // What was saved was saved about the message that was open, and this is a different one.
    setSaving({ status: "idle" });
    setJustDone({ status: "nothing" });
    setFiling(false);
    setAnswering(false);
    setForwarding(false);

    getMessage(messageId)
      .then((message) => {
        if (!active) return;
        setState(
          message ? { status: "ready", message } : { status: "missing" },
        );
      })
      .catch((reason: unknown) => {
        if (!active) return;
        setState({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });

    return () => {
      active = false;
    };
  }, [messageId]);

  // The key for answering, arriving from the shell. Held in a ref and read by an effect, because
  // `answer` is declared past the early returns below, and a hook cannot be: every hook this
  // component has must run on every render, including the ones that show nothing yet.
  const answerNow = useRef<(everybody: boolean) => void>(() => {});
  const asked = useRef(askedToAnswer);
  useEffect(() => {
    if (askedToAnswer === asked.current) return;
    asked.current = askedToAnswer;
    answerNow.current(false);
  }, [askedToAnswer]);
  const askedAll = useRef(askedToAnswerAll);
  useEffect(() => {
    if (askedToAnswerAll === askedAll.current) return;
    askedAll.current = askedToAnswerAll;
    answerNow.current(true);
  }, [askedToAnswerAll]);

  // The key for passing a message on, arriving the same way and for the same reason.
  const forwardNow = useRef<() => void>(() => {});
  const askedOn = useRef(askedToForward);
  useEffect(() => {
    if (askedToForward === askedOn.current) return;
    askedOn.current = askedToForward;
    forwardNow.current();
  }, [askedToForward]);

  // The key for throwing it away, arriving the same way and for the same reason.
  const throwAwayNow = useRef<() => void>(() => {});
  const askedAway = useRef(askedToThrowAway);
  useEffect(() => {
    if (askedToThrowAway === askedAway.current) return;
    askedAway.current = askedToThrowAway;
    throwAwayNow.current();
  }, [askedToThrowAway]);

  const dates = useMemo(
    () =>
      new Intl.DateTimeFormat(language, {
        dateStyle: "full",
        timeStyle: "short",
      }),
    [language],
  );

  // Nothing to answer or pass on until the message is on screen, and then it is that message. A
  // key pressed while the next one is still on its way is not about the one before it, whose
  // answer the ref would otherwise still be holding.
  answerNow.current = () => {};
  forwardNow.current = () => {};
  throwAwayNow.current = () => {};

  // Told apart, because they mean the opposite. "Nothing selected" in front of somebody who has
  // just opened a message is the pane saying they did not do the thing they did — and it is the
  // state a slow server leaves them in for the whole of a fetch.
  if (state.status === "loading") {
    return (
      <p
        aria-busy="true"
        className="m-auto max-w-sm p-8 text-center text-sm text-muted-foreground"
      >
        {t("reading.bodyLoading")}
      </p>
    );
  }

  if (state.status === "empty") {
    return (
      <p className="m-auto max-w-sm p-8 text-center text-sm text-muted-foreground">
        {/* Asking somebody to choose from an empty list is asking them to do something they
            cannot. What is true then is that there is nothing here, which is what the list beside
            this says too -- and the two lines sit on the same height for that reason. */}
        {nothingToChoose
          ? t("reading.nothingToRead")
          : t("reading.nothingSelected")}
      </p>
    );
  }

  if (state.status === "missing") {
    return (
      <p className="m-auto max-w-sm p-8 text-center text-sm text-muted-foreground">
        {t("reading.missing")}
      </p>
    );
  }

  if (state.status === "failed") {
    return (
      <p
        role="alert"
        className="m-auto max-w-sm p-8 text-center text-sm text-muted-foreground"
      >
        {describeError(state.error, t)}
      </p>
    );
  }

  const message = state.message;

  /**
   * Asks where this message could go, when somebody opens the menu and not before.
   *
   * The folders belong to the account the message is already in, take messages at all, and are not
   * the one it is in now. The account is found by the folder because the folder is what the reader
   * knows about the message; filing across accounts is not offered, because it would be a copy and
   * a delete rather than a move and this build does neither.
   *
   * Which folders take messages is decided in Rust and arrives as a flag. An interface that read
   * it off the role would carry a second copy of a rule, and the two would part company the day a
   * role is added.
   */
  function findSomewhereToPutIt() {
    if (folderId === null || elsewhere !== null) return;

    getMailboxTree()
      .then(({ accounts }) =>
        setElsewhere(
          accounts
            .find((account) =>
              account.folders.some((folder) => folder.folderId === folderId),
            )
            ?.folders.filter(
              (folder) => folder.canReceive && folder.folderId !== folderId,
            ) ?? [],
        ),
      )
      .catch(() => setElsewhere([]));
  }

  function file(destination: FolderIdDto) {
    if (folderId === null || filing) return;
    setFiling(true);

    moveMessage(message.messageId, folderId, destination)
      .then(() =>
        onFiled?.({ messageId: message.messageId, folderId, what: "moved" }),
      )
      .catch((reason: unknown) => {
        setFiling(false);
        setJustDone({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }

  function act(what: "archived" | "deleted") {
    // The buttons keep their place while the message under them changes, and it changes within a
    // frame or two of the first click. Without this the second click of an ordinary double-click
    // files a message nobody has seen.
    if (folderId === null || filing) return;
    setFiling(true);

    const asking =
      what === "archived"
        ? archiveMessage(message.messageId, folderId)
        : deleteMessage(message.messageId, folderId);

    asking
      .then(() => onFiled?.({ messageId: message.messageId, folderId, what }))
      .catch((reason: unknown) => {
        setFiling(false);
        setJustDone({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }

  /**
   * Opens an answer to this message.
   *
   * Who it goes to is asked for rather than worked out here, so the failure has somewhere to go: a
   * person who pressed the button and got nothing would have no idea why. Nothing opens until the
   * answer is known — a composer that filled its own fields in a moment later is one somebody has
   * already started typing into.
   */
  function answer(everybody: boolean) {
    if (answering) return;
    setAnswering(true);

    answerAudience(message.messageId, everybody)
      .then((answer) => onReply?.(answerTo(message, answer, t, dates)))
      .catch((reason: unknown) => {
        setJustDone({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      })
      .finally(() => setAnswering(false));
  }

  answerNow.current = answer;

  /**
   * Opens this message to be passed on: as its text with the files it carries, or `whole`, as one
   * file that is an exact copy of it, with everything it carries and every bit of how it looks.
   *
   * Read from the message itself in the core, which holds its files before saying which they are;
   * nothing opens until it has answered, for the reason an answer waits. Answered after this
   * reader has gone, the files go with it: nothing will show them, and the core lets them go.
   */
  function forward(whole: boolean) {
    if (forwarding) return;
    setForwarding(true);

    forwardMessage(message.messageId, whole)
      .then((forward) => {
        if (!here.current || onForward === undefined) {
          for (const file of forward.files)
            void forgetAttachment(file.token).catch(() => {});
          return;
        }
        onForward(
          whole
            ? forwardWholeOf(message, forward)
            : forwardOf(message, forward, t, dates),
        );
      })
      .catch((reason: unknown) => {
        if (!here.current) return;
        setJustDone({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      })
      .finally(() => {
        if (here.current) setForwarding(false);
      });
  }

  forwardNow.current = () => forward(false);
  throwAwayNow.current = () => act("deleted");

  function save() {
    setSaving({ status: "asking" });

    saveMessage(message.messageId)
      .then((outcome) => {
        setSaving(
          outcome.name === null
            ? { status: "idle" }
            : { status: "saved", name: outcome.name },
        );
      })
      .catch((reason: unknown) => {
        setSaving({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }

  return (
    // The one place a person may want to take the text with them: the message they are reading.
    <article
      data-selectable
      className="flex min-h-0 flex-1 flex-col overflow-auto"
    >
      <header className="border-b border-border p-6">
        <div className="flex items-start justify-between gap-4">
          <h1 dir="auto" className="text-lg font-semibold tracking-tight">
            {message.subject ?? t("list.noSubject")}
          </h1>

          {/* The message as it arrived, in a file. No path is sent from here: the panel that asks
              where it goes is raised in Rust, and what comes back is a name to show.

              An icon and a hint rather than a word: the arrow into a tray is what people already
              read as "put this on my computer", and the name is still there for anyone who needs
              it — as the hint, and as the button's own name for a screen reader. */}
          <div className="flex shrink-0 items-center gap-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("reading.reply")}
                  disabled={answering}
                  onClick={() => answer(false)}
                >
                  <Reply className="rtl:-scale-x-100" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("reading.reply")}</TooltipContent>
            </Tooltip>

            {/* Only where there is a room to answer. A message addressed to one person has the
                same two answers, and offering both invites the one that says nothing extra. */}
            {message.addressedToMore && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("reading.replyAll")}
                    disabled={answering}
                    onClick={() => answer(true)}
                  >
                    <ReplyAll className="rtl:-scale-x-100" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("reading.replyAll")}</TooltipContent>
              </Tooltip>
            )}

            {/* Passing it on as text is one press; the arrow beside it says there is another
                way, and its menu holds both, so what it lists is every way there is. */}
            <div className="flex items-center">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("reading.forward")}
                    disabled={forwarding}
                    onClick={() => forward(false)}
                  >
                    <Forward className="rtl:-scale-x-100" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("reading.forward")}</TooltipContent>
              </Tooltip>

              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="w-4"
                        aria-label={t("reading.forwardWays")}
                        disabled={forwarding}
                      >
                        <ChevronDown className="size-3" />
                      </Button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent>{t("reading.forwardWays")}</TooltipContent>
                </Tooltip>

                <DropdownMenuContent align="end">
                  <DropdownMenuItem onSelect={() => forward(false)}>
                    {t("reading.forward")}
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => forward(true)}>
                    {t("reading.forwardWhole")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            {folderId !== null && (
              <>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("reading.archive")}
                      disabled={filing}
                      onClick={() => act("archived")}
                    >
                      <Archive />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>{t("reading.archive")}</TooltipContent>
                </Tooltip>

                {/* Only where there is somewhere to put it. A menu whose every entry is refused
                    is a promise the interface cannot keep. */}
                <DropdownMenu
                  onOpenChange={(open) => open && findSomewhereToPutIt()}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={t("reading.moveTo")}
                          disabled={filing}
                        >
                          <FolderInput />
                        </Button>
                      </DropdownMenuTrigger>
                    </TooltipTrigger>
                    <TooltipContent>{t("reading.moveTo")}</TooltipContent>
                  </Tooltip>

                  <DropdownMenuContent align="end">
                    {elsewhere !== null && elsewhere.length === 0 ? (
                      <DropdownMenuItem disabled>
                        {t("reading.moveNowhere")}
                      </DropdownMenuItem>
                    ) : (
                      (elsewhere ?? []).map((folder) => (
                        <DropdownMenuItem
                          key={folder.folderId}
                          onSelect={() => file(folder.folderId)}
                        >
                          <span dir="auto">{folderName(folder, t)}</span>
                        </DropdownMenuItem>
                      ))
                    )}
                  </DropdownMenuContent>
                </DropdownMenu>

                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("reading.delete")}
                      disabled={filing}
                      onClick={() => act("deleted")}
                    >
                      <Trash2 />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>{t("reading.delete")}</TooltipContent>
                </Tooltip>
              </>
            )}

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("reading.save")}
                  disabled={saving.status === "asking"}
                  onClick={save}
                >
                  <Download />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("reading.save")}</TooltipContent>
            </Tooltip>
          </div>
        </div>

        {justDone.status === "failed" && (
          <p role="alert" className="mt-2 text-sm text-muted-foreground">
            {describeError(justDone.error, t)}
          </p>
        )}

        {saving.status === "saved" && (
          <p aria-live="polite" className="mt-2 text-sm text-muted-foreground">
            {t("reading.savedAs", { name: saving.name })}
          </p>
        )}

        {saving.status === "failed" && (
          <p role="alert" className="mt-2 text-sm text-muted-foreground">
            {describeError(saving.error, t)}
          </p>
        )}

        <dl className="mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
          <dt className="text-muted-foreground">{t("reading.from")}</dt>
          <dd dir="auto">{message.sender ?? t("list.unknownSender")}</dd>

          <dt className="text-muted-foreground">{t("reading.date")}</dt>
          <dd>
            <time dateTime={new Date(message.receivedAt * 1000).toISOString()}>
              {dates.format(new Date(message.receivedAt * 1000))}
            </time>
          </dd>

          {message.size !== null && (
            <>
              <dt className="text-muted-foreground">{t("reading.size")}</dt>
              <dd>
                {inRoundNumbers(String(message.size), language, t)}
              </dd>
            </>
          )}

          <dt className="text-muted-foreground">{t("reading.folders")}</dt>
          <dd>
            {/* Joined the way the language joins a list, not with an English comma. */}
            {listOf(
              language,
              message.placements.map((placement) =>
                folderName(
                  { name: placement.folderName, role: placement.folderRole },
                  t,
                ),
              ),
            )}
          </dd>
        </dl>
      </header>

      <MessageBody
        messageId={message.messageId}
        showPictures={showPictures}
      />
    </article>
  );
}
