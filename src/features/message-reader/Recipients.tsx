import { useEffect, useRef, useState } from "react";

import { useI18n } from "@/shared/i18n";
import type { RecipientsDto } from "@/shared/ipc/generated/RecipientsDto";
import { Button } from "@/shared/ui/primitives/button";

/** How many of a long list are shown until the rest is asked for. */
const FIRST = 3;

/**
 * The people one header of a message names: their addresses, joined the way the language joins a
 * list, as the folders a message is in are, and each set apart from the words around it so that it
 * reads as written in a window that runs right to left.
 *
 * A long list shows its first three, and how many more there are as the last of the list, so a
 * message written to forty people does not push its own text off the screen. The number is a
 * button that shows the rest, and a button after them folds them away again. Four are shown whole:
 * a button standing for one address takes the room the address would.
 */
export function Recipients({ recipients }: { recipients: RecipientsDto }) {
  const { t, language } = useI18n();
  const [whole, setWhole] = useState(false);
  /** The button that folds or unfolds, which the keyboard that pressed the other one moves to. */
  const toggle = useRef<HTMLButtonElement>(null);
  const followed = useRef(false);

  useEffect(() => {
    if (!followed.current) return;
    followed.current = false;
    toggle.current?.focus();
  }, [whole]);

  const folds = recipients.shown.length > FIRST + 1;
  const folded = folds && !whole;
  const shown = folded ? recipients.shown.slice(0, FIRST) : recipients.shown;
  // The ones the core did not hand over are counted too: past a hundred, and any no address is.
  const more = recipients.all - shown.length;
  const counted = more > 0 ? [t("reading.moreRecipients", { count: more })] : [];

  const parts = new Intl.ListFormat(language).formatToParts([...shown, ...counted]);
  const last = parts.map((part) => part.type).lastIndexOf("element");
  const fold = (event: React.MouseEvent<HTMLButtonElement>) => {
    // Only a button that had the focus hands it on: a click does not take it.
    followed.current = document.activeElement === event.currentTarget;
    setWhole(!whole);
  };
  const button = (said: string) => (
    <Button
      ref={toggle}
      type="button"
      variant="link"
      size="xs"
      aria-expanded={whole}
      className="h-auto px-0 align-baseline text-sm font-normal"
      onClick={fold}
    >
      {said}
    </Button>
  );

  return (
    <>
      {parts.map((part, at) => {
        if (part.type !== "element") return part.value;
        if (at === last && counted.length > 0) {
          return folded ? (
            <span key={at}>{button(part.value)}</span>
          ) : (
            <span key={at}>{part.value}</span>
          );
        }
        return <bdi key={at}>{part.value}</bdi>;
      })}
      {folds && !folded && <> {button(t("reading.fewerRecipients"))}</>}
    </>
  );
}
