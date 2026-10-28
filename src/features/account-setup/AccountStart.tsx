import { Eye, Loader2 } from "lucide-react";
import { useState, type FormEvent } from "react";

import { accountHint } from "@/shared/ipc/client";
import { keyTheCoreSent, useI18n, type TranslationKey } from "@/shared/i18n";
import { secretStoreName } from "@/shared/platform";
import { Button } from "@/shared/ui/primitives/button";
import { Input } from "@/shared/ui/primitives/input";
import { typedDirection } from "@/shared/ui/direction";

import { Field } from "./Field";
import type { Draft } from "./useAccountSetup";

/** What the screen is doing, when it is doing something that takes a moment. */
export type Working = null | "looking" | "adding";

/**
 * The first screen of adding an account: the three things a person already knows.
 *
 * Everything else about a mailbox — the servers, the ports, how it is encrypted — is either
 * something this build already knows or something the domain publishes, and both are looked up
 * from the address behind this button. A person who has to answer twelve questions to read their
 * mail is a person who closes the window at the fourth; the twelve are still there, on the screen
 * behind this one, for the mailbox nobody can answer for.
 */
export function AccountStart({
  secretsInAFile,
  draft,
  onEdit,
  onContinue,
  working,
}: {
  /** Whether a password goes to a file a master password opens rather than the system's store. */
  secretsInAFile: boolean;
  draft: Draft;
  onEdit: (field: keyof Draft, value: string) => void;
  /** Look the address up and, if anybody knows it, connect. */
  onContinue: () => void;
  working: Working;
}) {
  const { t } = useI18n();
  const whereItGoes = secretsInAFile
    ? t("account.passwordHintFile")
    : t("account.passwordHint", { store: t(secretStoreName()) });
  const [showing, setShowing] = useState(false);
  /** A catalogue key about this address's provider, or nothing to say. */
  const [aboutProvider, setAboutProvider] = useState<TranslationKey | null>(null);

  /**
   * Asks what this address's provider wants, and forgets the answer if the address changes again.
   *
   * A question that could not be asked says nothing rather than something: a warning nobody could
   * fetch is not an error a person can act on, and the field below still works without it.
   */
  const tellAbout = async (email: string) => {
    try {
      setAboutProvider(keyTheCoreSent(await accountHint(email)));
    } catch {
      setAboutProvider(null);
    }
  };

  const ready = draft.email.trim() !== "" && draft.password !== "";

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (working !== null) return;
    onContinue();
  };

  return (
    <form
      className="m-auto flex w-full max-w-xl flex-col gap-6 p-8"
      onSubmit={submit}
      aria-labelledby="account-start-title"
    >
      <header className="flex flex-col gap-2">
        <h1
          id="account-start-title"
          className="text-xl font-semibold tracking-tight"
        >
          {t("account.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {secretsInAFile
            ? t("account.introFile")
            : t("account.intro", { store: t(secretStoreName()) })}
        </p>
      </header>

      <fieldset className="flex flex-col gap-4" disabled={working !== null}>
        <legend className="sr-only">{t("account.identity")}</legend>

        <Field
          id="start-display-name"
          label={t("account.displayName")}
          hint={t("account.displayNameHint")}
        >
          <Input
            id="start-display-name"
            dir={typedDirection(draft.displayName)}
            aria-describedby="start-display-name-hint"
            value={draft.displayName}
            onChange={(event) => onEdit("displayName", event.target.value)}
          />
        </Field>

        <Field id="start-email" label={t("account.email")} required>
          <Input
            id="start-email"
            dir="ltr"
            required
            type="email"
            autoComplete="email"
            spellCheck={false}
            value={draft.email}
            onChange={(event) => onEdit("email", event.target.value)}
            // Asked when the address is finished rather than on every keystroke, and answered from
            // a table inside the application without asking anybody anything. The moment matters:
            // the password field is the next one down, and a warning about which password to type
            // is worth nothing after it has been typed.
            onBlur={(event) => void tellAbout(event.target.value)}
          />
        </Field>

        <Field
          id="start-password"
          label={t("account.password")}
          hint={
            aboutProvider === null ? (
              whereItGoes
            ) : (
              // Two sentences, each on its line: run together with a space they read as one,
              // since no line in the catalogue ends with a stop.
              <>
                <span className="block">{t(aboutProvider)}</span>
                <span className="block">{whereItGoes}</span>
              </>
            )
          }
          required
        >
          {/* The eye sits in the field rather than beside it: outside, it made the field
              narrower than the two above it for the sake of a button that is useless most of the
              time. It appears once there is something to show -- an eye over an empty field
              offers to reveal nothing. Field and eye run left to right together, so the room the
              field keeps for the eye is on the side the eye stands. */}
          <div dir="ltr" className="relative">
            <Input
              id="start-password"
              dir="ltr"
              required
              className={draft.password === "" ? undefined : "pe-9"}
              // Hidden by default and shown only while the eye is held: a password left in the
              // clear is a password read by whoever is standing behind this person, and a switch
              // that stays on is a password left in the clear.
              type={showing ? "text" : "password"}
              autoComplete="current-password"
              aria-describedby="start-password-hint"
              value={draft.password}
              onChange={(event) => onEdit("password", event.target.value)}
            />
            {draft.password !== "" && (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="absolute inset-y-0 end-1 my-auto"
                aria-label={t("account.showPassword")}
                title={t("account.showPassword")}
                // Held, not toggled -- and let go of on every way a press can end, including the
                // pointer leaving the button and the window losing the focus while it is down.
                // A press that ended somewhere this did not hear about would leave the password on
                // screen with nothing to take it off.
                onPointerDown={() => setShowing(true)}
                onPointerUp={() => setShowing(false)}
                onPointerLeave={() => setShowing(false)}
                onPointerCancel={() => setShowing(false)}
                onKeyDown={(event) => {
                  if (event.key === " " || event.key === "Enter") {
                    setShowing(true);
                  }
                }}
                onKeyUp={() => setShowing(false)}
                onBlur={() => setShowing(false)}
              >
                <Eye />
              </Button>
            )}
          </div>
        </Field>
      </fieldset>

      <div className="flex items-center gap-3">
        {/* The word on it does not change. A button that renames itself while it works is a
            button a person has to read again to find, and what it is doing is said beside it. */}
        <Button type="submit" disabled={!ready || working !== null}>
          {t("account.next")}
        </Button>
        {/* Something has to be moving. Looking up a domain and then reaching a mail server takes
            as long as both of them take, and a button that went quiet reads as a button that did
            nothing. */}
        {working !== null && (
          <span
            role="status"
            className="flex items-center gap-2 text-sm text-muted-foreground"
          >
            <Loader2
              aria-hidden="true"
              className="size-4 animate-spin motion-reduce:animate-none"
            />
            {working === "looking"
              ? t("account.findingServers")
              : t("account.addingNow")}
          </span>
        )}
      </div>
    </form>
  );
}
