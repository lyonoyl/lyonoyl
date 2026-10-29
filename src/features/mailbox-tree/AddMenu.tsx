import { Plus } from "lucide-react";
import { useRef, useState } from "react";

import { useTranslation } from "@/shared/i18n";
import { createAccountGroup } from "@/shared/ipc/client";
import { AddGroup } from "@/shared/ui/AddGroup";
import { Button } from "@/shared/ui/primitives/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/primitives/dropdown-menu";

/**
 * The plus in the sidebar's heading: an account to add, or a group to keep accounts in. The two
 * are offered alike, Add account beside Add group.
 *
 * A group is named in a panel over the window rather than in the sidebar: the sidebar draws a group
 * only once it exists, so the first one has nowhere in it to be named.
 */
export function AddMenu({
  onAddAccount,
  onGroupAdded,
}: {
  onAddAccount: () => void;
  /** Called once the core holds the new group, so the sidebar reads its arrangement again. */
  onGroupAdded: () => void;
}) {
  const t = useTranslation();
  const [addingGroup, setAddingGroup] = useState(false);
  /**
   * What was chosen, done once the menu has closed rather than at the press. The menu holds the
   * focus until it is gone: the account screen or the name field took it at the press, and the
   * menu took it straight back and dropped it on nothing as it closed.
   */
  const chosen = useRef<"account" | "group" | null>(null);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("shell.add")}
            title={t("shell.add")}
          >
            <Plus />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          // As wide as its longest line rather than as the plus, at whose width no line of it has
          // room and every one breaks.
          className="w-auto max-w-(--radix-dropdown-menu-content-available-width)"
          // The menu gives the focus back to the plus as it closes, and what opens next takes it
          // from there — and gives it back to the plus when it goes.
          onCloseAutoFocus={() => {
            const choice = chosen.current;
            chosen.current = null;
            if (choice === "account") onAddAccount();
            if (choice === "group") setAddingGroup(true);
          }}
        >
          <DropdownMenuItem
            onSelect={() => {
              chosen.current = "account";
            }}
          >
            {t("shell.addAccount")}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              chosen.current = "group";
            }}
          >
            {t("shell.addGroup")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      {addingGroup && (
        <AddGroup
          onAdd={(name) =>
            createAccountGroup(name).then(
              () => {
                onGroupAdded();
                return true;
              },
              () => false,
            )
          }
          onClose={() => setAddingGroup(false)}
        />
      )}
    </>
  );
}
