import type { TranslationKey, Translator } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { FolderDto } from "@/shared/ipc/generated/FolderDto";
import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { FolderRoleDto } from "@/shared/ipc/generated/FolderRoleDto";

/**
 * How a folder is named in the interface.
 *
 * A folder with a role is named in the reader's language; a folder the user made keeps the name it
 * has on the server. Providers translate their own folder names, so showing those would put the
 * inbox under a different word in every account.
 */

/**
 * Exhaustive by construction: a role added in Rust regenerates the union and this map stops
 * compiling until it has a name.
 */
const ROLE_NAMES: Record<FolderRoleDto, TranslationKey | null> = {
  custom: null,
  inbox: "folder.inbox",
  sent: "folder.sent",
  drafts: "folder.drafts",
  trash: "folder.trash",
  junk: "folder.junk",
  archive: "folder.archive",
  allMail: "folder.allMail",
  flagged: "folder.flagged",
};

/**
 * What a folder shows beside its name, if anything.
 *
 * Unread only means something where mail arrives and waits to be read. A message in Sent was
 * written here, one in the Archive was filed on purpose, and one in the Trash was thrown away —
 * a number beside any of them asks for attention that nothing is owed. Drafts count themselves
 * instead: what matters there is how many are waiting to be finished — the ones the server's
 * folder holds and the ones kept on this device together, because the folder shows both.
 *
 * Exhaustive by construction, like ROLE_NAMES above.
 */
const ROLE_BADGES: Record<FolderRoleDto, "unread" | "total" | null> = {
  custom: "unread",
  inbox: "unread",
  junk: "unread",
  flagged: "unread",
  drafts: "total",
  sent: null,
  archive: null,
  trash: null,
  allMail: null,
};

export type FolderBadge = { count: number; label: TranslationKey };

export function folderBadge(
  folder: Pick<FolderDto, "role" | "unread" | "total" | "draftsHere">,
): FolderBadge | null {
  const shows = ROLE_BADGES[folder.role];
  if (shows === null) {
    return null;
  }

  const count =
    shows === "total" ? folder.total + folder.draftsHere : folder.unread;
  return count > 0
    ? {
        count,
        label: shows === "total" ? "folder.draftCount" : "folder.unread",
      }
    : null;
}

/**
 * A message a person marked read or unread, which the counts beside the folders may not hold yet.
 *
 * The dot on the row changes under the finger, and the numbers beside the folders have to change
 * in the same frame or the window says two things at once. The core counts the change the moment
 * it is written down; until the sidebar has read itself again after that, it is laid over what the
 * sidebar last read.
 */
export interface ReadChange {
  folderId: FolderIdDto;
  /** One fewer unread, or one more. */
  unread: -1 | 1;
  /**
   * Where the write stands among everything asked for, as the core answered it, or `null` until it
   * has. A read of the counts holds the change exactly when it says the queue had got this far:
   * the two numbers come from the core, so a read already on its way when the change was written
   * is told apart from one that began after, which no clock in the window can do.
   */
  asked: number | null;
  /** When it was asked for, on the clock `performance.now()` keeps: how long it is kept by. */
  askedAt: number;
}

/**
 * The accounts as the sidebar last read them, with the read changes those counts do not hold.
 *
 * A folder's own count moves, and so does its account's when the folder is an inbox — which is
 * what every inbox together and a group add up. `readThrough` is how far the queue had got when the
 * counts being shown were read. A change not answered yet is laid over whatever was read: a read
 * can hold it only once it is written, and the answer follows the writing by the moment it takes
 * to say so.
 */
export function withReadChanges(
  accounts: AccountNodeDto[],
  changes: ReadChange[],
  readThrough: number,
): AccountNodeDto[] {
  const counted = changes.filter(
    (change) => change.asked === null || change.asked > readThrough,
  );
  if (counted.length === 0) return accounts;

  return accounts.map((account) => {
    let moved = false;
    let inbox = 0;
    const folders = account.folders.map((folder) => {
      const by = counted
        .filter((change) => change.folderId === folder.folderId)
        .reduce((sum, change) => sum + change.unread, 0);
      if (by === 0) return folder;
      moved = true;
      if (folder.role === "inbox") inbox += by;
      return { ...folder, unread: Math.max(0, folder.unread + by) };
    });

    return moved
      ? { ...account, folders, unread: Math.max(0, account.unread + inbox) }
      : account;
  });
}

/** An account's own row, and the row for all inboxes at once: both already count only inboxes. */
export function unreadBadge(count: number): FolderBadge | null {
  return count > 0 ? { count, label: "folder.unread" } : null;
}

export function folderName(
  folder: Pick<FolderDto, "name" | "role">,
  t: Translator,
): string {
  const key = ROLE_NAMES[folder.role];
  return key ? t(key) : folder.name;
}
