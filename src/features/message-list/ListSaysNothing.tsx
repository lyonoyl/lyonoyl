import type { ReactNode } from "react";

import { describeError, type Translator } from "@/shared/i18n";
import type { CommandError } from "@/shared/ipc/generated/CommandError";

/**
 * What stands where the rows would be when there are none.
 *
 * A list that has emptied — a search that matched nothing, the last message thrown away, an
 * account that would not answer — is said out loud rather than only shown: it replaces what
 * somebody was reading, and a reader with no eyes on the screen is otherwise told nothing at all.
 *
 * The refusal and the emptiness are one piece because they stand in one place and must sit at one
 * height: the pane beside this centres its own line over the whole column, and these two are
 * centred over what is left under the search box.
 */
export function ListSaysNothing({
  error,
  unreadOnly = false,
  t,
  children,
}: {
  /** A refusal from the core, when that is why there is nothing. */
  error?: CommandError;
  /** Whether the list was showing only unread mail, which is a different kind of nothing. */
  unreadOnly?: boolean;
  t: Translator;
  /** What the caller would rather say than "this folder is empty". */
  children?: ReactNode;
}) {
  if (error) {
    return (
      <p role="alert" className="p-8 text-sm text-muted-foreground">
        {describeError(error, t)}
      </p>
    );
  }

  return (
    // Padding below of one search box raises a centred line by half of one, which is exactly how
    // much lower this sat than the "nothing here" line in the pane beside it.
    //
    // Padding rather than a margin. `m-auto` is what centres this, and a margin below replaces the
    // auto one — so the automatic space all went above and the line dropped to the floor.
    <div
      role="status"
      className="m-auto flex flex-col px-8 pb-[var(--search-row)]"
    >
      {children ?? (
        <p className="text-sm text-muted-foreground">
          {t(unreadOnly ? "list.nothingUnread" : "list.empty")}
        </p>
      )}
    </div>
  );
}
