import { useRef, useState, type ReactNode } from "react";

import { useTranslation } from "@/shared/i18n";
import { removeAccountGroup } from "@/shared/ipc/client";
import { RemoveGroup } from "@/shared/ui/RemoveGroup";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/primitives/context-menu";

/**
 * What a group's row offers on a right click: removing the group, once its name is typed. The
 * mailboxes in it stay, moved out to the top of the sidebar.
 */
export function GroupMenu({
  group,
  mailboxes,
  onGroupRemoved,
  children,
}: {
  group: { id: string; name: string };
  /** How many mailboxes the row draws in the group. */
  mailboxes: number;
  /** Called once the core has removed the group, so the sidebar reads its arrangement again. */
  onGroupRemoved: () => void;
  /** The group's row, which is what is right-clicked. */
  children: ReactNode;
}) {
  const t = useTranslation();
  const [removingGroup, setRemovingGroup] = useState(false);
  /**
   * Whether Remove was chosen, acted on once the menu has closed and given the focus back: opened
   * at the press, the panel's field had the focus taken from it by the menu on its way out.
   */
  const chosen = useRef(false);

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger>{children}</ContextMenuTrigger>
        <ContextMenuContent
          aria-label={t("group.menuFor", { group: group.name })}
          onCloseAutoFocus={() => {
            if (!chosen.current) return;
            chosen.current = false;
            setRemovingGroup(true);
          }}
        >
          <ContextMenuItem
            variant="destructive"
            onSelect={() => {
              chosen.current = true;
            }}
          >
            {t("group.menuRemove")}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>

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
