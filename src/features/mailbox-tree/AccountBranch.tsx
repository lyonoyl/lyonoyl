import { folderBadge, folderName, unreadBadge } from "@/entities/folder";
import { isSameMailbox, type Mailbox } from "@/entities/mailbox";
import type { Translator } from "@/shared/i18n";
import {
  ContextMenu,
  ContextMenuTrigger,
} from "@/shared/ui/primitives/context-menu";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import { AccountMenu, type AccountActions } from "./AccountMenu";
import { CARRYING_AN_ACCOUNT, fractionOf, landingMark } from "./dropping";
import { Folding, FOLDER, INDENT, Row } from "./Row";

/** One account: its own row, which a drag picks up and drops on, and its folders under it. */
export function AccountBranch({
  account,
  single,
  groupId = null,
  actions,
  selected,
  onSelect,
  onToggle,
  t,
}: {
  account: AccountNodeDto;
  single: boolean;
  /** The group it is drawn in, so its own row sits one level in. Its folders do not: see `INDENT`. */
  groupId?: string | null;
  actions: AccountActions;
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onToggle: (accountId: string, collapsed: boolean) => void;
  t: Translator;
}) {
  const open = single || !account.collapsed;
  const grouped = groupId !== null;
  const { drag } = actions;
  const landing =
    drag.over?.id === account.accountId ? drag.over.where : null;

  return (
    <>
      {!single && (
        <ContextMenu>
          {/* The whole row is what is picked up, and what a row is dropped on. Not the button
              inside it: a drag started on a control is a drag a person did not mean, and every
              press would have to be told apart from a hold. */}
          <ContextMenuTrigger
            draggable
            // Says this is the window moving something of its own, not the browser carrying it
            // out. Without it the drag is cancelled at the document, one level above every
            // handler below, and the row cannot be picked up at all.
            data-drags=""
            aria-grabbed={
              drag.carried?.kind === "account" &&
              drag.carried.id === account.accountId
            }
            onDragStart={(event) => {
              // Something has to be carried or the drop never fires. Under a type only this
              // window knows, so a row dropped on another application carries nothing it can
              // read: as plain text it was an identifier out of somebody's mailbox, handed to
              // whatever was under the pointer.
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData(CARRYING_AN_ACCOUNT, account.accountId);
              drag.start({ kind: "account", id: account.accountId });
            }}
            onDragOver={(event) => {
              if (drag.carried === null) return;
              // Held here, so the sidebar does not take it for a drag over nothing.
              event.stopPropagation();
              const takes = drag.hover(
                { kind: "account", id: account.accountId, groupId },
                fractionOf(event),
              );
              if (!takes) return;
              // Refusing the default is what makes a drop possible at all.
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
            }}
            onDrop={(event) => {
              event.preventDefault();
              event.stopPropagation();
              drag.drop();
            }}
            onDragEnd={drag.end}
            // A line where the row would land, and nothing else moves until it does. Rows sliding
            // out of the way under the pointer is a list that answers before the person has.
            className={`block ${landingMark(landing)}`}
          >
            <Folding
              depth={grouped ? 1 : 0}
              label={t(
                account.collapsed ? "shell.expandNamed" : "shell.collapseNamed",
                { account: account.displayName.trim() || account.email },
              )}
              expanded={!account.collapsed}
              onFold={() => onToggle(account.accountId, !account.collapsed)}
            >
              <Row
                depth={grouped ? 1 : 0}
                label={account.displayName.trim() || account.email}
                title={account.email}
                badge={unreadBadge(account.unread)}
                selected={
                  selected !== null &&
                  isSameMailbox(selected, {
                    kind: "account",
                    accountId: account.accountId,
                  })
                }
                onSelect={() =>
                  onSelect({ kind: "account", accountId: account.accountId })
                }
                t={t}
              />
            </Folding>
          </ContextMenuTrigger>
          <AccountMenu account={account} actions={actions} t={t} />
        </ContextMenu>
      )}

      {/* An account whose folders have not arrived is an account being read, not an account with
          nothing in it. Left blank it reads as broken — which is what it looked like for the
          minute a first pass over a hosted server takes.

          And an account whose mail could not be read at all is neither: saying "reading folders"
          about it would be a message that never comes true. */}
      {open && account.folders.length === 0 && (
        <p
          role={account.answered ? undefined : "alert"}
          className={`py-1.5 pe-2 text-xs ${INDENT[single ? 0 : FOLDER]} ${
            account.answered ? "text-muted-foreground" : "text-destructive"
          }`}
        >
          {t(account.answered ? "shell.readingFolders" : "shell.mailNotRead")}
        </p>
      )}

      {open &&
        account.folders.map((folder) => (
          <Row
            key={folder.folderId}
            label={folderName(folder, t)}
            badge={folderBadge(folder)}
            depth={single ? 0 : FOLDER}
            selected={
              selected !== null &&
              isSameMailbox(selected, {
                kind: "folder",
                accountId: account.accountId,
                folderId: folder.folderId,
                role: folder.role,
              })
            }
            onSelect={() =>
              onSelect({
                kind: "folder",
                accountId: account.accountId,
                folderId: folder.folderId,
                role: folder.role,
              })
            }
            t={t}
          />
        ))}
    </>
  );
}
