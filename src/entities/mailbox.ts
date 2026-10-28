import type { FolderIdDto } from "@/shared/ipc/generated/FolderIdDto";
import type { FolderRoleDto } from "@/shared/ipc/generated/FolderRoleDto";
import type { ViewScopeDto } from "@/shared/ipc/generated/ViewScopeDto";

/**
 * What the message list is showing.
 *
 * One value covers the three things the sidebar can select, which is what lets the list, the
 * reading pane and the selection in the tree agree without any of them holding a second copy of
 * the answer.
 */
export type Mailbox =
  | { kind: "everything" }
  | { kind: "group"; groupId: string }
  | { kind: "account"; accountId: string }
  | {
      kind: "folder";
      accountId: string;
      folderId: FolderIdDto;
      /**
       * What the folder is for, as the sidebar knows it.
       *
       * Carried with the selection rather than looked up again: the window asks one question of
       * it -- is this the trash -- and looking that up a second time would mean a second place
       * that decides what the trash is.
       */
      role?: FolderRoleDto;
    };

/** The account a selection belongs to, or `null` when it spans more than one. */
export function accountOf(mailbox: Mailbox): string | null {
  if (mailbox.kind === "everything" || mailbox.kind === "group") return null;
  return mailbox.accountId;
}

/** How the core is asked for it. The folder scope carries no account: a folder names one already. */
export function scopeOf(mailbox: Mailbox): ViewScopeDto {
  switch (mailbox.kind) {
    case "everything":
      return { kind: "everything" };
    // Named by the group rather than by the accounts in it: which accounts those are is read at
    // the moment the list is drawn, so a group edited in another window cannot leave the list
    // showing a set nobody chose.
    case "group":
      return { kind: "group", groupId: mailbox.groupId };
    case "account":
      return { kind: "account", accountId: mailbox.accountId };
    case "folder":
      return { kind: "folder", folderId: mailbox.folderId };
  }
}

export function isSameMailbox(left: Mailbox, right: Mailbox): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind === "everything") return true;
  if (left.kind === "group")
    return left.groupId === (right as typeof left).groupId;
  if (left.kind === "account")
    return left.accountId === (right as typeof left).accountId;
  return left.folderId === (right as typeof left).folderId;
}

/**
 * A key that changes when the selection changes.
 *
 * Scrolling position and loaded pages belong to one selection; carrying them into the next one
 * would show the previous mailbox's rows at the new one's positions. A string rather than the
 * object itself, because the object is rebuilt on every render and would look new every time.
 */
export function scopeKey(scope: ViewScopeDto): string {
  switch (scope.kind) {
    case "everything":
      return "everything";
    // The group is in the key, so moving between two groups is a different list rather than the
    // same one showing other rows: carrying the scroll position across would land somebody
    // halfway down a mailbox they did not open.
    case "group":
      return `group:${scope.groupId}`;
    case "account":
      return `account:${scope.accountId}`;
    case "folder":
      return `folder:${scope.folderId}`;
    // One key for every search, not one per keystroke: what is looked for changes as somebody
    // types, and a key that changed with it would throw away the scroll position and the loaded
    // rows on every letter.
    case "looking":
      return "looking";
  }
}

/**
 * What the list is asking, as a string that changes whenever the answer would.
 *
 * Not `scopeKey`. That one identifies the list on the screen and deliberately does not move while
 * somebody types, or every letter would remount it and take the scroll position and the loaded
 * rows with it. But a list that is not remounted still has to ask again when the words change,
 * and reading the identity of the question off the identity of the list meant it never did: the
 * list answered whatever had settled first and then held it until the box was emptied.
 */
export function questionKey(scope: ViewScopeDto): string {
  return scope.kind === "looking" ? `looking:${scope.typed}` : scopeKey(scope);
}

export function mailboxKey(mailbox: Mailbox): string {
  return scopeKey(scopeOf(mailbox));
}

/**
 * Where a person left a mailbox: the message they had open there, the row it was on, and the row
 * at the top of the list.
 *
 * Kept for the session and handed back when they return, so going to another folder and back is
 * coming back to the same place rather than to the top of a list with nothing chosen. The top is a
 * row rather than a distance scrolled, so a list drawn at another density comes back to the same
 * messages.
 */
export interface LeftAt {
  messageId: string | null;
  folderId: FolderIdDto | null;
  /**
   * The conversation the message was chosen in, where the list is one of conversations: its row
   * stands for the message when the message is not the one the row shows.
   */
  conversationId: string | null;
  /** The row the message was last seen at, which is where it is looked for first. */
  at: number | null;
  /**
   * How many the list held when it was seen there: mail that arrives while the person is away
   * moves it down by as many, and how far to look for it on the way back is counted from this.
   */
  held?: number | null;
  top: number;
}

/**
 * Where the list is to put a person coming back to a mailbox: the message the reader shows, and
 * what is remembered of where it was — only when it is the message remembered there.
 */
export function returningTo(
  left: LeftAt | undefined,
  reading: { messageId: string; folderId: FolderIdDto | null } | null,
): LeftAt {
  const same = left !== undefined && left.messageId === reading?.messageId;
  return {
    messageId: reading?.messageId ?? null,
    folderId: reading?.folderId ?? null,
    conversationId: same ? left.conversationId : null,
    at: same ? left.at : null,
    held: same ? left.held : null,
    top: left?.top ?? 0,
  };
}

/**
 * Whether the list for this scope shows conversations rather than messages: one row per thread,
 * named by the newest message in it. A folder shows its messages, and a search the messages that
 * matched.
 */
export function listsConversations(scope: ViewScopeDto): boolean {
  return (
    scope.kind === "account" ||
    scope.kind === "everything" ||
    scope.kind === "group"
  );
}

/**
 * What a place is remembered under, or nothing for a list that is not a place to come back to.
 *
 * A search is answered afresh for every letter typed, and a place in one answer is not a place in
 * the next.
 */
export function leftAtKey(scope: ViewScopeDto): string | null {
  return scope.kind === "looking" ? null : scopeKey(scope);
}
