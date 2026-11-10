import { useState } from "react";

import { useTranslation } from "@/shared/i18n";
import { GroupNameField } from "@/shared/ui/GroupNameField";
import { GroupNotSaved } from "@/shared/ui/GroupNotSaved";
import { GroupPanel } from "@/shared/ui/GroupPanel";
import { useOneAnswer } from "@/shared/ui/useOneAnswer";
import { Button } from "@/shared/ui/primitives/button";

/**
 * Renaming a group: its name in the field, to be typed over, and nothing asked of the core while
 * the field is blank. A name left as it was closes the panel, since there is nothing to keep.
 */
export function RenameGroup({
  name,
  onRename,
  onClose,
}: {
  /** The name the group has now. */
  name: string;
  /** Asks for the new name, and answers whether the group now carries it. */
  onRename: (name: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const t = useTranslation();
  const [typed, setTyped] = useState(name);
  const { busy, refused, send } = useOneAnswer(onClose);
  const ready = typed.trim() !== "" && !busy;

  return (
    <GroupPanel
      title={t("group.renameTitle")}
      onCancel={onClose}
      onSubmit={() => {
        if (!ready) return;
        if (typed.trim() === name) {
          onClose();
          return;
        }
        send(() => onRename(typed));
      }}
    >
      <GroupNameField
        label={t("group.renameName")}
        value={typed}
        onChange={setTyped}
      />
      {refused && <GroupNotSaved>{t("group.renameNotSaved")}</GroupNotSaved>}
      <div className="flex gap-2">
        <Button type="submit" disabled={!ready}>
          {t("group.rename")}
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          {t("group.renameCancel")}
        </Button>
      </div>
    </GroupPanel>
  );
}
