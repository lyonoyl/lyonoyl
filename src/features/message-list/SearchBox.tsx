import { useEffect, useRef, useState } from "react";

import { useI18n } from "@/shared/i18n";
import { Input } from "@/shared/ui/primitives/input";
import { Button } from "@/shared/ui/primitives/button";
import { typedDirection } from "@/shared/ui/direction";

/**
 * How long the typing settles before the list is asked again.
 *
 * Short enough that the answer feels like it belongs to the last key pressed, long enough that a
 * word typed at speed is one question rather than six.
 */
export const SETTLES_AFTER_MS = 120;

/**
 * The box that narrows the list to what somebody is looking for.
 *
 * What is typed is reported as typed. Turning it into a question the index understands happens in
 * the core, in one place, because the index speaks a language with operators and quotes and
 * nothing typed here may reach it as itself.
 */
export function SearchBox({
  onLookFor,
  autoFocusKey,
}: {
  /** Called with what to look for, or nothing when the box is empty again. */
  onLookFor: (typed: string) => void;
  /** Changing this clears the box: a different mailbox is a different question. */
  autoFocusKey?: string;
}) {
  const { t } = useI18n();
  const [typed, setTyped] = useState("");
  const field = useRef<HTMLInputElement>(null);
  const report = useRef(onLookFor);
  report.current = onLookFor;

  // A different mailbox is a different question, and carrying the old one into it would show
  // somebody a list they did not ask for.
  useEffect(() => {
    setTyped("");
    report.current("");
  }, [autoFocusKey]);

  useEffect(() => {
    const waiting = setTimeout(() => report.current(typed), SETTLES_AFTER_MS);
    return () => clearTimeout(waiting);
  }, [typed]);

  function clear() {
    setTyped("");
    field.current?.focus();
  }

  return (
    <div className="flex h-[var(--search-row)] items-center gap-1 border-b border-border px-2">
      <Input
        // How the window's "/" finds this field. A marker rather than an id, because an id is
        // a promise that there is one of these, and a second list would break it quietly.
        data-search
        ref={field}
        type="search"
        role="searchbox"
        dir={typedDirection(typed)}
        value={typed}
        aria-label={t("search.label")}
        placeholder={t("search.placeholder")}
        onChange={(event) => setTyped(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && typed !== "") {
            // Escape empties the box before it does anything else in the window, because that is
            // what somebody who has just searched expects it to do.
            event.stopPropagation();
            clear();
          }
        }}
        className="h-8 border-0 bg-transparent shadow-none focus-visible:ring-0"
      />
      {typed !== "" ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 shrink-0 px-2 text-xs"
          onClick={clear}
        >
          {t("search.clear")}
        </Button>
      ) : null}
    </div>
  );
}
