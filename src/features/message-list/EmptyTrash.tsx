import { useState } from "react";

import { emptyFolder } from "@/shared/ipc/client";
import type { Translator } from "@/shared/i18n";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import { Button } from "@/shared/ui/primitives/button";

/**
 * The way to be rid of everything the trash holds, offered where the trash is open.
 *
 * Floating over the list rather than sitting under it, so opening the trash does not shove its
 * rows up by the height of an offer -- and so it is in the same corner as the notice about a
 * message just filed, which is the other thing the list has to say about itself.
 *
 * It asks first, and says how many. There is no way back from this: what is queued is one
 * permanent deletion per message, and the trash is exactly the place where the way back used to
 * be. A single press that emptied it would be a person one slip away from mail nobody can return.
 */
export function EmptyTrash({
  folderId,
  holds,
  onEmptied,
  t,
}: {
  folderId: FolderIdDto;
  /** How many the folder holds, which is what the question names. */
  holds: number;
  /** Told once the instructions are queued, so the list reads itself again. */
  onEmptied: () => void;
  t: Translator;
}) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);

  if (holds === 0) return null;

  const empty = () => {
    if (busy) return;
    setBusy(true);

    emptyFolder(folderId)
      .then(() => {
        setAsking(false);
        onEmptied();
      })
      .catch(() => {
        // Nothing was queued and nothing changes on screen, which is the honest answer to "it did
        // not happen". The offer stays where it was, to be pressed again or left alone.
      })
      .finally(() => setBusy(false));
  };

  return (
    <div className="pointer-events-auto absolute inset-x-3 bottom-3 z-10 flex items-center gap-3 rounded-lg border border-border bg-popover/95 px-3 py-2 text-sm shadow-lg backdrop-blur">
      <span className="min-w-0 flex-1 text-muted-foreground">
        {asking
          ? t("trash.emptyAsks", { count: holds })
          : t("trash.holds", { count: holds })}
      </span>

      {asking ? (
        <>
          <Button
            type="button"
            variant="destructive"
            size="sm"
            disabled={busy}
            onClick={empty}
          >
            {t("trash.emptyForGood")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => setAsking(false)}
          >
            {t("trash.keepThem")}
          </Button>
        </>
      ) : (
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => setAsking(true)}
        >
          {t("trash.empty")}
        </Button>
      )}
    </div>
  );
}
