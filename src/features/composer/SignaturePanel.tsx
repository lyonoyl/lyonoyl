import { useId, useState } from "react";

import { useTranslation } from "@/shared/i18n";
import { setSignature } from "@/shared/ipc/client";
import { Panel } from "@/shared/ui/Panel";
import { Button } from "@/shared/ui/primitives/button";
import { SignatureText } from "@/shared/ui/SignatureText";
import { useOneAnswer } from "@/shared/ui/useOneAnswer";

/**
 * A mailbox's signature written or changed without leaving the message: kept for the mailbox, as
 * the settings keep it, and then put into the message being written.
 *
 * Over the message rather than in the settings, where the mail clients people know send somebody
 * to change it: the settings are another screen here, and going there would take the message away.
 */
export function SignaturePanel({
  account,
  name,
  signature,
  onKept,
  onClose,
}: {
  /** The mailbox the message is written from. */
  account: string;
  /** What the mailbox is called. */
  name: string;
  /** Its signature now, empty for none. */
  signature: string;
  /** Told the signature as the core kept it. */
  onKept: (signature: string) => void;
  onClose: () => void;
}) {
  const t = useTranslation();
  const hint = useId();
  const [typed, setTyped] = useState(signature);
  const { busy, refused, send } = useOneAnswer(onClose);
  const title = t("compose.signatureFor", { account: name });

  return (
    <Panel
      title={title}
      onCancel={onClose}
      onSubmit={() => {
        if (busy) return;
        if (typed === signature) {
          onClose();
          return;
        }
        send(() =>
          setSignature(account, typed).then((kept) => {
            onKept(kept);
            return true;
          }),
        );
      }}
    >
      <SignatureText
        aria-label={title}
        aria-describedby={hint}
        value={typed}
        rows={5}
        onChange={(event) => setTyped(event.target.value)}
      />
      <p id={hint} className="text-xs text-muted-foreground">
        {t("compose.signatureHint")}
      </p>
      {refused && (
        <p role="alert" className="text-sm text-destructive">
          {t("compose.signatureNotSaved")}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" disabled={busy}>
          {t("compose.saveSignature")}
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          {t("compose.signatureCancel")}
        </Button>
      </div>
    </Panel>
  );
}
