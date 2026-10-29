import { useState } from "react";

import { useTranslation } from "@/shared/i18n";
import { GroupNameField } from "@/shared/ui/GroupNameField";
import { GroupNotSaved } from "@/shared/ui/GroupNotSaved";
import { GroupPanel } from "@/shared/ui/GroupPanel";
import { useOneAnswer } from "@/shared/ui/useOneAnswer";
import { Button } from "@/shared/ui/primitives/button";

/** Adding a group: a name, and nothing asked of the core while it is blank. */
export function AddGroup({
  onAdd,
  onClose,
}: {
  /** Asks for the group, and answers whether it now exists. */
  onAdd: (name: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const t = useTranslation();
  const [typed, setTyped] = useState("");
  const { busy, refused, send } = useOneAnswer(onClose);
  const ready = typed.trim() !== "" && !busy;

  return (
    <GroupPanel
      title={t("group.addTitle")}
      onCancel={onClose}
      onSubmit={() => {
        if (ready) send(() => onAdd(typed));
      }}
    >
      <GroupNameField
        label={t("group.addName")}
        value={typed}
        onChange={setTyped}
        placeholder={t("group.addPlaceholder")}
      />
      {refused && <GroupNotSaved>{t("group.addNotSaved")}</GroupNotSaved>}
      <div className="flex gap-2">
        <Button type="submit" disabled={!ready}>
          {t("group.add")}
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          {t("group.addCancel")}
        </Button>
      </div>
    </GroupPanel>
  );
}
