import { useEffect, useRef } from "react";

import { useI18n } from "@/shared/i18n";
import { openLink } from "@/shared/ipc/client";
import type { LinkAsked } from "@/shared/ipc/generated/LinkAsked";
import { Button } from "@/shared/ui/primitives/button";

/** A link somebody followed, waiting for an answer, and whether opening it has already failed once. */
export type Asked = { link: LinkAsked; failed: boolean };

/**
 * The question asked before anything leaves this machine.
 *
 * Both forms of the host are shown when they differ: a name that reads as one thing and is written
 * as another is the whole of a homograph attack, and showing only the readable half is how it
 * works. The address comes here only to be shown: what goes back is the name it is known by.
 */
export function LinkConfirmation({
  asked,
  onCancel,
  onFailed,
}: {
  asked: Asked;
  onCancel: () => void;
  onFailed: () => void;
}) {
  const { t } = useI18n();
  const { link } = asked;
  const refuse = useRef<HTMLButtonElement>(null);

  // Focus goes to the refusal, not to the opening. This panel appears because somebody clicked a
  // link inside a message, and a message is hostile input: a stray Enter must decline. Without
  // this, focus stayed wherever it was and the panel was a wall that keyboard could not answer.
  useEffect(() => {
    refuse.current?.focus();
  }, []);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-6"
      // Escape declines, the same key on all three systems.
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.stopPropagation();
        onCancel();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="link-title"
        className="flex w-full max-w-lg flex-col gap-4 rounded-lg border border-border bg-background p-6 shadow-lg"
      >
        <h1 id="link-title" className="text-lg font-semibold tracking-tight">
          {t("link.title")}
        </h1>
        {/* An address reads left to right in every language. Laid out right to left, the box
            opens on the end of a long one and its punctuation moves to the other side -- in the
            dialog a person reads to judge where a link from somebody else's mail leads. */}
        {link.host && (
          <p
            dir="ltr"
            className="text-xl font-semibold tracking-tight [text-align:match-parent]"
          >
            {link.host}
          </p>
        )}
        {link.hostAsWritten && (
          <p className="text-sm text-muted-foreground">
            {t("link.writtenAs", { written: link.hostAsWritten })}
          </p>
        )}
        <p
          dir="ltr"
          className="overflow-x-auto rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs"
        >
          {link.url}
        </p>
        <p className="text-sm text-muted-foreground">{t("link.body")}</p>
        {asked.failed && (
          <p role="alert" className="text-sm text-destructive">
            {t("link.failed")}
          </p>
        )}
        <div className="flex gap-2">
          <Button
            onClick={() => {
              openLink(link.name).then(onCancel).catch(onFailed);
            }}
          >
            {t("link.open")}
          </Button>
          <Button ref={refuse} variant="ghost" onClick={onCancel}>
            {t("link.cancel")}
          </Button>
        </div>
      </div>
    </div>
  );
}
