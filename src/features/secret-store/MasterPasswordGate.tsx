import { useState, type FormEvent } from "react";

import { describeError, useI18n } from "@/shared/i18n";
import {
  chooseMasterPassword,
  isCommandError,
  unlockSecrets,
} from "@/shared/ipc/client";
import { Button } from "@/shared/ui/primitives/button";
import { Input } from "@/shared/ui/primitives/input";
import { Label } from "@/shared/ui/primitives/label";

/**
 * The shortest master password this build accepts.
 *
 * It is the only thing between a stolen disk and every mailbox password on it, and unlike a
 * mailbox password there is no server to rate-limit guesses — the file can be attacked as fast as
 * the attacker's hardware allows.
 */
const LEAST = 12;

interface Props {
  /** Whether there is a file yet: choosing makes one, unlocking opens the one that is there. */
  mode: "choose" | "unlock";
  onOpened: () => void;
  onLater: () => void;
}

/**
 * The explanation and the master password, for a Linux desktop whose first start found no
 * credential store.
 *
 * Shown rather than assumed: a password that quietly went into a file instead of the system store
 * is the confusion this exists to prevent. It can be put off, and the screen behind it offers it
 * again; nothing is written until the password is set.
 */
export function MasterPasswordGate({ mode, onOpened, onLater }: Props) {
  const { t } = useI18n();
  const [password, setPassword] = useState("");
  const [repeated, setRepeated] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const choosing = mode === "choose";
  const tooShort = choosing && password.length > 0 && password.length < LEAST;
  const mismatch = choosing && repeated.length > 0 && password !== repeated;
  const ready = choosing
    ? password.length >= LEAST && password === repeated
    : password.length > 0;

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!ready || busy) return;

    setBusy(true);
    setFailure(null);
    try {
      await (choosing
        ? chooseMasterPassword(password)
        : unlockSecrets(password));
      // Held no longer than it takes to hand it over.
      setPassword("");
      setRepeated("");
      onOpened();
    } catch (reason: unknown) {
      setFailure(
        isCommandError(reason)
          ? describeError(reason, t)
          : t("error.bridgeUnavailable"),
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-6"
      // Escape leaves, the same key on all three systems, and it does what the "later" button
      // does rather than something of its own. A panel covering the whole window with no way out
      // but one button is a window somebody can be stuck in — and this one appears unasked, on
      // the way to mail they were already reading.
      //
      // Not while it is working: leaving mid-submit would drop a password that is on its way to
      // the credential store, and say nothing about what became of it.
      onKeyDown={(event) => {
        if (event.key !== "Escape" || busy) return;
        event.stopPropagation();
        onLater();
      }}
    >
      <form
        onSubmit={submit}
        role="dialog"
        aria-modal="true"
        aria-labelledby="master-password-title"
        className="flex w-full max-w-lg flex-col gap-4 rounded-lg border border-border bg-background p-6 shadow-lg"
      >
        <h1
          id="master-password-title"
          className="text-xl font-semibold tracking-tight"
        >
          {t(choosing ? "secrets.chooseTitle" : "secrets.unlockTitle")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t(choosing ? "secrets.chooseBody" : "secrets.unlockBody")}
        </p>
        {choosing && (
          <p className="text-sm text-muted-foreground">
            {t("secrets.warning")}
          </p>
        )}

        <div className="flex flex-col gap-2">
          <Label htmlFor="master-password">{t("secrets.password")}</Label>
          <Input
            id="master-password"
            type="password"
            autoFocus
            autoComplete={choosing ? "new-password" : "current-password"}
            value={password}
            disabled={busy}
            onChange={(event) => setPassword(event.target.value)}
          />
          {tooShort && (
            <p className="text-sm text-muted-foreground">
              {t("secrets.tooShort", { count: LEAST })}
            </p>
          )}
        </div>

        {choosing && (
          <div className="flex flex-col gap-2">
            <Label htmlFor="master-password-again">{t("secrets.repeat")}</Label>
            <Input
              id="master-password-again"
              type="password"
              autoComplete="new-password"
              value={repeated}
              disabled={busy}
              onChange={(event) => setRepeated(event.target.value)}
            />
            {mismatch && (
              <p className="text-sm text-muted-foreground">
                {t("secrets.mismatch")}
              </p>
            )}
          </div>
        )}

        {failure && (
          <p role="alert" className="text-sm text-destructive">
            {failure}
          </p>
        )}

        <p className="text-sm text-muted-foreground">{t("secrets.pending")}</p>

        <div className="flex gap-2">
          <Button type="submit" disabled={!ready || busy}>
            {busy
              ? t("secrets.working")
              : t(choosing ? "secrets.choose" : "secrets.unlock")}
          </Button>
          <Button
            type="button"
            variant="ghost"
            disabled={busy}
            onClick={onLater}
          >
            {t("secrets.later")}
          </Button>
        </div>
      </form>
    </div>
  );
}
