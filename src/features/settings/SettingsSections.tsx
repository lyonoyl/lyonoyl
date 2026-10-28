import { useEffect, useState } from "react";

import { useI18n } from "@/shared/i18n";
import { ColumnHeading } from "@/shared/ui/ColumnHeading";

import { SETTINGS_SECTIONS, anchorFor } from "./sections";

/**
 * The list of the settings, standing where the mailboxes stand the rest of the time.
 *
 * The settings cover the list and the message, and the column beside them was still offering the
 * folders of an account — rows that answer a question nobody is asking while this screen is open.
 * What belongs there is what the screen itself is made of: its parts, in their order, each one a
 * way to reach that part and a sign of which part is being read.
 *
 * It reaches the screen through the document rather than through props. The two stand in different
 * columns and neither contains the other, so passing this down would mean a piece of state in the
 * window's shell whose only readers are these two.
 */
/** Long enough to be found and short enough not to be read as a selection. */
export const ARRIVAL_SHOWS_FOR_MS = 1800;

export function SettingsSections() {
  const { t } = useI18n();
  const [current, setCurrent] = useState<string | null>(null);

  // Which part is being read, from what is actually on screen rather than from the last press: a
  // list that only followed presses would stop agreeing with the screen the moment somebody
  // scrolled instead. The bottom is discounted so the part named is the one at the top of the
  // pane, which is the one somebody is looking at.
  useEffect(() => {
    const parts = SETTINGS_SECTIONS.map((one) =>
      document.getElementById(anchorFor(one.id)),
    ).filter((one): one is HTMLElement => one !== null);
    if (parts.length === 0) return;

    const onScreen = new Set<string>();
    const watcher = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) onScreen.add(entry.target.id);
          else onScreen.delete(entry.target.id);
        }
        // The first in the screen's own order, not the first the observer happened to report.
        const first = parts.find((part) => onScreen.has(part.id));
        if (first) setCurrent(first.id);
      },
      // A line across the pane rather than a band down from the top. A band catches the part
      // above as well -- a tall one still hangs into it -- and the earlier of the two always won,
      // so the list named the part before the one that had just been scrolled to. A line is
      // crossed by exactly one part, which is the one being read.
      { rootMargin: "-40% 0px -60% 0px" },
    );

    for (const part of parts) watcher.observe(part);
    return () => watcher.disconnect();
  }, []);

  return (
    <div className="flex min-h-0 flex-col gap-3">
      <ColumnHeading title={t("settings.title")} />

      {/* Spaced rather than stacked. The one being read wears a filled background, and a
          neighbour lit up under the pointer ran straight into it -- two rows reading as one
          block, with no telling where either began. */}
      <nav aria-label={t("settings.title")} className="flex flex-col gap-1.5">
        {SETTINGS_SECTIONS.map((one) => {
          const name = anchorFor(one.id);
          const here = current === name;
          return (
            <button
              key={one.id}
              type="button"
              // Said rather than only coloured: which part is being read is the whole point of
              // the list, and a colour says it to one kind of reader.
              aria-current={here ? "true" : undefined}
              // No transition on the colour. Lighting up a moment after the pointer arrives
              // reads as the window thinking about it; the mailboxes this stands in place of
              // answer at once, and so does this.
              className={`rounded-md px-2 py-1.5 text-start text-sm ${
                here
                  ? "bg-muted font-medium text-foreground"
                  : "text-muted-foreground hover:bg-muted/60"
              }`}
              onClick={() => {
                const part = document.getElementById(name);
                if (!part) return;
                // Named at once, so the press answers even before the scrolling has finished --
                // and before the watcher has anything to say about where the screen ended up.
                setCurrent(name);
                // Say where it landed. The screen beside this is a long column of the same shapes,
                // and a scroll that ends somewhere in it leaves a person hunting for the part they
                // asked for. The mark fades on its own: one that stayed would be a second
                // selection on a screen that has one.
                // How long, handed to the stylesheet rather than written down twice: the mark
                // has to come off exactly when it has finished fading, and two numbers in two
                // files drift the moment one of them is tuned.
                part.style.setProperty(
                  "--arrived-for",
                  `${ARRIVAL_SHOWS_FOR_MS}ms`,
                );
                part.dataset.arrived = "";
                window.setTimeout(() => {
                  delete part.dataset.arrived;
                  part.style.removeProperty("--arrived-for");
                }, ARRIVAL_SHOWS_FOR_MS);
                part.scrollIntoView({
                  behavior: window.matchMedia(
                    "(prefers-reduced-motion: reduce)",
                  ).matches
                    ? "auto"
                    : "smooth",
                  block: "start",
                });
              }}
            >
              {t(one.title)}
            </button>
          );
        })}
      </nav>
    </div>
  );
}
