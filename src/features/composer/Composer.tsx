import { X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { NOTHING_TO_ANSWER, type Answer } from "@/entities/message";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import {
  discardDraft,
  forgetNotedDraft,
  isCommandError,
  keepDraft,
  noteDraft,
  reopenDraft,
  sendMessage,
} from "@/shared/ipc/client";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { DraftDto } from "@/shared/ipc/generated/DraftDto";
import { attachFile, forgetAttachment } from "@/shared/ipc/client";
import type { AttachingDto } from "@/shared/ipc/generated/AttachingDto";
import { inRoundNumbers } from "@/shared/ui/bytes";
import { Button } from "@/shared/ui/primitives/button";
import { Input } from "@/shared/ui/primitives/input";
import { Label } from "@/shared/ui/primitives/label";
import { describeError, useI18n } from "@/shared/i18n";
import { typedDirection } from "@/shared/ui/direction";
import { linksIn } from "@/shared/ui/links";

import { addressesIn, everyOneAnAddress } from "./addresses";

/** Where the message is in its life, from the writer's side. */
type Writing =
  | { status: "writing" }
  | { status: "handing over" }
  | { status: "failed"; error: CommandError };

/**
 * Writing a message.
 *
 * Plain text and nothing else, deliberately: rich text is a subsystem and a hundred and thirty
 * kilobytes of it, and plain text is what every mail client on the other end reads correctly.
 *
 * Pressing send does not send. It writes the message down — in the one file on this device that is
 * nobody's copy of anything — and a round hands it over within a second or two. That is why the
 * window closes at once and with no waiting: the message exists from the moment it is written, and
 * a machine that loses power on the way has not lost it.
 */
/** How long the typing stops before what is in the window is written down, in milliseconds. */
const KEPT_AFTER_MS = 800;

export function Composer({
  accounts,
  answer = NOTHING_TO_ANSWER,
  onWritten,
  onCancel,
  onKept,
  leaveRef,
}: {
  accounts: MailAccountSummaryDto[];
  /**
   * What this is an answer to, when it is one.
   *
   * The two headers travel as the answered message wrote them and are turned into `In-Reply-To`
   * and `References` in Rust, where that rule lives. A composer opened with nothing to answer
   * carries none of them, which is a message starting its own conversation.
   */
  answer?: Answer;
  /** Told when the message is in the outbox, so the window can say so and close. */
  onWritten: () => void;
  onCancel: () => void;
  /**
   * Told when what is being written is written down as a draft of its own for the first time.
   *
   * That is when the count beside Drafts has one more to count, and it counts it from then on:
   * the draft is on this device, and would be in the folder if the window closed this second.
   * Keeping it again replaces it, and says nothing.
   */
  onKept?: () => void;
  /**
   * Where the pane's cross finds the way out of this screen.
   *
   * Leaving here is not free: the last thing typed is inside a wait that has not run yet, and a
   * cross that only closed the screen would lose it. Filled on every render, so what it holds is
   * this render's own closing rather than the first one's.
   *
   * What it may be handed is what the screen does once this window has gone, after `onCancel`:
   * another message opening in its place. It runs only if the window does go.
   */
  leaveRef?: { current: ((then?: () => void) => void) | null };
}) {
  const { t, language } = useI18n();

  // An account with no server to send from cannot carry a message, and offering it would be a
  // message written and then lost in a queue nobody watches.
  const sending = useMemo(
    () => accounts.filter((account) => account.canSend),
    [accounts],
  );

  // The account the answered message arrived in, when it can still send and is still here.
  // Otherwise the first that can: an answer from an account that has been removed is not an
  // answer, and silently offering one would be worse than starting from a sensible default.
  const [from, setFrom] = useState(
    () =>
      sending.find((account) => account.id === answer.fromAccount)?.id ??
      sending[0]?.id ??
      "",
  );
  const [to, setTo] = useState(answer.to);
  // Folded away unless there is something in them. An answer to everybody arrives with people in
  // Cc, and hiding them behind a link would mean sending to somebody the person never saw.
  const [copies, setCopies] = useState(
    answer.cc !== "" || (answer.bcc ?? "") !== "",
  );
  /** Whether the copy fields were asked for here, rather than opened with the message. */
  const askedForCopies = useRef(false);
  /**
   * The files chosen for this message, those a draft opened here carried, and those of a message
   * it passes on. Names and sizes; the bytes stayed in the core.
   */
  const [attached, setAttached] = useState<AttachingDto[]>(
    () => answer.files ?? [],
  );
  /**
   * The files this window holds, for the way out to let go of.
   *
   * Changed in the same step as the files on screen rather than once they are drawn: the window
   * can go between a file arriving and the render that would show it, and a file it never drew
   * would be held in the core for nobody.
   */
  const holding = useRef(attached);
  /** Changes the files on screen and the files this window holds, together. */
  const changeAttached = useCallback(
    (change: (so_far: AttachingDto[]) => AttachingDto[]) => {
      holding.current = change(holding.current);
      setAttached(change);
    },
    [],
  );
  /**
   * Whether the window has gone.
   *
   * Leaving is not refused while a file is being read: a read that never ends would hold the
   * window open for good. A file read in after the window went arrives to nothing that will show
   * it or let go of it, so it is let go of as it arrives. The draft does not carry it: leaving
   * writes down what was on screen.
   */
  const gone = useRef(false);
  const [attachFailed, setAttachFailed] = useState<string | null>(null);
  /**
   * The files the message being passed on carries that are too heavy to go with it, read once
   * like everything else it opened with. Each is said on a line of its own for as long as the
   * window is open: they are not among the files on screen, and nothing else says they were there.
   */
  const [leftBehind] = useState(() => answer.leftBehind ?? []);
  /**
   * Whether a file is being asked for, or read once it is chosen.
   *
   * One at a time. Each is weighed beside the files on screen when it is asked for, so a second
   * asked for while the first is still being read would be weighed without it: two files that fit
   * apart and not together would both be held, and every keep after them refused. Nothing is sent
   * meanwhile either: the message would leave without the file, and nothing would say so.
   *
   * The attach button says so and refuses the press rather than being disabled. Disabled under the
   * focus of whoever pressed it from the keyboard, it would drop the focus to the document, and the
   * next Tab after the panel would start again at the top of the window. The state alone refuses a
   * second press: what one press changes is on screen before the next one arrives.
   */
  const [choosing, setChoosing] = useState(false);
  const [cc, setCc] = useState(answer.cc);
  const [bcc, setBcc] = useState(answer.bcc ?? "");
  const [subject, setSubject] = useState(answer.subject);
  // What an answer starts with: the message being answered, quoted, under a line saying whose it
  // is. Empty for a new message and for one this device does not hold.
  const [body, setBody] = useState(answer.body);
  /**
   * The conversation this message belongs to, held here like everything else about it.
   *
   * It was read straight off the prop, which is the one part of a message that was not. A window
   * brought back from a draft is rebuilt from the draft — and the single path that brings one back
   * is the Write button, whose prop answers nothing. So a reply left unfinished came back as a new
   * message, the next keystroke wrote that over the draft, and the message went out with no
   * `In-Reply-To`: a new conversation for the person who receives it and for this list.
   */
  const [answering] = useState(answer.answering);
  const [chain] = useState(answer.chain);
  /**
   * The draft this window was opened on, if any, read once like everything else it opened with.
   *
   * A second answer can reach a window that is open: the compose button pressed over a draft with
   * no address and no subject hands one over under the same key. Read from it, the window would
   * stop waiting for the draft's files and go on keeping under the draft's number, and its next
   * keep would take every file off the draft.
   */
  const [draftId] = useState(answer.draftId);
  /**
   * The name this opening of the composer goes by in the core, the same in every keep and note it
   * sends and in the message it hands to the outbox, and in no other composer's.
   *
   * What lets the core tell this message's keeps and notes from those of a composer taken off the
   * screen a moment ago and still finishing them, so the end of the application writes each
   * message down once and lets go only of the note it was asked to.
   */
  const [writing] = useState(aNameOfItsOwn);
  const [state, setState] = useState<Writing>({ status: "writing" });
  /**
   * The number this message is being kept under, once it has been kept at all.
   *
   * Held here rather than asked for each time: the next keep replaces the last one, and a number
   * asked for again after every keystroke would leave a row behind each of them.
   */
  const kept = useRef<number | null>(null);
  /**
   * The last keep asked for, settled or not.
   *
   * Every keep waits for the one before it, and so does sending. Two keeps out at once under no
   * number wrote the message down twice; a send while a keep was out went with nothing to throw
   * away, and the keep then landed a draft of a message that had already gone.
   */
  const keeping = useRef<Promise<void>>(Promise.resolve());
  /**
   * Who is told of a first keep, as this render was given it.
   *
   * Held aside rather than read by the keep itself: a keep that depended on it would be a new keep
   * on every render of the window, and each one would start the wait before writing over again.
   */
  const toldOfKeep = useRef(onKept);
  useEffect(() => {
    toldOfKeep.current = onKept;
  });
  /**
   * Whether anything may be written down yet, or sent.
   *
   * At once for a new message. A draft opened here waits for the files it carries: a keep naming
   * none of them takes every one of them off it.
   */
  const [settled, setSettled] = useState(draftId === undefined);
  /** Whether the window is on its way out, waiting for what is in it to be written down. */
  const [closing, setClosing] = useState(false);
  /** Whether the question about keeping what was written is on screen. */
  const [asking, setAsking] = useState(false);
  /**
   * What leaving goes on to once the window has gone, as the way out was handed it: another
   * message opening in this one's place. Nothing for a way out that only leaves.
   *
   * Set by every way out taken, to what it was handed or to nothing, and only a way out leads to
   * leaving: so after the question is put away, or a keep could not be made, the cross pressed
   * later does not open a message somebody stopped asking for.
   */
  const goingOnTo = useRef<(() => void) | null>(null);
  /** The question, so the focus goes into it: the form it replaced is gone from under the cursor. */
  const question = useRef<HTMLDivElement>(null);

  const busy = state.status === "handing over";
  /**
   * Whether the form takes no changes: while the message is going out, while the window is on its
   * way out, and until a draft opened here has its files back, which is for as long as the window
   * is open when they cannot be had. Nothing typed then could be written down — what goes, or is
   * kept or thrown away on the way out, is what was on screen when it was asked for — and the
   * window would close over it without a word.
   */
  const frozen = busy || !settled || closing;

  // A draft opened on purpose is filled in from what was kept; a message being started is not.
  //
  // It used to bring back the newest draft whenever the composer opened blank, which meant every
  // press of Compose started inside the last thing anybody had saved -- an address and a subject
  // from another day, in a window a person opened to write something else. What is saved lives in
  // the drafts folder now, and is opened from there.
  useEffect(() => {
    if (draftId === undefined) return;

    kept.current = draftId;

    // The files it carries come back as files attached a moment ago: listed, taken off and sent
    // the same way, under tokens of their own.
    let opening = true;
    reopenDraft(draftId)
      .then((files) => {
        if (!opening) {
          // Asked for by a window that has gone: nothing will ever show these, and what the core
          // holds for them is let go of rather than kept until the application quits.
          for (const file of files)
            void forgetAttachment(file.token).catch(() => {});
          return;
        }
        changeAttached((so_far) => [...files, ...so_far]);
        setSettled(true);
      })
      .catch((reason: unknown) => {
        if (!opening) return;
        // Not settled, so nothing is written over the draft, nothing is sent and nothing can be
        // typed: it stays on the disk as it was, files and all, and the window says why it cannot
        // go on.
        setState({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });

    return () => {
      opening = false;
    };
  }, [draftId, changeAttached]);

  /** Whether nothing at all is on screen, which is written down only over a draft kept already. */
  const nothing =
    to === "" &&
    cc === "" &&
    bcc === "" &&
    subject === "" &&
    body === "" &&
    attached.length === 0;
  /** What is on screen, as a keep and a note both write it down. */
  const onScreen = useMemo<DraftDto>(
    () => ({
      recipients: to,
      copies: cc,
      blind: bcc,
      subject,
      body,
      answering,
      chain,
      // Every file on screen, every time: the draft carries what is named here and nothing else,
      // so a file taken off leaves it at this keep. The core writes a file into it once and knows
      // it by its token after that, so naming a photograph again costs nothing.
      files: attached.map((file) => file.token),
      writing,
    }),
    [to, cc, bcc, subject, body, answering, chain, attached, writing],
  );

  /**
   * Writes down what is in the window, and answers when it is written.
   *
   * The same call the timer makes and the one closing waits for, so what is kept on the way out is
   * what was on screen and not what it was a second earlier.
   */
  const write = useCallback(() => {
    // The number is read when this keep's turn comes rather than when it was asked for: the keep
    // before it may be the one that brought it.
    const next = keeping.current
      .catch(() => {
        /* The keep before failed; this one is asked for anyway, and says for itself. */
      })
      .then(() => {
        if (!settled || from === "" || (nothing && kept.current === null))
          return;

        const under = kept.current;
        return keepDraft(from, under, onScreen).then((id) => {
          kept.current = id;
          if (id !== under) toldOfKeep.current?.();
        });
      });
    keeping.current = next;
    return next;
  }, [settled, from, nothing, onScreen]);

  // Written down while it is being typed, a moment after the typing stops. Every keystroke would
  // be a write to the disk this application waits for; only at the end would be a message lost by
  // the one thing that ends a window without warning.
  //
  // Not once the window is on its way out, where leaving writes down what is on screen itself. A
  // keep landing while the draft is being thrown away finds no number to keep under, and writes
  // the message down again as a draft of its own, every file with it.
  useEffect(() => {
    if (!settled || busy || closing) return;

    const later = window.setTimeout(() => {
      write().catch(() => {
        /* Said nowhere: a window that interrupts to report a failed save is a window that
           interrupts while somebody is writing. The next keystroke tries again, and closing the
           window does say so. */
      });
    }, KEPT_AFTER_MS);

    return () => window.clearTimeout(later);
  }, [settled, busy, closing, write]);

  /**
   * Whether the core holds a note of this message, so it is let go of once rather than at every
   * render that has nothing to note.
   */
  const noted = useRef(false);

  /** Lets go of what this composer noted, if it noted anything. */
  function letGoOfTheNote() {
    if (!noted.current) return;
    noted.current = false;
    void forgetNotedDraft(writing).catch(() => {});
  }

  // Noted to the core as it changes, for the end of the application to write down. Quitting takes
  // none of the ways out below: Cmd+Q, the Dock and logging out end the application from the
  // system's side, the keep the timer above is waiting to make never runs, and once the end has
  // begun this window cannot be asked anything. So the core holds the newest of what is on screen
  // all along, and writes it down as a keep when the application ends, wherever the system says
  // it is ending: logging out of a Linux desktop does not.
  //
  // Sent as each change is drawn rather than once the typing stops, for the same reason: the key
  // that quits comes after the last change, and a note that waited for the typing to stop could
  // still be waiting when it came. What a note costs grows with the text: turned into the request by the
  // engine this window runs on and read out of it by the core, about three milliseconds a change
  // at a megabyte of text, and about twenty-five at the sixteen megabytes a message may hold.
  //
  // Not while the message is going out or the window is on its way out: the core lets go of the
  // note in the same step as the outbox takes the message, and each way out lets go of it as it
  // leaves. Not before a draft opened here has its files back, and not for a message the keep
  // would not write down.
  useEffect(() => {
    if (!settled || busy || closing) return;

    if (from === "" || (nothing && kept.current === null)) {
      letGoOfTheNote();
      return;
    }

    noted.current = true;
    void noteDraft(from, kept.current, onScreen).catch(() => {
      /* Said nowhere, like a keep the timer makes: the next change notes it again. */
    });
  }, [settled, busy, closing, from, nothing, onScreen]);

  // Gone from the screen, the window lets go of the files it was holding in the core — after the
  // last keep has written them into the draft, and to no effect for any that sending took. Held
  // there, a draft opened and closed ten times would be its photographs held ten times over until
  // the application quits.
  useEffect(() => {
    // Here again on every mount: in development React mounts a component, takes it away and mounts
    // it again, and a window taken for gone would let go of every file as it arrived.
    gone.current = false;
    return () => {
      gone.current = true;
      // Off the screen, the message is not being written here, and what it leaves behind is what
      // its last keep wrote.
      letGoOfTheNote();
      const tokens = holding.current.map((file) => file.token);
      void keeping.current
        .catch(() => {
          /* A keep that failed kept nothing; the files are let go of all the same. */
        })
        .then(() => {
          for (const token of tokens)
            void forgetAttachment(token).catch(() => {});
        });
    };
  }, []);

  /**
   * Closes the window, and not before what is in it has been written down.
   *
   * The last thing typed is inside the wait the timer above is serving out, so closing without
   * this loses whatever was written in the second before it. And a keep that failed is said out
   * loud rather than closed over: losing the text silently is the one thing this is for.
   */
  /**
   * Whether anything has been written that would be lost by leaving.
   *
   * A draft still waiting for its files counts whatever is on screen: they are not on screen, and
   * leaving it unasked would throw it away with every one of them.
   */
  const written =
    !settled ||
    to.trim() !== "" ||
    cc.trim() !== "" ||
    bcc.trim() !== "" ||
    subject.trim() !== "" ||
    body.trim() !== "" ||
    attached.length > 0;

  /**
   * Leaving.
   *
   * Nothing written goes at once: there is nothing to decide about. Anything else is a question,
   * because both answers are ones people mean -- a message abandoned halfway is sometimes a
   * message to come back to and sometimes one to be rid of, and the window cannot tell which. It
   * used to keep every one of them, so Drafts filled with messages nobody meant to write.
   *
   * `then` is what the screen opens in this window's place, once it has gone.
   */
  function close(then?: () => void) {
    // Not while the message is going out. The refusal is here rather than on a disabled button
    // because the way out is now the pane's cross, which knows nothing about what is in flight.
    if (closing || busy) return;

    // The question is already up: this answers it rather than asking it again. Whatever holds the
    // focus, the one route out of this screen behaves the same — which is the point of there
    // being one route.
    if (asking) {
      setAsking(false);
      return;
    }
    goingOnTo.current = then ?? null;
    if (!written) {
      leave(false);
      return;
    }

    setAsking(true);
  }

  // The form is gone from under the pointer, so the focus has to be put somewhere. Left alone it
  // fell to the document: the question was announced by nothing and Tab started again at the top
  // of the window.
  useEffect(() => {
    if (asking) question.current?.focus();
  }, [asking]);

  /** Goes, keeping what was written or throwing it away. */
  function leave(keep: boolean) {
    if (closing) return;
    setAsking(false);
    setClosing(true);
    // Thrown away, it stops being written now: noted until the draft is gone, it would be written
    // down again if the application ended in between.
    if (!keep) letGoOfTheNote();

    // Written down before leaving, because the last thing typed is inside a wait that has not run
    // yet. Thrown away afterwards rather than instead: the timer may already have kept one while
    // somebody was typing, and that copy is the one to remove. A message being thrown away goes
    // even when it could not be written down — holding more text than a message may carry, say —
    // because what was not kept is what was asked to go.
    write()
      .catch((reason: unknown) => {
        if (keep) throw reason;
      })
      .then(() => {
        // Kept, and nothing has been taken since: the note held the same until this keep landed.
        if (keep) {
          letGoOfTheNote();
          return;
        }
        const kept_id = kept.current;
        kept.current = null;
        return kept_id === null
          ? undefined
          : discardDraft(kept_id).catch((reason: unknown) => {
              // Still there, so still the draft this window writes into: the next keep would
              // otherwise write the message down a second time beside it.
              kept.current ??= kept_id;
              throw reason;
            });
      })
      .then(() => {
        onCancel();
        goingOnTo.current?.();
      })
      .catch((reason: unknown) => {
        setClosing(false);
        setState({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }

  // No dependency list: `close` reads this render's state, and a leaving handed over once would
  // write down what was in the fields when the screen opened.
  //
  // Before the paint rather than after it, and that is not a detail. The cross belongs to the pane
  // and can be pressed the instant this screen is on screen; published from an ordinary effect,
  // the handler is republished only after the browser has painted, so there is a window in which
  // what is drawn is this render and what the cross calls is the one before it. Here that window
  // is a draft written down without the last thing typed in it.
  useLayoutEffect(() => {
    if (!leaveRef) return;
    leaveRef.current = close;
    return () => {
      leaveRef.current = null;
    };
  });

  // Somewhere to send it, and somewhere that could be an address. Any non-empty text used to do:
  // a half-typed name went to the server, which refused it after the wait, and what came back was
  // a failure about a message that could never have been sent.
  const addressed = [to, cc, bcc].some(everyOneAnAddress);

  /** One address per comma, with the spaces people paste taken off. */

  function send() {
    if (!addressed || from === "" || !settled || choosing) return;
    setState({ status: "handing over" });

    const chosen = sending.find((account) => account.id === from);
    /** The draft this message was kept as, once any keep still on its way has landed. */
    let wasKept: number | null = null;

    keeping.current
      .catch(() => {
        /* A keep that failed kept nothing, and the message is sent all the same. */
      })
      .then(() => {
        // Sent means it is no longer being written. Let go of before the server is asked, because
        // a message that has gone and a message being written are two things and only one of them
        // is true from here on.
        wasKept = kept.current;
        kept.current = null;

        // The note stays until the outbox has the message, and the core lets go of it in the same
        // step as it takes it, by the name this composer goes by. Let go of here, an application
        // ending while the message was handed over would find nothing to write down, and the
        // message would be in neither place. The draft it was kept as goes in that step too:
        // thrown away from here once the message had gone, it would outlive an application ending
        // in between, an older copy of the message for somebody to send again.
        return sendMessage(from, {
          from: chosen?.email ?? "",
          to: addressesIn(to),
          cc: addressesIn(cc),
          bcc: addressesIn(bcc),
          subject: subject.trim(),
          body,
          answering,
          chain,
          files: attached.map((file) => file.token),
          writing,
          kept: wasKept,
        });
      })
      // Told once the outbox has the message, which throws away the draft it was kept as in the
      // same step, because being told is what makes the window read the sidebar again: told
      // first, it would count beside Drafts a draft on its way out.
      .then(() => {
        // Let go of in the core as the outbox took it.
        noted.current = false;
        onWritten();
      })
      .catch((reason: unknown) => {
        // Not sent, so still being written, and still as the draft it was: let go of for good,
        // the next keep would write it down a second time beside the first.
        kept.current ??= wasKept;
        setState({
          status: "failed",
          error: isCommandError(reason)
            ? reason
            : { code: "internal", message: "", path: null },
        });
      });
  }

  if (asking) {
    return (
      // Instead of the form rather than over it: what is being decided is what happens to
      // everything in the form, and a question a person has to read past the thing it is about is
      // a question answered by whichever button is nearer.
      <div
        ref={question}
        tabIndex={-1}
        role="alertdialog"
        aria-labelledby="keep-draft-asks"
        className="m-auto flex max-w-md flex-col items-start gap-3 p-8 outline-none"
      >
        <h1
          id="keep-draft-asks"
          className="text-lg font-semibold tracking-tight"
        >
          {t("compose.keepDraft")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("compose.keepDraftBody")}
        </p>
        <div className="mt-2 flex items-center gap-2">
          <Button type="button" onClick={() => leave(true)}>
            {t("compose.keepIt")}
          </Button>
          <Button
            type="button"
            variant="destructive"
            onClick={() => leave(false)}
          >
            {t("compose.discardIt")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => setAsking(false)}
          >
            {t("compose.cancel")}
          </Button>
        </div>
      </div>
    );
  }

  if (sending.length === 0) {
    return (
      <div className="m-auto flex max-w-md flex-col items-start gap-3 p-8">
        <h1 className="text-lg font-semibold tracking-tight">
          {t("compose.nowhereToSend")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("compose.nowhereToSendBody")}
        </p>
        <Button variant="outline" onClick={onCancel}>
          {t("compose.cancel")}
        </Button>
      </div>
    );
  }

  return (
    <form
      className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col gap-4 p-6"
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
    >
      <h1 className="text-lg font-semibold tracking-tight">
        {t("compose.title")}
      </h1>

      {sending.length > 1 && (
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="compose-from">{t("compose.from")}</Label>
          <select
            id="compose-from"
            disabled={frozen}
            value={from}
            onChange={(event) => setFrom(event.target.value)}
            className="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
          >
            {sending.map((account) => (
              <option key={account.id} value={account.id} dir="auto">
                {account.displayName.trim() || account.email}
              </option>
            ))}
          </select>
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="compose-to">{t("compose.to")}</Label>
        <Input
          id="compose-to"
          dir="ltr"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          disabled={frozen}
          value={to}
          onChange={(event) => setTo(event.target.value)}
        />
      </div>

      {copies ? (
        <>
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor="compose-cc">{t("compose.cc")}</Label>
              {/* A way back out. Two fields that appear on a press and cannot be put away again
                  are two empty fields in every message somebody wrote after pressing it once. */}
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                disabled={frozen}
                aria-label={t("compose.dropCopies")}
                title={t("compose.dropCopies")}
                onClick={() => {
                  // What was typed in them goes with them: fields nobody can see must not decide
                  // who receives a message.
                  setCc("");
                  setBcc("");
                  setCopies(false);
                  askedForCopies.current = false;
                }}
              >
                <X />
              </Button>
            </div>
            <Input
              id="compose-cc"
              dir="ltr"
              autoFocus={askedForCopies.current}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              disabled={frozen}
              value={cc}
              onChange={(event) => setCc(event.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="compose-bcc">{t("compose.bcc")}</Label>
            <Input
              id="compose-bcc"
              dir="ltr"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              disabled={frozen}
              value={bcc}
              onChange={(event) => setBcc(event.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              {t("compose.bccHint")}
            </p>
          </div>
        </>
      ) : (
        <button
          type="button"
          disabled={frozen}
          // The button removes itself, so without this the focus falls to the document and the
          // two fields it asked for appear in silence. Focus goes into the first of them, which
          // is the thing that was asked for.
          onClick={() => {
            askedForCopies.current = true;
            setCopies(true);
          }}
          className="self-start text-sm text-muted-foreground underline underline-offset-2 hover:no-underline"
        >
          {t("compose.addCopies")}
        </button>
      )}

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="compose-subject">{t("compose.subject")}</Label>
        <Input
          id="compose-subject"
          // A subject answered in another language keeps its own order: `Re: Lunch tomorrow?`
          // laid out right to left puts the question mark in front.
          dir={typedDirection(subject)}
          disabled={frozen}
          value={subject}
          onChange={(event) => setSubject(event.target.value)}
        />
      </div>

      {/* Below the subject and above the text, which is where a person looks for it — and where
          what is attached stays visible while they write rather than being remembered. */}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={frozen}
          // Dimmed and out of the pointer's way like a disabled button, while keeping the focus:
          // see `choosing`.
          aria-disabled={choosing || undefined}
          className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
          onClick={() => {
            if (choosing) return;
            setAttachFailed(null);
            setChoosing(true);
            // Every file on screen goes with the question, so a file too heavy to go beside them
            // is refused as it is chosen, while somebody can choose another, rather than at every
            // keep that follows.
            void attachFile(attached.map((file) => file.token))
              .then((chosen) => {
                // `null` is the person closing the dialog, which is an answer and not a failure.
                if (!chosen) return;
                if (gone.current) {
                  void forgetAttachment(chosen.token).catch(() => {});
                  return;
                }
                changeAttached((so_far) => [...so_far, chosen]);
              })
              .catch((reason: unknown) =>
                setAttachFailed(
                  isCommandError(reason)
                    ? describeError(reason, t)
                    : t("error.bridgeUnavailable"),
                ),
              )
              .finally(() => setChoosing(false));
          }}
        >
          {t("compose.attach")}
        </Button>

        {attached.map((file) => (
          <Button
            key={file.token}
            type="button"
            variant="ghost"
            size="sm"
            disabled={frozen}
            aria-label={t("compose.removeAttachment", { name: file.name })}
            onClick={() => {
              // Forgotten in the core as well as here: the bytes are held there, and a file
              // dropped from this list and kept there is memory nothing will ever release.
              void forgetAttachment(file.token).catch(() => {});
              changeAttached((so_far) =>
                so_far.filter((held) => held.token !== file.token),
              );
            }}
          >
            <span dir="auto" className="max-w-48 truncate">
              {file.name}
            </span>
            <span className="ms-2 shrink-0 text-xs tabular-nums text-muted-foreground">
              {inRoundNumbers(file.bytes, language, t)}
            </span>
          </Button>
        ))}

        {attachFailed !== null && (
          <span role="alert" className="text-sm text-destructive">
            {attachFailed}
          </span>
        )}
      </div>

      {leftBehind.length > 0 && (
        <div role="alert" className="flex flex-col gap-1 text-sm text-destructive">
          {leftBehind.map((name, at) => (
            <p key={at}>{t("compose.leftBehind", { name })}</p>
          ))}
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col gap-1.5">
        <Label htmlFor="compose-body">{t("compose.body")}</Label>
        <MarkedText
          id="compose-body"
          // Every line in its own direction: an answer opens with a line in the window's language
          // and quotes lines in the language they were written in.
          dir={typedDirection(body)}
          disabled={frozen}
          value={body}
          onChange={setBody}
        />
      </div>

      {state.status === "failed" && (
        <p role="alert" className="text-sm text-destructive">
          {describeError(state.error, t)}
        </p>
      )}

      <div className="flex items-center gap-2">
        <Button type="submit" disabled={frozen || !addressed || choosing}>
          {busy ? t("compose.sending") : t("compose.send")}
        </Button>
      </div>
    </form>
  );
}

/**
 * The text of the message, with every address in it underlined as it is typed: it is a link when
 * the message is read here, and in most of the places it goes.
 *
 * A text field draws its text in one colour and nothing else, so the lines are drawn under it: a
 * copy of the text, laid out as the field lays it out and invisible but for the lines under its
 * addresses, behind a field with no background of its own. The copy wraps at the width the
 * field's text wraps at, which is narrower than the field by a scroll bar where a system draws
 * one, and scrolls with it. It is nobody's to read: the field is what a reader is told about.
 */
function MarkedText({
  id,
  dir,
  disabled,
  value,
  onChange,
}: {
  id: string;
  dir: "auto" | undefined;
  disabled: boolean;
  value: string;
  onChange: (value: string) => void;
}) {
  const field = useRef<HTMLTextAreaElement>(null);
  const copy = useRef<HTMLDivElement>(null);
  const links = useMemo(() => linksIn(value), [value]);

  const follow = useCallback(() => {
    const typed = field.current;
    const under = copy.current;
    if (typed === null || under === null) return;

    const style = window.getComputedStyle(typed);
    const borders =
      parseFloat(style.borderLeftWidth || "0") +
      parseFloat(style.borderRightWidth || "0");
    under.style.width = `${typed.clientWidth + borders}px`;
    under.scrollTop = typed.scrollTop;
    under.scrollLeft = typed.scrollLeft;
  }, []);

  // After every change of the text, before it is painted, so a line never stands where a word was.
  useLayoutEffect(follow, [follow, value]);
  useEffect(() => {
    const typed = field.current;
    if (typed === null || typeof ResizeObserver === "undefined") return;

    const watching = new ResizeObserver(follow);
    watching.observe(typed);
    return () => watching.disconnect();
  }, [follow]);

  let at = 0;
  const pieces: ReactNode[] = [];
  for (const { start, end } of links) {
    pieces.push(value.slice(at, start));
    pieces.push(
      <span
        key={start}
        data-link=""
        className="underline decoration-primary decoration-1 underline-offset-2"
      >
        {value.slice(start, end)}
      </span>,
    );
    at = end;
  }
  pieces.push(value.slice(at));

  return (
    <div className="relative min-h-48 flex-1">
      <div
        ref={copy}
        aria-hidden="true"
        dir={dir}
        className="pointer-events-none absolute start-0 top-0 h-full overflow-hidden rounded-md border border-transparent p-3 text-sm break-words whitespace-pre-wrap text-transparent"
      >
        {pieces}
        {/* A field opens a new line after a last line break; a copy without something after
            the break would not, and its lines would stand one short at the end. */}
        {"\u200B"}
      </div>
      <textarea
        ref={field}
        id={id}
        dir={dir}
        disabled={disabled}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onScroll={follow}
        // No frame round the whole of the message while it is written: the caret says where the
        // typing goes.
        className="absolute inset-0 h-full w-full resize-none rounded-md border border-input bg-transparent p-3 text-sm outline-none"
      />
    </div>
  );
}

/**
 * A name for one opening of the composer, made at random.
 *
 * Random rather than counted: the window can be loaded again while the core runs on, and a count
 * starting again at one would be a name the core may still hold for another message.
 */
function aNameOfItsOwn(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
