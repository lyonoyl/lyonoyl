import { useCallback, useState } from "react";

import type { ReadChange } from "@/entities/folder";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";

/** How long a change is kept: far longer than the sidebar takes to read itself again. */
const KEPT_FOR_MS = 60_000;

/**
 * The read and unread changes a person made that the counts beside the folders may not hold yet.
 *
 * One rule, and the reason this is a hook of its own: a change is laid over counts read before the
 * core wrote it down, and never over counts read after. Laid over those too, the count moves twice
 * for one message; dropped from the first, the number beside the folder goes back up for as long
 * as a read takes. Which is which the core says, by the number the write answers with. A write that
 * fails takes its change away, because the core never counted it.
 *
 * Answers with the changes, and with what the list calls when it marks a message: the folder it is
 * in, whether it is now read, and the write that says so.
 */
export function useReadChanges(): [
  ReadChange[],
  (folderId: FolderIdDto, read: boolean, written: Promise<number>) => void,
] {
  const [changes, setChanges] = useState<ReadChange[]>([]);

  const marking = useCallback(
    (folderId: FolderIdDto, read: boolean, written: Promise<number>) => {
      const askedAt = performance.now();
      const change: ReadChange = {
        folderId,
        unread: read ? -1 : 1,
        asked: null,
        askedAt,
      };

      setChanges((held) => [
        ...held.filter((one) => askedAt - one.askedAt < KEPT_FOR_MS),
        change,
      ]);
      written.then(
        (asked) =>
          setChanges((held) =>
            held.map((one) => (one === change ? { ...one, asked } : one)),
          ),
        () => setChanges((held) => held.filter((one) => one !== change)),
      );
    },
    [],
  );

  return [changes, marking];
}
