import type { AccountGroupDto } from "@/shared/ipc/generated/AccountGroupDto";
import type { AccountNodeDto } from "@/shared/ipc/generated/AccountNodeDto";
import { placeAccountInGroup } from "@/shared/ipc/client";
import type { Translator } from "@/shared/i18n";
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/shared/ui/primitives/context-menu";

/** A drag in progress: which account is being carried, and where it is hovering. */
type Dragging = {
  carried: string | null;
  /** The row under the pointer: an account, or a group at the top of the sidebar. */
  over: string | null;
  start: (accountId: string) => void;
  hover: (target: string) => void;
  drop: (target: string) => void;
  /** Whether the row being carried would move if it were dropped here. */
  lands: (target: string) => boolean;
  end: () => void;
};

/**
 * Moves one account inside the run it is drawn in, and answers with the run's new order.
 *
 * `null` where there is nowhere to go, which is the first account asked to go up and the last
 * asked to go down. Plain arithmetic over a list rather than anything about the sidebar: it is the
 * same answer for a drag and for a menu entry, and two of it would be a drag that puts a row
 * somewhere the keyboard cannot.
 */
export function movedWithin(
  run: string[],
  accountId: string,
  to: number,
): string[] | null {
  const from = run.indexOf(accountId);
  if (from < 0 || to < 0 || to >= run.length || to === from) return null;

  const ordered = [...run];
  const [carried] = ordered.splice(from, 1);
  ordered.splice(to, 0, carried);
  return ordered;
}

/**
 * What can be done to an account, on the row that names it.
 *
 * On the second button rather than the first: the first selects the account, and a menu that took
 * that gesture would leave no way to open the mailbox it is a menu for. Everything here already
 * exists somewhere else in the window — this is the short way to it from the row itself, and
 * nothing in it does work of its own.
 *
 * Removing an account is not here. It is one press away, inside the account's own screen, where
 * the sentence explaining what goes with it is already written; a second way to do it would be a
 * second place for that sentence to fall out of step with what actually happens.
 */
export type AccountActions = {
  groups: AccountGroupDto[];
  /**
   * Puts this account one place earlier or later among the ones drawn beside it.
   *
   * Answers whether there was anywhere to go: the first account has no place above it, and a
   * menu entry that does nothing is worse than one that is not offered.
   */
  move: (accountId: string, by: -1 | 1) => void;
  /** Whether there is anywhere to go that way, which is what greys the entry out. */
  canMove: (accountId: string, by: -1 | 1) => boolean;
  /** Everything a drag needs, held in the tree because a run is what a drag moves within. */
  drag: Dragging;
  /** Read this one account now. */
  sync: (account: AccountNodeDto) => void;
  /** Open the account's own screen, which is also where it is removed. */
  settings: (accountId: string) => void;
  /** Its group changed, so the sidebar reads itself again. */
  regrouped: () => void;
};

export function AccountMenu({
  account,
  actions,
  t,
}: {
  account: AccountNodeDto;
  actions: AccountActions;
  t: Translator;
}) {
  const { groups } = actions;
  const email = account.email;
  const holding =
    groups.find((group) => group.accounts.includes(account.accountId))?.id ??
    NO_GROUP;

  return (
    <ContextMenuContent aria-label={t("account.menuFor", { email })}>
      <ContextMenuItem onSelect={() => actions.sync(account)}>
        {t("account.getMail")}
      </ContextMenuItem>

      {/* Offered only where there is somewhere to move to. A submenu whose one entry is "not in a
          group" is a control that cannot change anything. */}
      {groups.length > 0 && (
        <ContextMenuSub>
          <ContextMenuSubTrigger className="rtl:[&>svg:last-child]:ml-0 rtl:[&>svg:last-child]:mr-auto rtl:[&>svg:last-child]:-scale-x-100">
            {t("account.moveToGroup")}
          </ContextMenuSubTrigger>
          <ContextMenuSubContent>
            <ContextMenuRadioGroup
              value={holding}
              onValueChange={(chosen) => {
                if (chosen === holding) return;
                void placeAccountInGroup(
                  account.accountId,
                  chosen === NO_GROUP ? null : chosen,
                )
                  .then(actions.regrouped)
                  .catch(() => {
                    // The sidebar keeps showing where the account actually is. Saying it moved
                    // when it did not is the one answer worse than not moving it.
                  });
              }}
            >
              <ContextMenuRadioItem value={NO_GROUP}>
                {t("account.noGroup")}
              </ContextMenuRadioItem>
              <ContextMenuSeparator />
              {groups.map((group) => (
                <ContextMenuRadioItem key={group.id} value={group.id}>
                  <span dir="auto">{group.name}</span>
                </ContextMenuRadioItem>
              ))}
            </ContextMenuRadioGroup>
          </ContextMenuSubContent>
        </ContextMenuSub>
      )}

      <ContextMenuSeparator />

      {/* The same move a drag makes. Here as well because a drag is a thing only a mouse can do,
          and the order of somebody's mailboxes is not a mouse-only setting. */}
      <ContextMenuItem
        disabled={!actions.canMove(account.accountId, -1)}
        onSelect={() => actions.move(account.accountId, -1)}
      >
        {t("account.moveUp")}
      </ContextMenuItem>
      <ContextMenuItem
        disabled={!actions.canMove(account.accountId, 1)}
        onSelect={() => actions.move(account.accountId, 1)}
      >
        {t("account.moveDown")}
      </ContextMenuItem>

      <ContextMenuSeparator />
      <ContextMenuLabel className="sr-only">{email}</ContextMenuLabel>
      <ContextMenuItem onSelect={() => actions.settings(account.accountId)}>
        {t("account.openSettings")}
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

/** Stands for "no group" in the radio group, where a value has to be a string. */
const NO_GROUP = "\u0000none";
