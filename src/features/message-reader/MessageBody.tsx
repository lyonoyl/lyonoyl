import { useEffect, useRef, useState } from "react";

import { describeError, useI18n } from "@/shared/i18n";
import {
  closeMessageBody,
  isCommandError,
  onMessageLink,
  onPictureMissing,
  saveAttachment,
  openMessageBody,
  whatTheWindowWears,
} from "@/shared/ipc/client";
import { Button } from "@/shared/ui/primitives/button";
import { inRoundNumbers } from "@/shared/ui/bytes";
import type { BodyFormDto } from "@/shared/ipc/generated/BodyFormDto";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { MessageBodyDto } from "@/shared/ipc/generated/MessageBodyDto";
import type { ShowPicturesDto } from "@/shared/ipc/generated/ShowPicturesDto";
import { LinkConfirmation, type Asked } from "./LinkConfirmation";

type State =
  | { status: "loading" }
  | {
      status: "ready";
      body: MessageBodyDto;
      /** Whether the pictures were asked for in the render that produced this body. */
      asked: boolean;
    }
  | { status: "failed"; error: CommandError };

/**
 * The body of a message, shown without this window ever holding it.
 *
 * The frame below is the third of the four layers between a message and the reader: the body is
 * sanitized in Rust, served on a scheme of its own under a policy of its own, and embedded here in
 * a sandbox with nothing allowed in it. No `allow-*`, ever — one of them turns a frame holding a
 * hostile message into a place that can run it, and the window it sits in is the one that holds
 * IPC.
 */
/**
 * The extension a file's name ends in, without its dot, or nothing where the name has none.
 *
 * What the system reads to decide what a saved file is and what opens it. A dot at the very start
 * is not one — it is what hides a file — and the window does not rely on the names reaching it
 * already without such a dot.
 */
function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");

  return dot > 0 ? name.slice(dot + 1) : "";
}

/**
 * The word for a kind of file: the extension its name ends in, in capitals.
 *
 * Taken from the name and never from the type the message declared for the part. The extension is
 * what the system goes by once the file is saved; the declared type is only the sender's word for
 * it, and a message can call an application a PDF — a row reading PDF beside it would be this
 * window repeating the claim. Not a catalogue key and not translated: an extension is part of the
 * name the message gave the file.
 *
 * Shown only when it is short and plain. An extension is whatever follows the last dot, and
 * `final draft` is not a word for a kind of anything. Where there is no word, the row is a name and
 * a size.
 */
function kindOf(name: string): string {
  const extension = extensionOf(name);

  return /^[a-z0-9]{1,8}$/i.test(extension) ? extension.toUpperCase() : "";
}

/** The most of a name's end that is kept whole: any extension a system reads, and not much more. */
const LONGEST_ENDING = 20;

/**
 * A file's name as two parts: the start, which may be cut short, and the ending, which never is.
 *
 * The ending is the extension, dot and all. A name cut at its end loses exactly that, and a sender
 * who knows it writes `Statement-for-your-account-ending-0000.pdf.app`: cut there, it reads as a
 * document and opens as an application. So the cut goes in the middle, and the ending stays on
 * screen. Past the length of any extension a system reads, only the last characters are kept
 * whole, so a name whose last dot is followed by a sentence rather than an extension still shows
 * how it starts.
 */
function startAndEnding(name: string): [string, string] {
  const extension = extensionOf(name);
  if (extension === "") return [name, ""];

  const characters = Array.from(name);
  const kept = Math.min(Array.from(extension).length + 1, LONGEST_ENDING);

  return [
    characters.slice(0, characters.length - kept).join(""),
    characters.slice(characters.length - kept).join(""),
  ];
}

/** The three marks the stylesheet keys its colours off, as one value to compare. */
function theLookOfTheWindow(): string {
  if (typeof document === "undefined") return "";

  const root = document.documentElement;

  return [
    root.classList.contains("dark"),
    root.getAttribute("data-scheme") ?? "",
    root.getAttribute("data-light-scheme") ?? "",
  ].join(":");
}

export function MessageBody({
  messageId,
  showPictures = "whenAsked",
}: {
  messageId: string;
  /** What the reader has said, once, about pictures that live on somebody else's machine. */
  showPictures?: ShowPicturesDto;
}) {
  const { t } = useI18n();
  const [state, setState] = useState<State>({ status: "loading" });
  const [asked, setAsked] = useState<Asked | null>(null);
  /** Which way the reader is looking at this message. */
  const [form, setForm] = useState<BodyFormDto>("sent");
  /** Whether the pictures this message points at are being fetched.
   *
   *  False for every message, including under a standing "show them": what that setting does is
   *  make the press below rather than skip it, and it is made once the text is on screen. Per
   *  message either way, because fetching a picture tells its host that this message was opened. */
  const [pictures, setPictures] = useState(false);
  /** Bumped to ask again for pictures that did not arrive the first time. */
  const [attempt, setAttempt] = useState(0);
  /**
   * Which look the window is in, as something that changes when the window changes.
   *
   * A body is drawn in the colours of the moment it is asked for, and its name serves only that
   * drawing, so a message already open cannot be repainted — it has to be asked for again. What is watched is the three marks the
   * stylesheet keys off, rather than the colours themselves: those are read at the moment the body
   * is asked for, and reading them on every mutation would be work for nothing. Watching the
   * document rather than the stored setting catches every reason they change — a scheme chosen in
   * settings, and the system turning dark under a window told to follow it.
   */
  const [wearing, setWearing] = useState(theLookOfTheWindow);

  useEffect(() => {
    if (typeof MutationObserver === "undefined") return;

    const watching = new MutationObserver(() =>
      setWearing(theLookOfTheWindow()),
    );

    watching.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-scheme", "data-light-scheme"],
    });

    return () => watching.disconnect();
  }, []);

  /** The message and the setting this screen was last put in order for. */
  const [putInOrderFor, setPutInOrderFor] = useState({
    messageId,
    showPictures,
  });

  // A different message is looked at as it was sent, and with nothing fetched yet whatever the last
  // one needed. The setting is watched here too, so an answer given while a message is open is
  // about that message rather than about the next one — in both directions.
  //
  // Done while rendering rather than in an effect, which is React's own answer for state that
  // follows a prop. An effect runs after the browser has drawn, and in that gap the screen offers
  // a button belonging to the message before this one: a press landing there is answered by a
  // reset that undoes it.
  if (
    putInOrderFor.messageId !== messageId ||
    putInOrderFor.showPictures !== showPictures
  ) {
    setPutInOrderFor({ messageId, showPictures });
    setForm("sent");
    setPictures(false);
    setAttempt(0);
  }

  // A link in the message was followed. The frame could not go anywhere — it has no permission to —
  // so what it did instead was ask, and this is where the person answers.
  useEffect(() => {
    let active = true;
    let stop: (() => void) | null = null;

    onMessageLink((link) => active && setAsked({ link, failed: false }))
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        /* Without the listener a link does nothing, which is the safe direction. */
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  /** Which message the body on screen belongs to, so a redraw is told from an opening. */
  const shownFor = useRef<string | null>(null);
  /** The name the frame is fetching from, so the one it replaces is the one let go. */
  const showing = useRef<string | null>(null);

  /** Whether the frame came for a picture of this body and it was not there. */
  const [wentMissing, setWentMissing] = useState(false);

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;

    onPictureMissing(() => {
      if (active) setWentMissing(true);
    })
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {
        /* Without the listener the message renders with holes and says nothing, which is what this
           is here to stop, but it is not worth refusing to show the message over. */
      });

    return () => {
      active = false;
      stop?.();
    };
  }, []);

  // Let go of the last one when the reader leaves, and of nothing else here: what is on screen is
  // let go by what replaces it.
  useEffect(
    () => () => {
      if (showing.current === null) return;

      void closeMessageBody(showing.current).catch(() => {
        /* Nothing depends on it having happened; the store holds a few and drops the quietest. */
      });
    },
    [],
  );

  useEffect(() => {
    let active = true;

    // Blank only when there is nothing to look at. Asking for the same message again — as plain
    // text, with its pictures, under different colours — is a redraw of something already on
    // screen, and taking it away and putting it back is the message flickering out of existence
    // for as long as the round trip takes. What is shown stays until what replaces it is ready.
    setState((shown) =>
      shown.status === "ready" && shownFor.current === messageId
        ? shown
        : { status: "loading" },
    );

    // Read at the moment the body is asked for, so a message opened after the colours changed is
    // served on the ones that are on screen now.
    void openMessageBody(messageId, form, pictures, whatTheWindowWears())
      .then((body) => {
        // Nothing points at this one: it was answered after the reader had moved on.
        if (!active) {
          void closeMessageBody(body.name).catch(() => {});
          return;
        }

        {
          // The frame is still fetching pictures from the body on screen -- the page reached it
          // long before they did. Letting go of it here, rather than when the effect that opened
          // it re-ran, is the whole point: closing it while it is on screen answered every one of
          // those pictures with nothing, and left a message whose text is there and whose pictures
          // are empty frames.
          const replaced = showing.current;

          showing.current = body.name;
          shownFor.current = messageId;
          // The answer and the question that produced it, kept together. What the notice says
          // depends on both -- "nobody asked for these" and "there is nowhere to get these" are
          // the same number under different questions -- and reading the number from one render
          // under the flag of the next says the second when the first is true, with no press
          // offered to change it.
          setWentMissing(false);
          setState({ status: "ready", body, asked: pictures });

          // A standing answer of "show them" is this press, made for the reader rather than by
          // them. It is made *after* the text is on screen and not before: the pictures are
          // fetched while the body is being built, six at a time with twenty seconds each, so
          // asking for them in the first breath is a message that shows nothing at all until the
          // slowest host has finished refusing. What happens instead is what a press does — the
          // text appears, the strip says the pictures are being fetched, and they arrive into a
          // message somebody is already reading.
          //
          // Only where there is something to fetch: a message pointing at nothing elsewhere is
          // finished, and asking again for it would be a second render for no picture.
          if (showPictures === "always" && !pictures && body.remoteBlocked > 0) {
            setPictures(true);
          }

          if (replaced !== null && replaced !== body.name) {
            void closeMessageBody(replaced).catch(() => {});
          }
        }
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
  }, [messageId, form, pictures, attempt, wearing, showPictures]);

  if (state.status === "loading") {
    return (
      <p className="m-auto max-w-md p-8 text-center text-sm text-muted-foreground">
        {t("reading.bodyLoading")}
      </p>
    );
  }

  if (state.status === "failed") {
    return (
      <p
        role="alert"
        className="m-auto max-w-md p-8 text-center text-sm text-muted-foreground"
      >
        {describeError(state.error, t)}
      </p>
    );
  }

  const { body } = state;

  // Every place this build reads a message in was taken — by other messages, most often by the
  // round that reads mail for the search index. Said as its own thing rather than as "nothing to
  // show": the message is fine, the moment was busy, and asking again is worth doing.
  if (body.kind === "busy") {
    return (
      <div
        aria-busy="true"
        aria-live="polite"
        className="m-auto flex max-w-md flex-col items-center gap-3 p-8 text-center"
      >
        <span
          aria-hidden="true"
          className="size-5 animate-spin rounded-full border-2 border-muted-foreground/30 border-t-muted-foreground motion-reduce:animate-none"
        />
        <p className="text-sm text-muted-foreground">{t("reading.bodyBusy")}</p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => setAttempt((n) => n + 1)}
        >
          {t("reading.bodyBusyAgain")}
        </Button>
      </div>
    );
  }

  if (body.kind === "nothing") {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="m-auto flex max-w-md flex-col items-center gap-3 p-8 text-center">
          <p className="text-sm text-muted-foreground">
            {t("reading.bodyEmpty")}
          </p>
          <p className="text-sm text-muted-foreground">
            {t("reading.bodyEmptyWayOut")}
          </p>
          <HowToLook form={form} onChange={setForm} />
        </div>
        {/* A message can have nothing to show and still carry files — one whose whole content is an
            attachment, or a reply too deep to draw whose pictures travelled inside it. They are
            listed here so they are never lost with the body. */}
        <Attachments messageId={messageId} files={body.attachments} />
      </div>
    );
  }

  // Three different reasons a picture is not on screen, and only one of them is worth a second
  // press. Told apart because "the host said no" and "we never asked" send a person to different
  // places -- and lumped together, the second sent them back to try again at what cannot succeed.
  //
  // Asked for and did not arrive: a host that was down a moment ago may not be.
  const notHad = body.remoteNotHad;
  // Never asked about at all: not an address this build fetches, or a picture the message says
  // travels inside it and then does not carry. Trying again changes nothing.
  const nowhereToGet = body.remoteBlocked;
  const missing = notHad + nowhereToGet;
  // These arrived and are not shown, so nothing is gained by asking for them again. Said as its
  // own sentence: a notice inviting someone to retry what cannot succeed wastes their time.
  const notShown = body.remoteNotShown;
  // The question this body answered, not the one on screen now: between a press and the answer
  // they differ, and that gap is a sentence about the wrong thing.
  const wasAskedFor = state.asked;
  // An answer to a different question is on its way: what is on screen was drawn for one setting of
  // the pictures and the reader has since asked for another. A picture is given twenty seconds to
  // arrive, six at a time, so this is seconds of a person looking at a screen that has not changed
  // -- and the press that started it still reading as a press they have not made yet.
  const fetching = wasAskedFor !== pictures;
  const notices = [
    !wasAskedFor && missing > 0 && t("reading.imagesBlocked", { count: missing }),
    wasAskedFor && notHad > 0 && t("reading.picturesNotHad", { count: notHad }),
    wasAskedFor &&
      nowhereToGet > 0 &&
      t("reading.picturesNowhereToGet", { count: nowhereToGet }),
    notShown > 0 && t("reading.picturesNotShown", { count: notShown }),
    wentMissing && t("reading.picturesWentMissing"),
    body.truncated && t("reading.bodyTruncated"),
  ].filter((notice): notice is string => typeof notice === "string");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {asked && (
        <LinkConfirmation
          asked={asked}
          onCancel={() => setAsked(null)}
          onFailed={() => setAsked({ ...asked, failed: true })}
        />
      )}
      <div className="flex items-center gap-3 border-b border-border bg-muted px-6 py-2">
        <p className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
          {/* Apart, because each is a sentence of its own and none ends with a stop. */}
          {notices.join(" \u00b7 ")}
        </p>
        {/* Offered only where a second press could change something: the ones this build never
            asked about are not going to arrive by being asked for again. */}
        {(fetching || wentMissing || (wasAskedFor ? notHad : missing) > 0) && (
          <button
            type="button"
            disabled={fetching}
            onClick={() => {
              setPictures(true);
              // Asking again when the answer is already yes: the attempt is what makes the request
              // happen a second time, and a host that was down a moment ago may not be.
              setAttempt((made) => made + 1);
            }}
            className="shrink-0 rounded border border-border px-2 py-0.5 text-xs disabled:opacity-60"
          >
            {fetching
              ? t("reading.fetchingPictures")
              : wasAskedFor || wentMissing
                ? t("reading.tryPicturesAgain")
                : t("reading.showPictures")}
          </button>
        )}
        <HowToLook form={form} onChange={setForm} />
      </div>
      <iframe
        title={t("reading.body")}
        src={body.url}
        sandbox=""
        referrerPolicy="no-referrer"
        className="min-h-0 w-full flex-1 border-0 bg-white dark:bg-neutral-900"
      />

      {/* Under the body rather than in it: what came with a message is not part of what it says,
          and a file listed inside the frame would be a file listed by the message itself. */}
      <Attachments messageId={messageId} files={body.attachments} />
    </div>
  );
}

/**
 * The files a message came with, each a button that saves it.
 *
 * Shown under a body that was drawn and under one there was nothing to draw alike: a message whose
 * whole content is an attachment, and a reply too deep to render whose pictures travelled inside it,
 * both have files to hand over and no body to hang them beneath. Nothing where the message carried
 * nothing.
 */
function Attachments({
  messageId,
  files,
}: {
  messageId: string;
  files: MessageBodyDto["attachments"];
}) {
  const { t, language } = useI18n();

  if (files.length === 0) return null;

  return (
    <div className="flex flex-col gap-1 border-t border-border px-6 py-3">
      <p className="text-xs font-medium text-muted-foreground">
        {t("reading.attachments")}
      </p>
      <ul className="flex flex-wrap gap-2">
        {files.map((file) => (
          <li key={file.at}>
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={t("reading.saveAttachment", { name: file.name })}
              onClick={() => {
                // Asked by position. The name is shown and never sent: two files may share
                // one, and where this goes is the system's dialog to decide.
                void saveAttachment(messageId, file.at).catch(() => {
                  /* Said by the dialog's own failure, or by nothing: closing it is an answer. */
                });
              }}
            >
              <FileName name={file.name} />
              <span className="ms-2 shrink-0 text-xs tabular-nums text-muted-foreground">
                {[kindOf(file.name), inRoundNumbers(file.bytes, language, t)]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </Button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * A file's name, cut in the middle where it does not fit, so that how it ends is always on screen.
 *
 * The two parts run left to right whatever script the name is in, and the ending, read left to
 * right as well, is always at the right edge. Laid out in the name's own direction, one
 * right-to-left letter at the front would turn the row around: the ending at the far left, drawn
 * `app.`, and the `.pdf` before it the last thing read. The start keeps the name's own direction
 * and gives way; the ending does not.
 *
 * The name never takes more than its room. An ending wider than the room is clipped at the room's
 * edge rather than drawn over the size beside it — no extension anything opens a file by is that
 * wide.
 *
 * Spaces are kept as written: a space just before the last dot is part of the name, and drawn as
 * two parts it would otherwise be dropped where they meet.
 */
function FileName({ name }: { name: string }) {
  const [start, ending] = startAndEnding(name);

  return (
    <span dir="ltr" className="flex min-w-0 max-w-56 overflow-hidden">
      <span dir="auto" className="overflow-hidden text-ellipsis whitespace-pre">
        {start}
      </span>
      {ending && <span className="shrink-0 whitespace-pre">{ending}</span>}
    </span>
  );
}

/**
 * The three ways to look at a message.
 *
 * Not a fallback that appears when something failed: a message that renders as almost nothing is
 * often doing that on purpose, and the reader is owed a way to look at what actually arrived
 * rather than a blank pane and no explanation.
 */
function HowToLook({
  form,
  onChange,
}: {
  form: BodyFormDto;
  onChange: (form: BodyFormDto) => void;
}) {
  const { t } = useI18n();
  const ways: [BodyFormDto, string][] = [
    ["sent", t("reading.asSent")],
    ["text", t("reading.asText")],
    ["source", t("reading.asSource")],
  ];

  return (
    <div
      role="group"
      aria-label={t("reading.howToLook")}
      className="flex shrink-0 gap-1 rounded-md border border-border p-0.5"
    >
      {ways.map(([way, label]) => (
        <button
          key={way}
          type="button"
          aria-pressed={form === way}
          onClick={() => onChange(way)}
          className={`rounded px-2 py-0.5 text-xs ${
            form === way ? "bg-background font-medium" : "text-muted-foreground"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
