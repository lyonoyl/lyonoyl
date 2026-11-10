import { unreadBadge } from "@/entities/folder";
import { isSameMailbox, type Mailbox } from "@/entities/mailbox";
import type { Translator } from "@/shared/i18n";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import type { SortingDto } from "@/shared/ipc/generated/SortingDto";
import { AccountBranch } from "./AccountBranch";
import type { AccountActions } from "./AccountMenu";
import { CARRYING_A_GROUP, fractionOf, landingMark } from "./dropping";
import { GroupMenu } from "./GroupMenu";
import { Folding, FOLDER, INDENT, Row } from "./Row";

/**
 * One group: a row that shows the mail of everything in it, and the mailboxes under it.
 *
 * The row is selectable as well as foldable, because the narrowing is the point — a person with
 * fifty mailboxes reads "Work" far more often than any one mailbox in it.
 */
export function GroupBranch({
  group,
  accounts,
  selected,
  onSelect,
  onFold,
  onToggleAccount,
  actions,
  t,
}: {
  group: { id: string; name: string; collapsed: boolean; sorting: SortingDto };
  accounts: AccountNodeDto[];
  selected: Mailbox | null;
  onSelect: (mailbox: Mailbox) => void;
  onFold: (groupId: string, collapsed: boolean) => void;
  onToggleAccount: (accountId: string, collapsed: boolean) => void;
  actions: AccountActions;
  t: Translator;
}) {
  const { drag } = actions;
  // Below an open group's row is its first mailbox, so a drop there goes into the group.
  const open = !group.collapsed && accounts.length > 0;
  const landing = drag.over?.id === group.id ? drag.over.where : null;

  return (
    <>
      {/* Picked up whole, with everything in it, and where a drop goes into the group or beside
          it. Which of those a drop does is the pointer's place on the row: its middle is the
          group's, its edges the places above and below it. */}
      <div
        draggable
        // Says this is the window moving something of its own, as on an account's row.
        data-drags=""
        aria-grabbed={drag.carried?.kind === "group" && drag.carried.id === group.id}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData(CARRYING_A_GROUP, group.id);
          drag.start({ kind: "group", id: group.id });
        }}
        onDragOver={(event) => {
          if (drag.carried === null) return;
          event.stopPropagation();
          const takes = drag.hover(
            { kind: "group", id: group.id, open },
            fractionOf(event),
          );
          if (!takes) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
        }}
        onDrop={(event) => {
          event.preventDefault();
          event.stopPropagation();
          drag.drop();
        }}
        onDragEnd={drag.end}
        className={landingMark(landing)}
      >
        <GroupMenu
          group={group}
          mailboxes={accounts.length}
          canMove={(by) => actions.canMove(group.id, by)}
          onMove={(by) => actions.move(group.id, by)}
          onGroupRenamed={actions.regrouped}
          onGroupSorted={actions.regrouped}
          onGroupRemoved={actions.regrouped}
        >
          <Folding
            depth={0}
            // Not the account's label: what a group hides is mailboxes, and "Hide folders" over a
            // list of mailboxes names the wrong thing.
            label={
              group.collapsed ? t("shell.showMailboxes") : t("shell.hideMailboxes")
            }
            expanded={!group.collapsed}
            onFold={() => onFold(group.id, !group.collapsed)}
          >
            <Row
              label={group.name}
              badge={unreadBadge(
                accounts.reduce((sum, account) => sum + account.unread, 0),
              )}
              selected={
                selected !== null &&
                isSameMailbox(selected, { kind: "group", groupId: group.id })
              }
              onSelect={() => onSelect({ kind: "group", groupId: group.id })}
              t={t}
            />
          </Folding>
        </GroupMenu>
      </div>

      {/* A group somebody made and has not filled yet. Said rather than drawn as a blank, which
          reads as a group whose mailboxes failed to load. */}
      {!group.collapsed && accounts.length === 0 && (
        <p
          className={`py-1.5 pe-2 text-xs text-muted-foreground ${INDENT[FOLDER]}`}
        >
          {t("shell.groupIsEmpty")}
        </p>
      )}

      {/* No wrapper, and no padding on one: an account in a group is drawn one level in by the
          row itself. A container that indented them took the width off the surface a person
          points at as well, so the thing that lit up under the pointer stopped short of the edge
          and was a different width from every row outside a group. */}
      {!group.collapsed &&
        accounts.map((account) => (
          <AccountBranch
            key={account.accountId}
            account={account}
            single={false}
            groupId={group.id}
            actions={actions}
            selected={selected}
            onSelect={onSelect}
            onToggle={onToggleAccount}
            t={t}
          />
        ))}
    </>
  );
}
