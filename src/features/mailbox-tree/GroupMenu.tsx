import { useRef, useState, type ReactNode } from "react";

import { useTranslation } from "@/shared/i18n";
import {
  removeAccountGroup,
  renameAccountGroup,
  sortAccountGroup,
} from "@/shared/ipc/client";
import type { SortingDto } from "@/shared/ipc/generated/SortingDto";
import { RemoveGroup } from "@/shared/ui/RemoveGroup";
import { RenameGroup } from "@/shared/ui/RenameGroup";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/shared/ui/primitives/context-menu";

/**
 * What a group's row offers on a right click: a new name, the order its mailboxes are drawn in,
 * and removing it once its name is typed. The mailboxes in a removed group stay, moved out to the
 * top of the sidebar.
 */
export function GroupMenu({
  group,
  mailboxes,
  canMove,
  onMove,
  onGroupRenamed,
  onGroupSorted,
  onGroupRemoved,
  children,
}: {
  group: { id: string; name: string; sorting: SortingDto };
  /** How many mailboxes the row draws in the group. */
  mailboxes: number;
  /** Whether there is a place that way at the top of the sidebar, which greys the entry out. */
  canMove: (by: -1 | 1) => boolean;
  /** Moves the group one place up or down at the top of the sidebar, with everything in it. */
  onMove: (by: -1 | 1) => void;
  /** Called once the core has renamed the group, so the sidebar reads its arrangement again. */
  onGroupRenamed: () => void;
  /** Called once the core keeps the group's new order, so the sidebar draws it. */
  onGroupSorted: () => void;
  /** Called once the core has removed the group, so the sidebar reads its arrangement again. */
  onGroupRemoved: () => void;
  /** The group's row, which is what is right-clicked. */
  children: ReactNode;
}) {
  const t = useTranslation();
  const [renamingGroup, setRenamingGroup] = useState(false);
  const [removingGroup, setRemovingGroup] = useState(false);
  /**
   * Which panel was chosen, opened once the menu has closed and given the focus back: opened at
   * the press, the panel's field had the focus taken from it by the menu on its way out.
   */
  const chosen = useRef<"rename" | "remove" | null>(null);

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger>{children}</ContextMenuTrigger>
        <ContextMenuContent
          aria-label={t("group.menuFor", { group: group.name })}
          onCloseAutoFocus={() => {
            const panel = chosen.current;
            chosen.current = null;
            if (panel === "rename") setRenamingGroup(true);
            if (panel === "remove") setRemovingGroup(true);
          }}
        >
          <ContextMenuItem
            onSelect={() => {
              chosen.current = "rename";
            }}
          >
            {t("group.menuRename")}
          </ContextMenuItem>

          <ContextMenuSub>
            <ContextMenuSubTrigger className="rtl:[&>svg:last-child]:ml-0 rtl:[&>svg:last-child]:mr-auto rtl:[&>svg:last-child]:-scale-x-100">
              {t("group.menuSort")}
            </ContextMenuSubTrigger>
            <ContextMenuSubContent>
              {/* By hand ticks neither. It is the order a person dragged the mailboxes into, which
                  is said by dragging them rather than chosen here. */}
              <ContextMenuRadioGroup
                value={group.sorting}
                onValueChange={(value) => {
                  const sorting = SORTINGS.find((one) => one === value);
                  if (sorting === undefined || sorting === group.sorting) return;
                  void sortAccountGroup(group.id, sorting)
                    .then(onGroupSorted)
                    .catch(() => {
                      // The menu keeps ticking the order the group is actually kept in.
                    });
                }}
              >
                <ContextMenuRadioItem value="dateAdded">
                  {t("group.menuSortByDateAdded")}
                </ContextMenuRadioItem>
                <ContextMenuRadioItem value="name">
                  {t("group.menuSortByName")}
                </ContextMenuRadioItem>
              </ContextMenuRadioGroup>
            </ContextMenuSubContent>
          </ContextMenuSub>

          <ContextMenuSeparator />

          {/* The same move a drag makes. Here as well because a drag is a thing only a mouse can
              do, and the order of somebody's sidebar is not a mouse-only setting. */}
          <ContextMenuItem disabled={!canMove(-1)} onSelect={() => onMove(-1)}>
            {t("group.moveUp")}
          </ContextMenuItem>
          <ContextMenuItem disabled={!canMove(1)} onSelect={() => onMove(1)}>
            {t("group.moveDown")}
          </ContextMenuItem>

          <ContextMenuSeparator />
          <ContextMenuItem
            variant="destructive"
            onSelect={() => {
              chosen.current = "remove";
            }}
          >
            {t("group.menuRemove")}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

      {renamingGroup && (
        <RenameGroup
          name={group.name}
          onRename={(name) =>
            renameAccountGroup(group.id, name).then(
              () => {
                onGroupRenamed();
                return true;
              },
              () => false,
            )
          }
          onClose={() => setRenamingGroup(false)}
        />
      )}

      {removingGroup && (
        <RemoveGroup
          name={group.name}
          mailboxes={mailboxes}
          onRemove={() =>
            removeAccountGroup(group.id).then(
              () => {
                onGroupRemoved();
                return true;
              },
              () => false,
            )
          }
          onClose={() => setRemovingGroup(false)}
        />
      )}
    </>
  );
}

/** The orders the menu offers. By hand is not one of them: it is what dragging says. */
const SORTINGS: SortingDto[] = ["dateAdded", "name"];
