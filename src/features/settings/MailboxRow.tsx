import { useId, useState } from "react";
import { Pencil, Signature } from "lucide-react";

import { inOrderOfNames } from "@/entities/group";
import { useI18n } from "@/shared/i18n";
import { setSignature } from "@/shared/ipc/client";
import type { AccountGroupDto } from "@/shared/ipc/generated/AccountGroupDto";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import { Chooser } from "@/shared/ui/Chooser";
import { Button } from "@/shared/ui/primitives/button";

import { SignatureField } from "./SignatureField";

/**
 * One mailbox in the settings: its name, the group it is in, its signature, and the way to how it
 * is reached.
 *
 * The signature opens under the row rather than standing there for every mailbox, so a hundred
 * mailboxes are still a list of a hundred lines.
 */
export function MailboxRow({
  account,
  groups,
  onChooseGroup,
  onOpen,
  onSignatureKept,
}: {
  account: MailAccountSummaryDto;
  /** The groups a mailbox can be put in, offered only once there is one. */
  groups: AccountGroupDto[];
  onChooseGroup: (group: string | null) => void;
  /** Opens the mailbox itself: how it is reached, and taking it off this device. */
  onOpen: () => void;
  /** Told the signature as the core kept it, so the composer signs with it from now on. */
  onSignatureKept: (accountId: string, signature: string) => void;
}) {
  const { t, language } = useI18n();
  const name = account.displayName.trim() || account.email;
  const fieldId = useId();
  /** Whether the signature is open under the row. */
  const [signing, setSigning] = useState(false);
  /** What was typed into the signature and could not be kept, or nothing. */
  const [unsaved, setUnsaved] = useState<string | null>(null);

  function keep(typed: string) {
    setSignature(account.id, typed).then(
      (kept) => {
        setUnsaved(null);
        onSignatureKept(account.id, kept);
      },
      () => {
        // Said under the row, with the field open again on what was typed, so nothing is lost
        // and the person can try again or give it up.
        setUnsaved(typed);
        setSigning(true);
      },
    );
  }

  return (
    <li className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-4">
        <span
          dir="auto"
          className="min-w-0 truncate text-sm"
          title={account.email}
        >
          {name}
        </span>
        <div className="flex shrink-0 items-center gap-2">
          {/* In the row of the mailbox it is about. Offered only once a group exists: a list whose
              one entry is "No group" is a control that cannot do anything. The same width in
              every row, so the rows line up. */}
          {groups.length > 0 && (
            <Chooser
              label={t("settings.groupOf", { account: name })}
              size="sm"
              className="w-40"
              value={
                groups.find((group) => group.accounts.includes(account.id))
                  ?.id ?? null
              }
              options={[
                { value: null, label: t("settings.noGroup") },
                // By name: a list to find a group in, unlike the sidebar's own order.
                ...inOrderOfNames(groups, language).map((group) => ({
                  value: group.id,
                  label: group.name,
                  dir: "auto" as const,
                })),
              ]}
              onChoose={onChooseGroup}
            />
          )}
          {/* Which mailbox, in the name and in the hint, as the pencil beside it says. */}
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("settings.signatureOf", { account: name })}
            title={t("settings.signatureOf", { account: name })}
            aria-expanded={signing}
            aria-controls={signing ? fieldId : undefined}
            onClick={() => {
              // The field is left before it goes, so what was typed is kept: on macOS a button
              // takes no focus when it is clicked, and a field taken away with the focus in it is
              // never left.
              const filling = document.activeElement;
              if (filling instanceof HTMLElement) filling.blur();
              setSigning((open) => !open);
            }}
          >
            <Signature />
          </Button>
          {/* Which mailbox, in the name and in the hint. A column of pencils all called "Edit" is
              a list a reader cannot choose in. */}
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("settings.editAccount", { account: name })}
            title={t("settings.editAccount", { account: name })}
            onClick={onOpen}
          >
            <Pencil />
          </Button>
        </div>
      </div>

      {signing && (
        <SignatureField
          id={fieldId}
          label={t("settings.signatureFor", { account: name })}
          hint={t("settings.signatureHint")}
          stored={account.signature ?? ""}
          unsaved={unsaved}
          onKeep={keep}
          onGiveUp={() => setUnsaved(null)}
        />
      )}
      {unsaved !== null && (
        <p role="alert" className="text-sm text-destructive">
          {t("settings.signatureNotSaved")}
        </p>
      )}
    </li>
  );
}
