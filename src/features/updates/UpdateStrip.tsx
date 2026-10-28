import { CircleArrowUp } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  installTheUpdate,
  lookForAnUpdate,
  onUpdateComing,
  onUpdatePutting,
  restartIntoTheNewVersion,
} from "@/shared/ipc/client";
import type { OfferedDto } from "@/shared/ipc/generated/OfferedDto";
import { useI18n, type Translator } from "@/shared/i18n";
import type { TranslationKey } from "@/shared/i18n/catalog";
import { inRoundNumbers } from "@/shared/ui/bytes";
import { Button } from "@/shared/ui/primitives/button";
import { RailButton } from "@/shared/ui/RailButton";

/**
 * What the strip is doing, which is what decides what it says.
 *
 * `hidden` is not the same as `idle`: somebody who pressed *Hide this* is not asked again for this
 * version, and somebody who has not been offered anything has nothing to hide. Collapsing the two
 * would turn *no update* into *an update you dismissed*, and the strip would never come back.
 */
type Doing =
  | { at: "nothing" }
  | { at: "offered"; offered: OfferedDto }
  | { at: "hidden"; version: string }
  | { at: "coming"; offered: OfferedDto; soFar: number; whole: number | null }
  | { at: "putting"; offered: OfferedDto }
  | { at: "ready"; offered: OfferedDto }
  | { at: "failed"; offered: OfferedDto; why: TranslationKey };

/**
 * A strip in the sidebar's foot saying a newer version is out.
 *
 * It asks nothing: whether this copy looks for a newer version is answered on the onboarding and in
 * the settings, and in no third place.
 *
 * It draws nothing at all when there is nothing to say — the same rule the activity line beside it
 * lives by. A corner that announces *you are up to date* all day is a corner people stop reading,
 * and then it is not there for the day it matters.
 *
 * **On a background that stands out, and not the one a selected folder wears.** `bg-accent` and
 * `bg-muted` are the same value in this palette, and `bg-muted` is what a chosen mailbox already
 * carries — so an announcement wearing it would be pixel-identical to whichever folder somebody
 * happened to click.
 */
export function UpdateStrip({
  className = "",
  folded = false,
  onUnfold,
}: {
  /** Where it stands in the foot, which the foot decides: a line of its own, above the rest. */
  className?: string;
  /**
   * Drawn in the folded sidebar: a mark whose hint says what the strip would, and whose press
   * opens the sidebar, where the strip's buttons are. Hidden there instead, an offer and a
   * finished download would wait unseen for as long as somebody keeps the sidebar folded.
   */
  folded?: boolean;
  onUnfold?: () => void;
}) {
  const { t, language } = useI18n();
  const [doing, setDoing] = useState<Doing>({ at: "nothing" });

  // Held in a ref as well, because the listener below is set up once and would otherwise close over
  // the state as it was when it was made.
  const going = useRef(false);

  useEffect(() => {
    let stillHere = true;

    /**
     * Quietly, and without saying anything if it fails: nobody asked.
     *
     * Asked again while the window stays open, because a window somebody leaves up for a fortnight
     * would otherwise never hear about anything. **This is not how often the project is asked** —
     * that is decided below the window, which writes down when it last looked and refuses to look
     * again for a day. What this interval sets is how soon after that day passes the strip notices.
     */
    const ask = () => {
      lookForAnUpdate(false)
        .then((offered) => {
          // Never over something already happening. Somebody watching a download would otherwise
          // see the bar replaced by the offer it came from.
          if (stillHere && offered !== null)
            setDoing((was) => (was.at === "nothing" ? { at: "offered", offered } : was));
        })
        .catch(() => {
          /* A check nobody started reports nothing. */
        });
    };

    ask();
    const again = setInterval(ask, 60 * 60 * 1000);

    return () => {
      stillHere = false;
      clearInterval(again);
    };
  }, []);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let stillHere = true;

    onUpdateComing((how) => {
      if (!going.current) return;
      setDoing((was) =>
        was.at === "coming"
          ? { ...was, soFar: how.soFar, whole: how.whole }
          : was,
      );
    })
      .then((off) => {
        if (stillHere) stop = off;
        else off();
      })
      .catch(() => {
        /* Without it the bar does not move; the install still finishes. */
      });

    return () => {
      stillHere = false;
      stop?.();
    };
  }, []);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let stillHere = true;

    onUpdatePutting(() => {
      if (!going.current) return;
      setDoing((was) =>
        was.at === "coming" ? { at: "putting", offered: was.offered } : was,
      );
    })
      .then((off) => {
        if (stillHere) stop = off;
        else off();
      })
      .catch(() => {
        /* The bar stays where it was; the install still finishes. */
      });

    return () => {
      stillHere = false;
      stop?.();
    };
  }, []);

  const install = useCallback((offered: OfferedDto) => {
    going.current = true;
    setDoing({ at: "coming", offered, soFar: 0, whole: offered.bytes });

    installTheUpdate()
      .then((what) => {
        going.current = false;
        // The command asks the project again rather than installing what this window was shown
        // minutes ago, so "there is nothing newer after all" is an ordinary answer. Saying
        // "installed" over it would tell somebody their application had been replaced when nothing
        // was even fetched.
        setDoing(
          what === "swapped" ? { at: "ready", offered } : { at: "nothing" },
        );
      })
      .catch((reason: unknown) => {
        going.current = false;
        setDoing({ at: "failed", offered, why: whyOf(reason) });
      });
  }, []);

  const size = (bytes: number) => inRoundNumbers(String(bytes), language, t);
  const says = said(doing, size, t);

  if (says === null) return null;

  if (folded) {
    return (
      <div role="status" className={className}>
        <RailButton label={says} tone="news" onClick={() => onUnfold?.()}>
          <CircleArrowUp />
        </RailButton>
      </div>
    );
  }

  return (
    <div
      role="status"
      className={`flex flex-col gap-2 rounded-md border border-primary/30 bg-primary/10 px-2 py-2 ${className}`}
    >
      <span className="text-xs leading-snug">{says}</span>

      {doing.at === "offered" && (
        <div className="flex gap-1.5">
          <Button
            type="button"
            size="xs"
            onClick={() => install(doing.offered)}
          >
            {t("update.install")}
          </Button>
          <Button
            type="button"
            size="xs"
            variant="ghost"
            onClick={() =>
              setDoing({ at: "hidden", version: doing.offered.version })
            }
          >
            {t("update.dismiss")}
          </Button>
        </div>
      )}

      {/* A real bar where the size is known, and the count above where it is not. A bar drawn
        against a total nobody gave would be a bar that means nothing. */}
      {doing.at === "coming" && doing.whole !== null && (
        <progress
          className="h-1 w-full"
          aria-label={t("update.progressLabel")}
          value={doing.soFar}
          max={doing.whole}
        />
      )}

      {doing.at === "ready" && (
        <Button
          type="button"
          size="xs"
          onClick={() => {
            restartIntoTheNewVersion().catch(() => {
              /* Nothing returns from a restart that worked, so this is one that did not. The
                 version on disk is already the new one; opening it again by hand runs it. */
            });
          }}
        >
          {t("update.restart")}
        </Button>
      )}

      {doing.at === "failed" && (
        <div className="flex gap-1.5">
          <Button
            type="button"
            size="xs"
            onClick={() => install(doing.offered)}
          >
            {t("update.retry")}
          </Button>
          <Button
            type="button"
            size="xs"
            variant="ghost"
            onClick={() =>
              setDoing({ at: "hidden", version: doing.offered.version })
            }
          >
            {t("update.dismiss")}
          </Button>
        </div>
      )}
    </div>
  );
}

/**
 * What the strip says while it is doing something, or nothing while it is not: in its own words
 * when the sidebar is open, and in the hint of its mark when it is folded.
 */
function said(
  doing: Doing,
  size: (bytes: number) => string,
  t: Translator,
): string | null {
  switch (doing.at) {
    case "nothing":
    case "hidden":
      return null;
    case "offered":
      return t("update.offered", {
        version: doing.offered.version,
        size: size(doing.offered.bytes),
      });
    case "coming":
      return doing.whole === null
        ? t("update.arrived", { done: size(doing.soFar) })
        : t("update.coming", {
            done: size(doing.soFar),
            total: size(doing.whole),
          });
    case "putting":
      return t("update.putting");
    case "ready":
      return t("update.ready");
    case "failed":
      return t(doing.why);
  }
}

/**
 * Which sentence a failure gets.
 *
 * Two of them are worth telling apart and the rest are not. A download that did not arrive is a bad
 * afternoon; one carrying somebody else's signature is not, and a person deserves to be told which
 * of those happened rather than *something went wrong* over both.
 */
function whyOf(reason: unknown): TranslationKey {
  const code =
    typeof reason === "object" && reason !== null && "code" in reason
      ? String((reason as { code: unknown }).code)
      : "";

  // The download was somebody else's, or was not the version offered.
  if (code === "providerProtocol") return "update.notOurs";
  // The system this is running on does not replace an application from inside itself, which is
  // every copy on Windows and Linux. Read as a disk failure until it had a kind of its own.
  if (code === "notOursToReplace") return "update.notOursToReplace";
  if (code === "storageUnavailable") return "update.nowhereToPutIt";

  return "update.failed";
}
