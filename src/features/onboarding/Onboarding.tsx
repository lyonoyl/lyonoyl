import { Check } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useI18n, type TranslationKey } from "@/shared/i18n";
import {
  appIconsWearable,
  setAppIcon,
  setLookForUpdates,
  setShareAnonymousData,
  setWelcomed,
} from "@/shared/ipc/client";
import type { AppIconDto } from "@/shared/ipc/generated/AppIconDto";
import type { LookForUpdatesDto } from "@/shared/ipc/generated/LookForUpdatesDto";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import { AppIcons, appIconPicture } from "@/shared/ui/AppIcons";
import { Checkbox } from "@/shared/ui/Checkbox";
import { Button } from "@/shared/ui/primitives/button";

/** What the second step says Lyonoyl Mail does for a person, in the order it is read. */
const PROMISES: ReadonlyArray<TranslationKey> = [
  "onboarding.promiseLocal",
  "onboarding.promiseNoCode",
  "onboarding.promiseNoTracking",
  "onboarding.promiseLinks",
  "onboarding.promisePasswords",
  "onboarding.promiseEncrypted",
  "onboarding.promiseNothingLeaves",
  "onboarding.promiseFlawsDealtWith",
];

/**
 * How far the lines of a list reach from its start edge, wrapped as they stand: the width that keeps
 * every line where it is. Zero where nothing has been laid out.
 *
 * Read from the lines of text rather than from the items, whose boxes are as wide as the room they
 * were given whether a line fills it or not.
 */
export function widestLine(list: HTMLElement): number {
  const rtl = getComputedStyle(list).direction === "rtl";
  const edge = list.getBoundingClientRect();
  const range = document.createRange();
  const texts = document.createTreeWalker(list, NodeFilter.SHOW_TEXT);
  let widest = 0;
  for (let text = texts.nextNode(); text !== null; text = texts.nextNode()) {
    range.selectNodeContents(text);
    for (const line of Array.from(range.getClientRects())) {
      widest = Math.max(
        widest,
        rtl ? edge.right - line.left : line.right - edge.left,
      );
    }
  }
  return widest;
}

/** How long it takes to go once it is finished, as long as the stylesheet's `duration-300`. */
const LEAVING_MS = 300;

/**
 * What a person goes through before the window is theirs: a welcome, what Lyonoyl Mail protects
 * them from, the two questions it asks before anything leaves this device — whether anonymous
 * reports about failures may be shared, and whether a newer version may be looked for — and,
 * where the system lets a running application change its picture, which picture it wears.
 *
 * It covers the whole window, and the window is not seen until it is finished: there is no way to
 * leave it before the last step, by a cross or by Escape. Finished, it writes the answers and that it
 * was finished, and fades away with the window under it; the window is handed the settings once it
 * has gone, so the onboarding is not taken away in the middle of going. Until the settings it shows
 * have arrived it is the cover alone.
 */
export function Onboarding({
  preferences,
  onChanged,
  onDone,
}: {
  /** `null` until the settings have been read. */
  preferences: PreferencesDto | null;
  /** Called with the whole of the settings as the core stored them. */
  onChanged: (preferences: PreferencesDto) => void;
  /** Called once it is finished and gone. */
  onDone: () => void;
}) {
  const [leaving, setLeaving] = useState(false);
  // The latest of each, so the wait for the fading is started once and not again with each new
  // callback, and what was stored is handed over when it ends.
  const handOver = useRef({ onChanged, onDone });
  useLayoutEffect(() => {
    handOver.current = { onChanged, onDone };
  });
  const stored = useRef<PreferencesDto | null>(null);

  function gone() {
    if (stored.current !== null) handOver.current.onChanged(stored.current);
    handOver.current.onDone();
  }

  useEffect(() => {
    if (!leaving) return;
    const timer = window.setTimeout(gone, LEAVING_MS);
    return () => window.clearTimeout(timer);
  }, [leaving]);

  /** `null` where nothing was written: the person went on without the answers. */
  function finished(written: PreferencesDto | null) {
    stored.current = written;
    // At once for somebody who asked for less motion: a fade they turned off is a wait with
    // nothing to see.
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      gone();
      return;
    }
    setLeaving(true);
  }

  return (
    <div
      data-onboarding={leaving ? "leaving" : "shown"}
      className={`fixed inset-0 z-50 flex items-center justify-center bg-background p-6 ${
        leaving ? "animate-out fade-out-0 fill-mode-forwards duration-300" : ""
      }`}
    >
      {preferences !== null && (
        <Steps
          preferences={preferences}
          leaving={leaving}
          onChanged={onChanged}
          onFinished={finished}
        />
      )}
    </div>
  );
}

/**
 * The card and its steps: four, and a fifth for the picture where the system lets it be changed.
 *
 * The answers are written when it is finished and not before. Each box starts as the setting
 * stands, so going through it again changes nothing a person changed elsewhere.
 *
 * Every step stands in the same place, and the card is as tall as the tallest of them: moving
 * between them changes what is in the card, never its size or where its buttons are. The picture
 * and the heading keep their place on every step, and what is under them stands in the middle of
 * the room left between the heading and the buttons.
 */
function Steps({
  preferences,
  leaving,
  onChanged,
  onFinished,
}: {
  preferences: PreferencesDto;
  leaving: boolean;
  onChanged: (preferences: PreferencesDto) => void;
  onFinished: (written: PreferencesDto | null) => void;
}) {
  const { t, language } = useI18n();
  const ids = useId();
  const forward = useRef<HTMLButtonElement>(null);
  const backward = useRef<HTMLButtonElement>(null);
  const [step, setStep] = useState(0);
  const [keepToMyself, setKeepToMyself] = useState(
    !preferences.shareAnonymousData,
  );
  // Clear unless the stored answer is once a day: nobody having been asked yet starts it clear.
  const [lookForUpdates, setLook] = useState(
    preferences.lookForUpdates === "onceADay",
  );
  const [saving, setSaving] = useState(false);
  const [notSaved, setNotSaved] = useState(false);
  // Whether the picture is offered: asked once, and not offered until the answer is yes.
  const [wearable, setWearable] = useState(false);
  // The picture is put on the moment it is chosen rather than when the card is finished: the Dock
  // is where the choice is seen. The one on its way to the core is shown until the core answers,
  // and the core's word after that, so a choice it refused goes back to what the Dock still shows.
  const [choosing, setChoosing] = useState<AppIconDto | null>(null);
  const icon = choosing ?? preferences.appIcon;
  const titleOf = (index: number) => `${ids}-title-${index}`;
  const promises = useRef<HTMLUListElement>(null);

  // The promises as wide as their widest line once wrapped, so the list stands in the middle with
  // every line from one edge. Left to the items, its width is the card's wherever one item wraps,
  // and the lines stand against the start of the card.
  const fitPromises = useCallback(() => {
    const list = promises.current;
    if (list === null) return;
    list.style.width = "";
    const widest = widestLine(list);
    if (widest > 0) list.style.width = `${Math.ceil(widest)}px`;
  }, []);
  useLayoutEffect(fitPromises, [fitPromises, language]);
  useEffect(() => {
    const room = promises.current?.parentElement;
    if (!room || typeof ResizeObserver === "undefined") return;
    const watching = new ResizeObserver(fitPromises);
    watching.observe(room);
    return () => watching.disconnect();
  }, [fitPromises]);

  useEffect(() => {
    let current = true;
    appIconsWearable()
      .then((answer) => {
        if (current) setWearable(answer);
      })
      .catch(() => {
        /* Not offered: the four steps stand as they are. */
      });
    return () => {
      current = false;
    };
  }, []);

  function wear(chosen: AppIconDto) {
    setChoosing(chosen);
    setAppIcon(chosen)
      .then(onChanged)
      .catch(() => {
        /* Not kept, so not worn: the core's word stands. */
      })
      // Only the latest choice is let go of: an earlier answer arriving after it does not take
      // it off the screen.
      .finally(() => setChoosing((now) => (now === chosen ? null : now)));
  }

  function back() {
    setStep(step - 1);
    // On the first step the way back goes out of sight, and a focus a keyboard put on it would go
    // with it. Only that focus is moved: a press by the pointer leaves the focus where it was.
    if (step === 1 && document.activeElement === backward.current) {
      forward.current?.focus();
    }
  }

  async function finish() {
    setSaving(true);
    setNotSaved(false);
    let stored = preferences;
    try {
      const share = !keepToMyself;
      if (share !== stored.shareAnonymousData) {
        stored = await setShareAnonymousData(share);
      }
      const looking: LookForUpdatesDto = lookForUpdates ? "onceADay" : "never";
      if (looking !== stored.lookForUpdates) {
        stored = await setLookForUpdates(looking);
      }
      // Last, so a start after a failure halfway greets the person again rather than skipping
      // the questions it did not get answers to.
      if (!stored.welcomed) stored = await setWelcomed(true);
      onFinished(stored);
    } catch {
      // What was written is worn, what was not is said, and the step stays open to try again or
      // to go on without it.
      if (stored !== preferences) onChanged(stored);
      setNotSaved(true);
      setSaving(false);
    }
  }

  // Writing settings down failing is no reason to keep a person here: they go on, and the answers
  // are theirs to give again in the settings. Said on the last step.
  const unsaved = notSaved && (
    <div className="flex flex-col items-center gap-2">
      <p role="alert" className="max-w-md text-sm text-balance text-destructive">
        {t("onboarding.notSaved")}
      </p>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={leaving}
        onClick={() => onFinished(null)}
      >
        {t("onboarding.continueUnsaved")}
      </Button>
    </div>
  );

  const steps: ReadonlyArray<{
    picture: PictureOf | "icon";
    title: TranslationKey;
    content: ReactNode;
  }> = [
    {
      picture: "welcome",
      title: "onboarding.welcomeTitle",
      content: (
        <p className="max-w-md text-sm text-balance text-muted-foreground">
          {t("onboarding.welcomeBody")}
        </p>
      ),
    },
    {
      picture: "private",
      title: "onboarding.privateTitle",
      content: (
        // As wide as its longest line and standing in the middle, with every line starting at the
        // same edge; once laid out, as wide as its widest line after wrapping.
        <ul
          ref={promises}
          className="flex w-fit max-w-full flex-col gap-2 text-start text-sm"
        >
          {PROMISES.map((promise) => (
            <li key={promise} className="flex gap-2.5">
              <Check
                aria-hidden="true"
                className="mt-0.5 size-4 shrink-0 text-primary"
              />
              <span className="text-balance">{t(promise)}</span>
            </li>
          ))}
        </ul>
      ),
    },
    {
      picture: "reports",
      title: "onboarding.shareTitle",
      content: (
        <>
          <p className="max-w-md text-sm text-balance text-muted-foreground">
            {t("settings.shareAnonymousDataHint")}
          </p>
          <div className="text-start">
            <Checkbox
              label={t("onboarding.keepToMyself")}
              checked={keepToMyself}
              disabled={saving}
              onChange={setKeepToMyself}
            />
          </div>
        </>
      ),
    },
    {
      picture: "updates",
      title: "onboarding.updatesTitle",
      content: (
        <>
          <p className="max-w-md text-sm text-balance text-muted-foreground">
            {t("settings.checkForUpdatesHint")}
          </p>
          <div className="text-start">
            <Checkbox
              label={t("settings.checkForUpdates")}
              checked={lookForUpdates}
              disabled={saving}
              onChange={setLook}
            />
          </div>
          {/* The last step unless the picture comes after it. */}
          {!wearable && unsaved}
        </>
      ),
    },
    ...(wearable
      ? [
          {
            picture: "icon" as const,
            title: "onboarding.iconTitle" as const,
            content: (
              <>
                <p className="max-w-md text-sm text-balance text-muted-foreground">
                  {t("onboarding.iconBody")}
                </p>
                <AppIcons
                  chosen={icon}
                  labelledBy={titleOf(4)}
                  disabled={saving || leaving}
                  onChoose={wear}
                />
                {unsaved}
              </>
            ),
          },
        ]
      : []),
  ];
  const last = step === steps.length - 1;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleOf(step)}
      className={`flex w-full max-w-2xl flex-col rounded-xl border border-border bg-background shadow-lg ${
        leaving ? "animate-out zoom-out-95 fill-mode-forwards duration-300" : ""
      }`}
    >
      {/* One place for every step, out of sight and out of reach but the one shown. */}
      <div className="grid px-8 pt-10">
        {steps.map((one, index) => (
          <section
            key={one.title}
            aria-labelledby={titleOf(index)}
            aria-hidden={index === step ? undefined : true}
            inert={index !== step}
            data-step={index === step ? "shown" : "waiting"}
            className={`col-start-1 row-start-1 flex flex-col items-center text-center ${
              index === step
                ? "animate-in fade-in-0 duration-200 motion-reduce:animate-none"
                : "invisible"
            }`}
          >
            {one.picture === "icon" ? (
              // The picture itself, as the Dock shows it, where the other steps draw theirs.
              <img
                src={appIconPicture(icon)}
                alt=""
                draggable={false}
                className="size-24 shrink-0"
              />
            ) : (
              <Picture of={one.picture} />
            )}
            <h2
              id={titleOf(index)}
              className="mt-5 text-xl font-semibold tracking-tight"
            >
              {t(one.title)}
            </h2>
            <div
              data-under-the-heading
              className="flex w-full flex-1 flex-col items-center justify-center gap-4 py-5"
            >
              {one.content}
            </div>
          </section>
        ))}
      </div>

      <div className="flex items-center justify-between gap-4 px-8 pt-3 pb-8">
        <div aria-hidden="true" className="flex gap-1.5">
          {steps.map((one, index) => (
            <span
              key={one.title}
              className={`size-1.5 rounded-full transition-colors ${
                index === step ? "bg-primary" : "bg-muted-foreground/30"
              }`}
            />
          ))}
        </div>
        <p className="sr-only" aria-live="polite">
          {t("onboarding.step", { current: step + 1, total: steps.length })}
        </p>

        <div className="flex gap-2">
          <Button
            ref={backward}
            type="button"
            variant="ghost"
            size="lg"
            aria-hidden={step === 0 ? true : undefined}
            disabled={step === 0 || saving || leaving}
            onClick={back}
            className={`px-4 ${step === 0 ? "invisible" : ""}`}
          >
            {t("onboarding.back")}
          </Button>
          {/* Nothing has the focus at the start: the window was not opened from the keyboard, and a
              ring round the first button there is a ring nobody asked for. Tab reaches it. */}
          <Button
            ref={forward}
            type="button"
            size="lg"
            disabled={saving || leaving}
            onClick={last ? () => void finish() : () => setStep(step + 1)}
            className="px-4"
          >
            {/* Both words in one place, so the button is as wide as the longer of them and the
                one beside it stays where it is when the last step changes the word. */}
            <span className="grid">
              <span
                aria-hidden={last ? true : undefined}
                className={`col-start-1 row-start-1 ${last ? "invisible" : ""}`}
              >
                {t("onboarding.next")}
              </span>
              <span
                aria-hidden={last ? undefined : true}
                className={`col-start-1 row-start-1 ${last ? "" : "invisible"}`}
              >
                {t("onboarding.finish")}
              </span>
            </span>
          </Button>
        </div>
      </div>
    </div>
  );
}

type PictureOf = "welcome" | "private" | "reports" | "updates";

/**
 * A picture for each step: a soft disc, a shape in the quiet colour, and its point in the palette's
 * own, so every palette draws it in its own colours.
 */
function Picture({ of }: { of: PictureOf }) {
  return (
    <svg
      viewBox="0 0 96 96"
      aria-hidden="true"
      fill="none"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-24 shrink-0"
    >
      <circle cx="48" cy="48" r="46" className="fill-muted" />
      {of === "welcome" && (
        <>
          <rect
            x="24"
            y="38"
            width="48"
            height="32"
            rx="5"
            className="stroke-muted-foreground"
          />
          <path d="m26 41 22 15 22-15" className="stroke-muted-foreground" />
          <path
            d="M48 25c-1.5-3-7-2.7-7 1.3 0 2.9 3.3 5.3 7 7.7 3.7-2.4 7-4.8 7-7.7 0-4-5.5-4.3-7-1.3Z"
            strokeWidth={1.5}
            className="fill-primary stroke-primary"
          />
        </>
      )}
      {of === "private" && (
        <>
          <path
            d="M48 21 69 28.5V45c0 13.5-8.8 22.6-21 27-12.2-4.4-21-13.5-21-27V28.5Z"
            className="stroke-muted-foreground"
          />
          <path
            d="m38.5 46.5 6.5 6.5 13-13.5"
            strokeWidth={2.75}
            className="stroke-primary"
          />
        </>
      )}
      {of === "reports" && (
        <>
          <rect
            x="21"
            y="29"
            width="54"
            height="38"
            rx="5"
            className="stroke-muted-foreground"
          />
          <path d="M21 37h54" className="stroke-muted-foreground" />
          <path
            d="M28 53h9l4-8 5 15 5-12 3 5h14"
            strokeWidth={2.5}
            className="stroke-primary"
          />
        </>
      )}
      {of === "updates" && (
        // Two arrows going round, the way every system draws looking again.
        <g transform="translate(24 24) scale(2)" strokeWidth={1}>
          <path
            d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"
            className="stroke-muted-foreground"
          />
          <path d="M21 3v5h-5" className="stroke-primary" strokeWidth={1.25} />
          <path
            d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"
            className="stroke-muted-foreground"
          />
          <path d="M8 16H3v5" className="stroke-primary" strokeWidth={1.25} />
        </g>
      )}
    </svg>
  );
}
