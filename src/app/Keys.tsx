import { useEffect, useRef } from "react";

import type { TranslationKey, Translator } from "@/shared/i18n";
import { chordName, keyNames } from "@/shared/platform";
import { Button } from "@/shared/ui/primitives/button";

/**
 * The keys the window answers to.
 *
 * Shown rather than left to be discovered: a shortcut nobody knows about is a shortcut that does
 * not exist, and the one that opens this list is the only one worth remembering unaided.
 */
export function Keys({ onClose, t }: { onClose: () => void; t: Translator }) {
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    close.current?.focus();
  }, []);

  const named = keyNames(t);
  // A letter and the chord that asks for the same, where there is one: the letter for somebody
  // who keeps their hands where they are, the chord for somebody another mail application taught
  // it to. A chord is named the way the system in front of the person names it: symbols on macOS,
  // words on the other two.
  const rows: { keys: string[]; what: TranslationKey }[] = [
    { keys: ["c", chordName(named, "N")], what: "keys.compose" },
    { keys: ["r", chordName(named, "R")], what: "keys.answer" },
    { keys: ["a", chordName(named, "R", true)], what: "keys.answerAll" },
    { keys: ["f", chordName(named, "F", true)], what: "keys.forward" },
    { keys: ["/"], what: "keys.search" },
    { keys: ["u"], what: "keys.markRead" },
    { keys: ["↑ ↓"], what: "keys.move" },
    { keys: ["← →"], what: "keys.conversation" },
    { keys: [named.enter], what: "keys.open" },
    {
      keys: [named.delete, chordName(named, named.delete)],
      what: "keys.throwAway",
    },
    { keys: [named.escape], what: "keys.leave" },
    { keys: ["?"], what: "keys.showThese" },
    { keys: [chordName(named, ",")], what: "keys.settings" },
  ];

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-6"
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.stopPropagation();
        onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="keys-title"
        className="flex w-full max-w-md flex-col gap-4 rounded-lg border border-border bg-background p-6 shadow-lg"
      >
        <h1 id="keys-title" className="text-lg font-semibold tracking-tight">
          {t("keys.title")}
        </h1>

        <dl className="flex flex-col gap-1.5 text-sm">
          {rows.map((row) => (
            <div key={row.what} className="flex items-baseline gap-3">
              <dt className="w-32 shrink-0 font-mono text-xs text-muted-foreground">
                {row.keys.join(" \u00b7 ")}
              </dt>
              <dd className="min-w-0 flex-1">{t(row.what)}</dd>
            </div>
          ))}
        </dl>

        <Button ref={close} variant="outline" onClick={onClose}>
          {t("keys.close")}
        </Button>
      </div>
    </div>
  );
}
