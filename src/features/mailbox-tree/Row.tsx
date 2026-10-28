import {
  Archive,
  ChevronRight,
  CircleAlert,
  FilePen,
  Flag,
  Folder,
  Inbox,
  Mails,
  OctagonAlert,
  Send,
  Trash2,
  type LucideIcon,
} from "lucide-react";
import type { ReactNode } from "react";

import {
  folderBadge,
  folderName,
  unreadBadge,
  type FolderBadge,
} from "@/entities/folder";
import { isSameMailbox, type Mailbox } from "@/entities/mailbox";
import type { Translator } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { FolderRoleDto } from "@/shared/ipc/generated/FolderRoleDto";
import { RailButton } from "@/shared/ui/RailButton";

/**
 * How far in each level of the tree sits, as the row's own left padding.
 *
 * Depth is expressed here and nowhere else, and that is the whole of the rule: a row is a
 * full-width button, so padding inside it moves the words while the surface a person points at
 * stays the width of the sidebar. A wrapper with padding round a row moves the surface too, and
 * then the thing that lights up is a different width at every level and starts short of the edge.
 *
 * Every level clears the same chevron column, whether or not the row has a chevron, so the words
 * of a row that folds and the words of a row that does not sit on one line.
 */
export const INDENT = ["ps-8", "ps-10", "ps-12"] as const;

/** Where the chevron for a row at this depth sits, over the padding the row leaves for it. */
const CHEVRON = ["start-1", "start-3"] as const;

/**
 * The step a folder sits at, and it is the same step for every folder in the sidebar.
 *
 * A mailbox is a mailbox: letting the indent add up through a group put the folders of a grouped
 * account a step to the right of everybody else's, and the sidebar read as two ladders that
 * happened to be drawn in one column.
 */
export const FOLDER = 2;

/**
 * A row that folds: the row itself, with its chevron laid over the padding it left.
 *
 * Over it rather than beside it. A chevron standing before the row takes width from it, so the row
 * beside a chevron and the row without one are different widths and start in different places —
 * and the part a person points at is the part that shrinks. Laid over, the chevron costs the row
 * nothing, and it is a sibling rather than a child because a button inside a button is not a thing
 * a browser will draw.
 */
export function Folding({
  depth,
  label,
  expanded,
  onFold,
  children,
}: {
  depth: 0 | 1;
  label: string;
  expanded: boolean;
  onFold: () => void;
  children: ReactNode;
}) {
  return (
    <div className="relative">
      <button
        type="button"
        aria-label={label}
        aria-expanded={expanded}
        onClick={onFold}
        className={`absolute top-1/2 z-10 -translate-y-1/2 rounded-md p-1 text-muted-foreground hover:bg-muted/60 ${CHEVRON[depth]}`}
      >
        <ChevronRight
          className={`size-3.5 transition-transform ${expanded ? "rotate-90" : "rtl:rotate-180"}`}
        />
      </button>
      {children}
    </div>
  );
}

export function Row({
  label,
  title,
  badge,
  selected,
  depth = 0,
  onSelect,
  t,
  folded = false,
  icon = null,
}: {
  label: string;
  title?: string;
  badge: FolderBadge | null;
  selected: boolean;
  depth?: 0 | 1 | 2;
  onSelect: () => void;
  t: Translator;
  /** Drawn in the folded sidebar: the icon alone, with the name and the count in its hint. */
  folded?: boolean;
  /** What the row is drawn as when the sidebar is folded. */
  icon?: ReactNode;
}) {
  if (folded) {
    const hints: string[] = [];
    if (title !== undefined && title !== label) hints.push(title);
    if (badge !== null) hints.push(t(badge.label, { count: badge.count }));

    return (
      <RailButton
        label={label}
        hints={hints}
        current={selected}
        dot={badge !== null}
        onClick={onSelect}
      >
        {icon}
      </RailButton>
    );
  }

  return (
    <button
      type="button"
      title={title}
      aria-current={selected ? "true" : undefined}
      onClick={onSelect}
      className={`flex w-full items-center gap-2 rounded-md py-1.5 pe-2 text-start ${
        INDENT[depth]
      } ${
        selected
          ? "bg-muted font-medium text-foreground"
          : "text-muted-foreground hover:bg-muted/60"
      }`}
    >
      <span dir="auto" className="flex-1 truncate">
        {label}
      </span>
      {badge !== null && (
        <span
          aria-label={t(badge.label, { count: badge.count })}
          className="shrink-0 rounded-full bg-primary/10 px-1.5 text-xs text-primary"
        >
          {badge.count}
        </span>
      )}
    </button>
  );
}

/**
 * What a folder is drawn as in the folded sidebar, by what it is for.
 *
 * Exhaustive by construction, like the names in `entities/folder`: a role added in Rust stops this
 * compiling until it has an icon. A folder somebody made is `custom`, and so is one a newer build
 * marked in a way this one does not read, and both are drawn as a plain folder.
 */
const ROLE_ICONS: Record<FolderRoleDto, LucideIcon> = {
  custom: Folder,
  inbox: Inbox,
  sent: Send,
  drafts: FilePen,
  trash: Trash2,
  junk: OctagonAlert,
  archive: Archive,
  allMail: Mails,
  flagged: Flag,
};

function FolderIcon({ role }: { role: FolderRoleDto }) {
  const Icon = ROLE_ICONS[role];

  // The plane flies the way the window reads.
  return <Icon className={role === "sent" ? "rtl:-scale-x-100" : undefined} />;
}

/** A line across the folded sidebar, under the buttons that do something and above the mailboxes. */
export function RailRule() {
  return <div aria-hidden="true" className="my-1 h-px w-8 shrink-0 bg-border" />;
}

/**
 * The mailboxes in the folded sidebar: the folders of the only account, each the icon of what it is
 * for, and with several accounts the row they share, All inboxes, alone.
 *
 * An icon is told apart from the others by what it is for, and with several accounts every
 * folder's icon is there once for each of them with nothing to say whose it is. What was chosen
 * stays chosen and the list goes on showing it; a folder of one of several accounts is chosen with
 * the sidebar open.
 */
export function Rail({
  accounts,
  selected,
  onSelect,
  onUnfold,
  t,
}: {
  accounts: AccountNodeDto[];
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onUnfold: (() => void) | undefined;
  t: Translator;
}) {
  // A mailbox being read is a wait that ends by itself, and is not said here. One whose mail could
  // not be read is, by a mark naming it, whichever account it is.
  const unread = accounts.filter(
    (account) => account.folders.length === 0 && !account.answered,
  );
  const marks = unread.map((account) => (
    <div key={account.accountId} role="alert">
      <RailButton
        label={t("shell.mailNotRead")}
        hints={[account.email]}
        tone="alert"
        onClick={() => onUnfold?.()}
      >
        <CircleAlert />
      </RailButton>
    </div>
  ));

  const [only] = accounts;
  if (accounts.length !== 1 || only === undefined) {
    return (
      <>
        {accounts.length > 1 && (
          <Row
            label={t("shell.allInboxes")}
            badge={unreadBadge(
              accounts.reduce((sum, account) => sum + account.unread, 0),
            )}
            selected={selected?.kind === "everything"}
            onSelect={() => onSelect({ kind: "everything" })}
            t={t}
            folded
            icon={<Inbox />}
          />
        )}
        {marks}
      </>
    );
  }

  return (
    <>
      {marks}
      {only.folders.map((folder) => {
        const mailbox: Mailbox = {
          kind: "folder",
          accountId: only.accountId,
          folderId: folder.folderId,
          role: folder.role,
        };

        return (
          <Row
            key={folder.folderId}
            label={folderName(folder, t)}
            badge={folderBadge(folder)}
            selected={selected !== null && isSameMailbox(selected, mailbox)}
            onSelect={() => onSelect(mailbox)}
            t={t}
            folded
            icon={<FolderIcon role={folder.role} />}
          />
        );
      })}
    </>
  );
}
