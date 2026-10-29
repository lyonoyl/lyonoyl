import { useState } from "react";

import { useTranslation } from "@/shared/i18n";
import { GroupNameField } from "@/shared/ui/GroupNameField";
import { GroupNotSaved } from "@/shared/ui/GroupNotSaved";
import { GroupPanel } from "@/shared/ui/GroupPanel";
import { useOneAnswer } from "@/shared/ui/useOneAnswer";
import { Button } from "@/shared/ui/primitives/button";

/**
 * Removing a group, once its name is typed.
 *
 * The mailboxes in it are not removed with it: they move out of it, to the top of the sidebar, and
 * the panel says so only when there are some. The name is asked for because the press is the whole
 * of the decision, and a group is something a person arranged by hand.
 *
 * It asks the core through what its caller gives it and is told whether the core did it, so one
 * panel serves the sidebar, which reads its list again itself, and the settings, which are told.
 */
export function RemoveGroup({
  name,
  mailboxes,
  onRemove,
  onClose,
}: {
  name: string;
  /** How many mailboxes are in the group, which decides whether there is anything to say about them. */
  mailboxes: number;
  /** Asks for the group to go, and answers whether it has. */
  onRemove: () => Promise<boolean>;
  onClose: () => void;
}) {
  const t = useTranslation();
  const [typed, setTyped] = useState("");
  const { busy, refused, send } = useOneAnswer(onClose);
  const ready = sameName(typed, name) && !busy;

  return (
    <GroupPanel
      // The name is not in the title: it stands below, on its own, where it is typed from.
      title={t("group.removeTitle")}
      onCancel={onClose}
      onSubmit={() => {
        if (ready) send(onRemove);
      }}
    >
      {mailboxes > 0 && (
        <p className="text-sm text-muted-foreground">
          {t("group.removeMovesMailboxes")}
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        {t("group.removeTypeName")}
      </p>
      {/* The name to type, set apart from the sentence asking for it so every letter of it can be
          seen — a space at its end included. It is the person's own word, not a sentence of ours,
          and runs the way its own letters do. */}
      <p
        dir="auto"
        className="max-w-full self-start rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-xs break-all whitespace-pre-wrap"
      >
        {name}
      </p>
      <GroupNameField
        label={t("group.removeName")}
        value={typed}
        onChange={setTyped}
      />
      {refused && <GroupNotSaved>{t("group.removeNotSaved")}</GroupNotSaved>}
      <div className="flex gap-2">
        <Button type="submit" variant="destructive" disabled={!ready}>
          {t("group.remove")}
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          {t("group.removeCancel")}
        </Button>
      </div>
    </GroupPanel>
  );
}

/**
 * The name as it is shown, setting aside only the spaces around it and how its letters were
 * composed. A near miss in any letter is a different group as far as the press knows.
 */
function sameName(typed: string, name: string): boolean {
  return typed.trim().normalize("NFC") === name.trim().normalize("NFC");
}
