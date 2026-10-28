import { CircleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import { useI18n, type Translator } from "@/shared/i18n";
import {
  howThingsAre,
  onMailChanged,
  whatIsBeingRead,
} from "@/shared/ipc/client";
import type { BeingReadDto } from "@/shared/ipc/generated/BeingReadDto";
import { RailButton } from "@/shared/ui/RailButton";
import { WHEN_IDLE, WHILE_READING, asFarAsMailboxes, worthSaying } from "./steady";
import type { HowThingsAreDto } from "@/shared/ipc/generated/HowThingsAreDto";

/**
 * The one place that answers two questions: is this mail current, and did what I asked for land.
 *
 * One line rather than a panel, because those are the only two questions, and a panel that has to
 * be opened is a panel nobody opens. It is quiet when there is nothing to say — an application
 * that announces "everything is fine" all day teaches people to stop reading it.
 */
export function ActivityLine({
  className = "",
  folded = false,
  onUnfold,
}: {
  /** Where it stands in the foot, which the foot decides: beside the gear, taking what is left. */
  className?: string;
  /**
   * Drawn in the folded sidebar, which has no room for a sentence: a round is its bar alone, and
   * what needs saying is a mark whose hint says it and whose press opens the sidebar. The words
   * are still the line's, for a screen reader, as they are when it is open.
   */
  folded?: boolean;
  onUnfold?: () => void;
}) {
  const { t, language } = useI18n();
  const [how, setHow] = useState<HowThingsAreDto | null>(null);
  /**
   * The last answer worth showing, and whether it is still worth showing.
   *
   * Kept apart from the answer itself because the two change at different moments: the count moves
   * with every question, and what is on screen moves by the rules in `steady.ts` — nothing said
   * about work that is over in a blink, and nothing taken away the instant it ends.
   */
  const [showing, setShowing] = useState<BeingReadDto | null>(null);
  const work = useRef<{ since: number | null; saidAt: number | null }>({
    since: null,
    saidAt: null,
  });

  useEffect(() => {
    let active = true;

    const ask = () => {
      howThingsAre()
        .then((answer) => active && setHow(answer))
        .catch(() => {
          /* Nothing to say is better than saying the wrong thing about how fresh the mail is. */
        });
    };

    ask();

    // Asked again when a pass changed something, and on a slow tick besides: "updated four minutes
    // ago" goes stale on its own, with nothing happening to announce it.
    //
    const ticking = window.setInterval(ask, 30_000);

    // The one question asked often, because it is the only thing here that moves while somebody is
    // looking. A round lasts a few seconds, so anything slower misses whole rounds. It costs a
    // number held in memory: no database, no mailbox opened, nothing that grows with how much mail
    // there is.
    //
    // Two things keep that cheap question from being asked for nothing. A window nobody can see is
    // not asked at all — a laptop with the lid shut would otherwise spend the night at four
    // questions a second — and a window where nothing is being read is asked at walking pace, then
    // put back on the fast tick the moment a round appears. A round lasts seconds, so noticing it
    // a second late still leaves the whole of it on screen.
    let watching: number | null = null;
    const watch = (every: number) => {
      if (watching !== null) window.clearInterval(watching);
      watching = window.setInterval(look, every);
    };

    function look() {
      whatIsBeingRead()
        .then((far) => {
          if (!active) return;

          // When this stretch of work began, remembered here because the answer does not carry it
          // and the rules below are about how long it has been going.
          const now = Date.now();
          const busy = far.asked > 0;
          const wasBusy = work.current.since !== null;
          if (busy && work.current.since === null) work.current.since = now;
          if (!busy) work.current.since = null;

          // The end of a stretch of reading is when the accounts not updating can have changed: a
          // refresh that read an account takes it off that count. Left to the slow tick, the line
          // went on counting it for up to half a minute after the button had finished.
          if (wasBusy && !busy) ask();

          // Held at the fast tick while anything is being read, and for as long after as the line
          // is still saying something — the last moment of a round is the one worth seeing.
          watch(busy || work.current.saidAt !== null ? WHILE_READING : WHEN_IDLE);

          const say = worthSaying({ busy, ...work.current }, now);
          if (say && busy) work.current.saidAt = now;

          // While the work is going, what is shown is the newest answer. When it has just ended,
          // what is shown is the end of it: every mailbox, every folder, a full bar. Held only to
          // the last mid-way answer, the line spent its final moment saying eight of nine about
          // work that was finished — and nine of nine was never seen at all, because the count of
          // what is being read goes back to nought the instant the last one stops.
          if (!say) setShowing(null);
          else if (busy) setShowing(far);
          else
            setShowing((held) =>
              held === null
                ? null
                : {
                    done: held.asked,
                    asked: held.asked,
                    through: held.of,
                    of: held.of,
                  },
            );
        })
        .catch(() => {
          /* A question that could not be asked is not news. */
        });
    }

    const whenSeen = () => {
      if (document.visibilityState === "hidden") {
        if (watching !== null) window.clearInterval(watching);
        watching = null;
        return;
      }

      look();
      watch(WHEN_IDLE);
    };

    whenSeen();
    document.addEventListener("visibilitychange", whenSeen);

    let stop: (() => void) | null = null;
    onMailChanged(ask)
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {});

    return () => {
      active = false;
      document.removeEventListener("visibilitychange", whenSeen);
      window.clearInterval(ticking);
      if (watching !== null) window.clearInterval(watching);
      stop?.();
      if (stop === null) {
        /* The listener may still be on its way; the flag above stops it doing anything. */
      }
    };
  }, []);

  // Worked out before anything that needs the database, and drawn whether or not that answered.
  // Behind a guard that waits on `how`, this disappears for the whole of every round: `how` is
  // unread exactly when every account is being read at once, because the question behind it waits
  // for a connection to the profile that nine passes are already using.
  const round = showing === null ? null : readingNow(showing, t, folded);

  const lines: string[] = [];
  const foot = () => (
    <Foot
      round={round}
      lines={lines}
      className={className}
      folded={folded}
      onUnfold={onUnfold}
    />
  );
  if (how === null) return round === null ? null : foot();

  if (how.stoppedBy === "storage") {
    lines.push(t("activity.storage"));
  } else if (how.stoppedBy !== null) {
    // Said with how long ago the mail was last current, and without it where it never was: a
    // sentence with "not yet checked" put where a time goes reads as a broken one.
    lines.push(
      how.lastSuccessAt === null
        ? t(
            how.stoppedBy === "offline"
              ? "activity.offlineNever"
              : how.stoppedBy === "signIn"
                ? "activity.signInNever"
                : "activity.otherNever",
          )
        : t(
            how.stoppedBy === "offline"
              ? "activity.offline"
              : how.stoppedBy === "signIn"
                ? "activity.signIn"
                : "activity.other",
            { when: howLongAgo(how.lastSuccessAt, language) },
          ),
    );
  } else if (how.accountsFailing > 0) {
    lines.push(t("activity.accountsFailing", { count: how.accountsFailing }));
  }

  // First, because it is about something a person wrote that did not go. A message the outbox will
  // not try again is back in the drafts, and the composer it was written in closed when it was
  // queued, so this line is the only thing that says so.
  if (how.cameBack > 0)
    lines.push(t("activity.cameBack", { count: how.cameBack }));

  // A message on its way, said once it is late: tried and not taken, or still waiting after a push
  // and a round. Until a server takes it, it is in no folder, so without this line nothing anywhere
  // would say it exists.
  if (how.sending > 0)
    lines.push(t("activity.sending", { count: how.sending }));
  if (how.waiting > 0)
    lines.push(t("activity.waiting", { count: how.waiting }));
  if (how.stopped > 0)
    lines.push(t("activity.stopped", { count: how.stopped }));


  // Quiet when everything is as it should be, and specific when it is not.
  if (lines.length === 0 && round === null) return null;

  return foot();
}

/**
 * The foot of the sidebar: what is happening now, and what needs saying.
 *
 * What needs saying stands beside the gear and wraps rather than being cut short. The foot costs
 * height only while there is something to say, and what is said is read whole by everybody: a
 * sentence cut off at the column's edge would put its end -- a message that came back to the
 * drafts -- behind a hint that only a pointer ever finds.
 */
function Foot({
  round,
  lines,
  className,
  folded,
  onUnfold,
}: {
  round: ReactNode | null;
  lines: string[];
  className: string;
  folded: boolean;
  onUnfold: (() => void) | undefined;
}) {
  return (
    <div
      role="status"
      className={`flex flex-col gap-1 text-xs leading-snug text-muted-foreground ${
        folded ? "items-center" : ""
      } ${className}`}
    >
      {round}
      {lines.length > 0 &&
        (folded ? (
          <RailButton
            label={lines[0] ?? ""}
            hints={lines.slice(1)}
            onClick={() => onUnfold?.()}
          >
            <CircleAlert />
          </RailButton>
        ) : (
          <p className="px-1">{lines.join(" · ")}</p>
        ))}
    </div>
  );
}

/**
 * A round under way, said while it is under way and gone the moment it is not.
 *
 * The only thing on this line that moves while somebody is watching it, so it is drawn above the
 * rest — and worked out before anything that needs the database, because the database is busy for
 * exactly as long as a round lasts.
 */
function readingNow(
  reading: BeingReadDto,
  t: Translator,
  folded: boolean,
): ReactNode {
  // How far through the burst the core says it is, and mailboxes where it says nothing.
  const along =
    reading.of > 0
      ? { sofar: reading.through, of: reading.of }
      : { sofar: reading.done, of: reading.asked };

  return (
    <div className="flex w-full flex-col gap-1 px-1">
      {/* Folded, the words are for a screen reader alone, and the bar says it to everybody else. */}
      <div
        className={
          folded ? "sr-only" : "flex items-baseline justify-between gap-2"
        }
      >
        <span className="truncate">
          {reading.asked === 1
            ? t("activity.checkingOne")
            : t("activity.checkingMail")}
        </span>
        {/* No count over one mailbox: "nought of one" about the only mailbox there is reads as
            something being wrong rather than as something being read. */}
        {reading.asked > 1 && (
          <span className="shrink-0 tabular-nums">
            {t("activity.howFar", {
              done: asFarAsMailboxes(reading),
              total: reading.asked,
            })}
          </span>
        )}
      </div>
      {/* Drawn rather than a `<progress>`, for one reason: a bar that jumps from one folder to
          the next in a single frame is a bar that flickers, and no engine will transition the
          element's own filling. Everything a reader needs is still said, by the attributes a
          `<progress>` would have carried.

          By folders, not by mailboxes. Nine mailboxes read at once start and finish together, so
          a bar counted by mailbox stands still for most of a round and then fills in one frame; a
          folder finishes every few hundred milliseconds, all round long. */}
      <div
        role="progressbar"
        aria-label={t("activity.howFarLabel")}
        aria-valuemin={0}
        aria-valuemax={along.of}
        aria-valuenow={along.sofar}
        className="h-0.5 w-full overflow-hidden rounded-full bg-muted"
      >
        <div
          className="h-full rounded-full bg-primary transition-[width] duration-500 ease-out motion-reduce:transition-none"
          style={{
            width: `${along.of > 0 ? (along.sofar / along.of) * 100 : 0}%`,
          }}
        />
      </div>
    </div>
  );
}

/**
 * When the mail was last current, said the way a person would say it.
 *
 * Rounded on purpose: nobody needs "updated 4 minutes and 12 seconds ago", and a line that changes
 * every second is a line that draws the eye for nothing.
 */
function howLongAgo(at: number, language: string): string {
  const seconds = Math.max(0, Math.round(Date.now() / 1000 - at));
  const relative = new Intl.RelativeTimeFormat(language, { numeric: "auto" });

  if (seconds < 60) return relative.format(-seconds, "second");
  if (seconds < 3600)
    return relative.format(-Math.round(seconds / 60), "minute");
  if (seconds < 86_400)
    return relative.format(-Math.round(seconds / 3600), "hour");

  return relative.format(-Math.round(seconds / 86_400), "day");
}
