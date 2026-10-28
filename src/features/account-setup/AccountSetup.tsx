import { CircleCheck, CircleX, Loader2 } from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";

import {
  describeError,
  useTranslation,
  type TranslationKey,
  type Translator,
} from "@/shared/i18n";
import { AccountStart } from "./AccountStart";
import { secretStoreName } from "@/shared/platform";
import { Field } from "./Field";
import { Button } from "@/shared/ui/primitives/button";
import { findServerSettings } from "@/shared/ipc/client";
import { Input } from "@/shared/ui/primitives/input";
import { Label } from "@/shared/ui/primitives/label";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/primitives/alert-dialog";
import {
  getAccountSettings,
  isCommandError,
  removeAccount,
} from "@/shared/ipc/client";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { EncryptionDto } from "@/shared/ipc/generated/EncryptionDto";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import { typedDirection } from "@/shared/ui/direction";

import {
  EMPTY_DRAFT,
  useAccountSetup,
  withServers,
  type Draft,
  type Outcome,
} from "./useAccountSetup";

const ENCRYPTIONS: { value: EncryptionDto; label: TranslationKey }[] = [
  { value: "tls", label: "account.encryptionTls" },
  { value: "startTls", label: "account.encryptionStartTls" },
  { value: "none", label: "account.encryptionNone" },
];

/**
 * The account form.
 *
 * Two actions, because they answer different questions: checking says whether the settings reach
 * the server, adding stores them. Both are verified by the core against the real server, so a
 * mistyped password cannot become a stored account.
 */
export function AccountSetup({
  onAdded,
  onCancel,
  step,
  onStep,
  leaveRef,
  onRemoved,
  reconnecting,
  secretsInAFile = false,
  address,
  passwordMissing,
}: {
  onAdded: (account: MailAccountSummaryDto) => void;
  onCancel: () => void;
  /**
   * Which of the two screens is showing.
   *
   * Held outside this form because the way back is the arrow in the pane's corner, which stands
   * beside the screen rather than in it -- the same corner on every screen, so a person looks in
   * one place for it.
   */
  step: "start" | "settings";
  onStep: (step: "start" | "settings") => void;
  /**
   * Where the pane's cross finds the way out of this screen.
   *
   * Leaving here is refused while the form is working -- walking out halfway through connecting
   * leaves a person unable to tell whether the account was added. The button that held that
   * refusal is gone, so the refusal lives in the leaving itself.
   */
  leaveRef?: { current: (() => void) | null };
  /** Told after an account has been taken off this device, so the sidebar can stop showing it. */
  onRemoved?: (accountId: string) => void;
  /** An account already here. Fills the form in. */
  reconnecting?: string;
  /**
   * Whether this device keeps its secrets in a file a master password opens rather than in a store
   * of the system's own. Every sentence here about where a password goes names one of the two, and
   * the system's store named on a device without one is the confusion the master password is there
   * to prevent.
   */
  secretsInAFile?: boolean;
  /**
   * The address of that account, as the window's list of accounts holds it.
   *
   * What a removal asks to be typed back, and handed in together with the account rather than read
   * with its settings, for two reasons. The settings can fail to read for an account the list shows
   * perfectly well -- a row an older version wrote, a store that did not answer -- and that is
   * exactly the account somebody opens to take away. And the address field below can be edited:
   * the account a removal takes is the stored one, and a question naming whatever was typed over
   * it would be asking about an account that is not the one going.
   */
  address?: string;
  /**
   * Why it was opened: because the password is gone, or because somebody asked to see it.
   *
   * The two want different words. Told "enter the password again" about an account that is working
   * perfectly, a person looks for the problem there is not.
   */
  passwordMissing?: boolean;
}) {
  const t = useTranslation();
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);

  // The settings are already known; what is missing is the password, and that is the only thing
  // the person should have to find.
  useEffect(() => {
    if (!reconnecting) return;
    let active = true;

    getAccountSettings(reconnecting)
      .then((settings) => {
        if (!active) return;
        setDraft((current) => ({
          ...current,
          email: settings.email,
          displayName: settings.displayName,
          imapHost: settings.imapHost,
          imapPort: String(settings.imapPort),
          imapEncryption: settings.imapEncryption,
          imapUsername: settings.imapUsername,
          smtpHost: settings.submission?.host ?? "",
          smtpPort: settings.submission
            ? String(settings.submission.port)
            : current.smtpPort,
          smtpEncryption:
            settings.submission?.encryption ?? current.smtpEncryption,
          smtpUsername: settings.submission?.username ?? "",
        }));
        setPortEdited(true);
      })
      .catch(() => {
        /* An empty form is still a form; nothing here is worth an error message. */
      });

    return () => {
      active = false;
    };
  }, [reconnecting]);
  /** Once the port is typed in by hand, changing the encryption stops overwriting it. */
  const [portEdited, setPortEdited] = useState(false);
  const { outcome, busy, test, add, portFor, reset } = useAccountSetup(onAdded);

  /** Leaving, and not while the form is in the middle of something. */
  function leave() {
    if (busy) return;
    onCancel();
  }

  // No dependency list: `leave` reads this render's `busy`, and a leaving handed over once would
  // still be answering for the form as it was when the screen opened.
  //
  // Before the paint rather than after it, for the same reason the composer publishes its own that
  // way: the cross belongs to the pane and can be pressed the instant this screen is drawn, and an
  // ordinary effect republishes the handler only after the browser has painted. In that window the
  // screen says the form has stopped and the cross still refuses to leave.
  useLayoutEffect(() => {
    if (!leaveRef) return;
    leaveRef.current = leave;
    return () => {
      leaveRef.current = null;
    };
  });

  /**
   * Whether the servers are being looked for, so the button can say so.
   *
   * Its own state rather than the form's `busy`: that one greys out the whole form while an
   * account is being tried, and greying out the address while looking for its servers would take
   * away the one field a person might want to correct.
   */
  const [looking, setLooking] = useState<"no" | "yes" | "nothing" | "found">(
    "no",
  );

  /**
   * Fills in the servers for the address, when anybody knows them.
   *
   * On the button and never while typing: a search on every keystroke would send half-finished
   * domains to whoever owns them, and a half-finished one is not even this person's.
   *
   * The first answer is an account already on this device at the same domain, so the second
   * mailbox somebody adds there is connected from the first screen without anything more asked.
   */
  const findServers = async (): Promise<Draft | null> => {
    setLooking("yes");
    try {
      const { settings } = await findServerSettings(draft.email, reconnecting);
      if (settings === null) {
        setLooking("nothing");
        return null;
      }
      // Written into the draft on screen, and handed back for the connection that follows it. The
      // update has not run yet at the moment the second of those needs the answer, so the merge
      // is a function over values and is applied twice rather than read out of state once.
      setDraft((current) => withServers(current, settings));
      setLooking("found");
      return withServers(draft, settings);
    } catch {
      // Nobody answered, or nobody knows. The same thing to the person either way: they type the
      // settings in, which is what they would have done without the button.
      setLooking("nothing");
      return null;
    }
  };

  /**
   * What the first screen's button does: look the address up, and connect if anybody knows it.
   *
   * Nothing found means nobody could answer for this mailbox, and the long form is where a person
   * answers for it themselves. A connection refused ends up in the same place, because that is
   * where every field it might be wrong in can be seen and corrected.
   */
  const continueFromStart = async () => {
    const filled = await findServers();
    if (filled === null) {
      onStep("settings");
      return;
    }
    add(filled, reconnecting);
  };

  const edit = (field: keyof Draft) => (value: string) => {
    // A different address is a different question, so an answer about the last one stops standing.
    if (field === "email") setLooking("no");
    setDraft((current) => ({ ...current, [field]: value }));
    if (outcome.status !== "idle") reset();
  };

  const chooseEncryption = (encryption: EncryptionDto) => {
    setDraft((current) => ({ ...current, imapEncryption: encryption }));
    if (outcome.status !== "idle") reset();
    if (portEdited) return;
    portFor(encryption).then((port) => {
      if (port === null) return;
      setDraft((current) =>
        // The answer arrives after another encryption may have been picked.
        current.imapEncryption === encryption
          ? { ...current, imapPort: String(port) }
          : current,
      );
    });
  };

  // A refusal on the first screen has nowhere to be acted on: that screen holds three fields and
  // the trouble is usually in one of the eight it does not show. The long form is where every one
  // of them can be seen, so that is where the answer is given.
  useEffect(() => {
    if (step === "start" && outcome.status === "failed") onStep("settings");
  }, [step, outcome.status, onStep]);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    // `reconnecting` is the account this screen opened on, and its presence is what makes this a
    // correction rather than an addition. A person who came here because something was wrong can
    // fix the host and the port in the same breath as the password, which until now was only
    // possible by removing the account -- and removing an account takes its mail with it.
    add(draft, reconnecting);
  };

  /**
   * Whether this screen is correcting an account that is already here.
   *
   * One answer to that question rather than one per place that asks: the words on the button, the
   * line under it and the sentence the working state prints all mean the same thing by it.
   */
  const editing = reconnecting !== undefined;

  const complete =
    draft.email.trim() !== "" &&
    draft.imapHost.trim() !== "" &&
    draft.password !== "";

  if (step === "start") {
    return (
      <AccountStart
        secretsInAFile={secretsInAFile}
        draft={draft}
        onEdit={(field, value) => edit(field)(value)}
        onContinue={() => void continueFromStart()}
        working={
          outcome.status === "adding"
            ? "adding"
            : looking === "yes"
              ? "looking"
              : null
        }
      />
    );
  }

  return (
    <form
      className="m-auto flex w-full max-w-xl flex-col gap-6 px-8 pt-8"
      onSubmit={submit}
      aria-labelledby="account-setup-title"
    >
      <header className="flex flex-col gap-2">
        <h1
          id="account-setup-title"
          className="text-xl font-semibold tracking-tight"
        >
          {!reconnecting
            ? t("account.manualTitle")
            : passwordMissing
              ? t("account.reconnectTitle")
              : t("account.settingsTitle")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {/* Two ways lead here and they are not the same news. Nobody knew the address, or
              somebody did and the mail server refused what was tried — telling the second person
              that their provider is unknown would be a sentence that is simply untrue. */}
          {!reconnecting
            ? looking === "nothing"
              ? t("account.serversNotFound")
              : t("account.manualIntro")
            : passwordMissing
              ? secretsInAFile
                ? t("account.reconnectIntroFile")
                : t("account.reconnectIntro", { store: t(secretStoreName()) })
              : t("account.settingsIntro")}
        </p>
      </header>

      <fieldset className="flex flex-col gap-4" disabled={busy}>
        <legend className="sr-only">{t("account.identity")}</legend>

        <Field id="account-email" label={t("account.email")} required>
          <Input
            id="account-email"
            dir="ltr"
            required
            type="email"
            autoComplete="email"
            spellCheck={false}
            value={draft.email}
            onChange={(event) => edit("email")(event.target.value)}
          />
        </Field>

        <Field
          id="account-display-name"
          label={t("account.displayName")}
          hint={t("account.displayNameHint")}
        >
          <Input
            id="account-display-name"
            dir={typedDirection(draft.displayName)}
            aria-describedby="account-display-name-hint"
            autoComplete="name"
            value={draft.displayName}
            onChange={(event) => edit("displayName")(event.target.value)}
          />
        </Field>
      </fieldset>

      <fieldset
        className="flex flex-col gap-4 border-t border-border pt-6"
        disabled={busy}
      >
        <legend className="px-0 text-sm font-medium">
          {t("account.server")}
        </legend>

        <div className="flex gap-3">
          <div className="flex-1">
            <Field id="account-host" label={t("account.host")} required>
              <Input
                id="account-host"
                dir="ltr"
                required
                // The example is in the field rather than under it. A line of grey text saying
                // "for example" costs a line of the form and is read after the field it explains;
                // in the field it is read before, which is when it is of any use.
                placeholder={t("account.hostExample")}
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                value={draft.imapHost}
                onChange={(event) => edit("imapHost")(event.target.value)}
              />
            </Field>
          </div>
          <div className="w-24">
            <Field id="account-port" label={t("account.port")}>
              <Input
                id="account-port"
                dir="ltr"
                inputMode="numeric"
                value={draft.imapPort}
                onChange={(event) => {
                  setPortEdited(true);
                  edit("imapPort")(event.target.value.replace(/\D/g, ""));
                }}
              />
            </Field>
          </div>
        </div>

        <div
          role="radiogroup"
          aria-label={t("account.encryption")}
          className="flex flex-col gap-2"
        >
          <span className="text-sm font-medium">{t("account.encryption")}</span>
          <div className="flex gap-2">
            {ENCRYPTIONS.map(({ value, label }) => (
              <Button
                key={value}
                type="button"
                role="radio"
                aria-checked={draft.imapEncryption === value}
                variant={
                  draft.imapEncryption === value ? "secondary" : "outline"
                }
                onClick={() => chooseEncryption(value)}
              >
                {t(label)}
              </Button>
            ))}
          </div>
          {draft.imapEncryption === "none" && (
            <p className="text-xs text-muted-foreground">
              {t("account.encryptionNoneHint")}
            </p>
          )}
        </div>

        <Field
          id="account-username"
          label={t("account.username")}
          hint={t("account.usernameHint")}
        >
          <Input
            id="account-username"
            dir="ltr"
            aria-describedby="account-username-hint"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            placeholder={draft.email.trim()}
            value={draft.imapUsername}
            onChange={(event) => edit("imapUsername")(event.target.value)}
          />
        </Field>

        <div className="flex flex-col gap-4 border-t border-border pt-4">
          <div>
            <span className="text-sm font-medium">{t("account.sending")}</span>
            <p className="text-xs text-muted-foreground">
              {t("account.sendingHint")}
            </p>
          </div>

          <div className="flex gap-3">
            <div className="flex-1">
              <Field id="account-smtp-host" label={t("account.sendingHost")}>
                <Input
                  id="account-smtp-host"
                  dir="ltr"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  value={draft.smtpHost}
                  onChange={(event) => edit("smtpHost")(event.target.value)}
                />
              </Field>
            </div>
            <div className="w-24">
              <Field id="account-smtp-port" label={t("account.sendingPort")}>
                <Input
                  id="account-smtp-port"
                  dir="ltr"
                  inputMode="numeric"
                  value={draft.smtpPort}
                  onChange={(event) =>
                    edit("smtpPort")(event.target.value.replace(/\D/g, ""))
                  }
                />
              </Field>
            </div>
          </div>

          <div
            role="radiogroup"
            aria-label={t("account.sendingEncryption")}
            className="flex gap-2"
          >
            {ENCRYPTIONS.map(({ value, label }) => (
              <Button
                key={value}
                type="button"
                role="radio"
                // Named for the half it belongs to. The same three words appear above for reading,
                // and a screen reader announcing "STARTTLS" twice in one form says nothing about
                // which server it is about.
                aria-label={t("account.sendingWith", { method: t(label) })}
                aria-checked={draft.smtpEncryption === value}
                variant={
                  draft.smtpEncryption === value ? "secondary" : "outline"
                }
                onClick={() => edit("smtpEncryption")(value)}
              >
                {t(label)}
              </Button>
            ))}
          </div>

          <Field
            id="account-smtp-username"
            label={t("account.sendingUsername")}
            hint={t("account.sendingUsernameHint")}
          >
            <Input
              id="account-smtp-username"
              dir="ltr"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              placeholder={draft.imapUsername.trim() || draft.email.trim()}
              value={draft.smtpUsername}
              onChange={(event) => edit("smtpUsername")(event.target.value)}
            />
          </Field>
        </div>

        <Field
          id="account-password"
          label={t("account.password")}
          hint={
            secretsInAFile
              ? t("account.passwordHintFile")
              : t("account.passwordHint", { store: t(secretStoreName()) })
          }
          required
        >
          <Input
            id="account-password"
            dir="ltr"
            required
            aria-describedby="account-password-hint"
            type="password"
            autoComplete="current-password"
            value={draft.password}
            onChange={(event) => edit("password")(event.target.value)}
          />
        </Field>
      </fieldset>

      {/* Stuck to the bottom of the pane, because twelve fields are taller than any window this
          ships in: at the size it opens the button sat two hundred pixels below the fold, and
          opening an account's settings showed a form with no way to commit it until the person
          scrolled to the end. Backed rather than transparent, so the fields do not read through
          it, and carrying the bottom padding the form gave up. */}
      <div className="sticky bottom-0 flex flex-col gap-3 border-t border-border bg-background pt-6 pb-8">
        <Result
          outcome={outcome}
          t={t}
          saving={editing}
          secretsInAFile={secretsInAFile}
        />

        <div className="flex items-center gap-2">
          <Button type="submit" disabled={busy || !complete}>
            {editing
              ? outcome.status === "adding"
                ? t("account.saving")
                : t("account.save")
              : outcome.status === "adding"
                ? t("account.adding")
                : t("account.add")}
          </Button>
          <Button
            type="button"
            variant="outline"
            disabled={busy || !complete}
            onClick={() => test(draft)}
          >
            {outcome.status === "testing"
              ? t("account.testing")
              : t("account.test")}
          </Button>
        </div>

        {/* Why it will not press. The screen fills every other field from what is stored and
            deliberately does not fill this one -- the password lives in this device's secret store and
            this window never reads it back -- so a person opening their settings meets a Save that
            is grey for a reason nothing on the screen gives. */}
        {editing && draft.password === "" && (
          <p className="text-xs text-muted-foreground">
            {secretsInAFile
              ? t("account.saveNeedsPasswordFile")
              : t("account.saveNeedsPassword", { store: t(secretStoreName()) })}
          </p>
        )}
      </div>

      {/* Only for an account already here: there is nothing to remove while one is being added.
          And only with its address, because the question asks for that address to be typed back,
          and with none known an empty field would match it before anything was typed. The
          padding below is the pane's own margin: the form gives its bottom padding to the sticky
          block above, and without this the button sat on the window's edge. */}
      {reconnecting && address !== undefined && address.trim() !== "" && (
        <div className="mt-6 border-t border-border pt-6 pb-8">
          <RemoveAccount
            accountId={reconnecting}
            email={address}
            disabled={busy}
            onRemoved={onRemoved}
            secretsInAFile={secretsInAFile}
            t={t}
          />
        </div>
      )}
    </form>
  );
}

/**
 * What the last action produced, said in one line with a mark beside it.
 *
 * A person pressing "check connection" is asking one question, and the answer to it is yes or no.
 * The number of folders is the evidence rather than the answer, so it is said underneath and
 * quietly — and while the question is still open, something has to be moving, or a form that is
 * waiting on a slow server looks like a form that ignored the press.
 *
 * A failure is announced, so a screen reader does not miss it.
 */
function Result({
  outcome,
  t,
  saving,
  secretsInAFile,
}: {
  outcome: Outcome;
  t: Translator;
  /** Correcting an account already here, rather than adding one. */
  saving: boolean;
  /** Where a refusal about the secrets has to say they are. */
  secretsInAFile: boolean;
}) {
  if (outcome.status === "testing" || outcome.status === "adding") {
    return (
      <p
        className="flex items-center gap-2 text-sm text-muted-foreground"
        role="status"
      >
        <Loader2 aria-hidden="true" className="size-4 shrink-0 animate-spin" />
        {t(
          outcome.status === "testing"
            ? "account.checking"
            : saving
              ? "account.savingNow"
              : "account.addingNow",
        )}
      </p>
    );
  }

  if (outcome.status === "tested") {
    return (
      <div role="status" className="flex items-start gap-2 text-sm">
        <CircleCheck
          aria-hidden="true"
          className="mt-0.5 size-4 shrink-0 text-primary"
        />
        <span className="flex flex-col">
          <span className="text-foreground">{t("account.connected")}</span>
          <span className="text-xs text-muted-foreground">
            {t("account.tested", { count: outcome.mailboxCount })}
          </span>
        </span>
      </div>
    );
  }

  if (outcome.status === "added") {
    return (
      <p
        className="flex items-center gap-2 text-sm text-foreground"
        role="status"
      >
        <CircleCheck
          aria-hidden="true"
          className="size-4 shrink-0 text-primary"
        />
        {t("account.added", { email: outcome.account.email })}
      </p>
    );
  }

  if (outcome.status === "failed") {
    const reason =
      outcome.failure.kind === "command"
        ? describeError(outcome.failure.error, t, secretsInAFile)
        : t("error.bridgeUnavailable");
    return (
      <div role="alert" className="flex items-start gap-2 text-sm">
        <CircleX
          aria-hidden="true"
          className="mt-0.5 size-4 shrink-0 text-destructive"
        />
        <span className="flex flex-col">
          <span className="text-destructive">{t("account.notConnected")}</span>
          <span className="text-xs text-muted-foreground">{reason}</span>
        </span>
      </div>
    );
  }

  return null;
}

/**
 * The way to take an account off this device, and the question asked before it goes.
 *
 * Asked in a dialog that wants the address typed back. What follows cannot be undone -- the mail,
 * the password and anything waiting to be sent go with it -- and a question answered by one more
 * press is a question answered by the same slip that asked it. Typing the address is nothing a
 * hand does by accident, and it makes the person read which account it is.
 *
 * The dialog stays up until the answer is in. Nothing closes it while the removal is on its way,
 * and a removal that failed keeps it open with the failure said inside: a dialog that closed after
 * a failure would read as a removal that happened.
 */
function RemoveAccount({
  accountId,
  email,
  disabled,
  onRemoved,
  secretsInAFile,
  t,
}: {
  accountId: string;
  /** The address as it is stored, which is what has to be typed back. Never empty. */
  email: string;
  /** The form is in the middle of something, which is no moment to take its account away. */
  disabled: boolean;
  onRemoved?: (accountId: string) => void;
  /** Where a refusal about the secrets has to say they are. */
  secretsInAFile: boolean;
  t: Translator;
}) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [removing, setRemoving] = useState(false);
  /** Why the last attempt failed: a code from the core, or the bridge not answering at all. */
  const [failure, setFailure] = useState<CommandError | "bridge" | null>(null);
  const field = useRef<HTMLInputElement>(null);

  // Spaces carried in with a pasted address, and the case it was typed in, say nothing about
  // which account is meant, so neither stands between a person and the answer they gave.
  const confirmed = typed.trim().toLowerCase() === email.trim().toLowerCase();

  const remove = () => {
    if (!confirmed || removing) return;
    setRemoving(true);
    setFailure(null);

    removeAccount(accountId)
      .then(() => {
        setRemoving(false);
        setOpen(false);
        onRemoved?.(accountId);
      })
      .catch((reason: unknown) => {
        setRemoving(false);
        setFailure(isCommandError(reason) ? reason : "bridge");
        // Back in the field, where Enter tries again. The button that was pressed was disabled
        // for the attempt, and a disabled button lets the focus fall out of the dialog.
        field.current?.focus();
      });
  };

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        // Not while the removal is on its way, whichever way out was tried. Closed then, the
        // dialog would say nothing about whether the account went.
        if (removing) return;
        setOpen(next);
        // Each asking starts empty. An address left typed from the last time is an answer given
        // before the question was read.
        if (next) {
          setTyped("");
          setFailure(null);
        }
      }}
    >
      <AlertDialogTrigger asChild>
        {/* Red from the palette being worn rather than a red of its own, so it stays the same
            warning in the light, in the dark and in every scheme. Quiet until it is pointed at:
            it is the last thing on the form, not the thing the form is for. */}
        <Button
          type="button"
          variant="ghost"
          disabled={disabled}
          className="text-destructive hover:bg-destructive/10 hover:text-destructive focus-visible:border-destructive/40 focus-visible:ring-destructive/20 aria-expanded:bg-destructive/10 aria-expanded:text-destructive dark:hover:bg-destructive/20"
        >
          {t("settings.removeAccount")}
        </Button>
      </AlertDialogTrigger>

      <AlertDialogContent
        // No key pressed in here reaches the window. The dialog is the only thing on the screen
        // while it is up, and a letter the window took as a shortcut -- `c` from the refusal
        // button -- would open a message behind it, under a question that then seems to be about
        // the message. Escape is not among them: the dialog takes that one before it gets here.
        onKeyDown={(event) => event.stopPropagation()}
        // Into the field rather than onto the refusal: the only way forward is typing, and Enter
        // there does nothing until the address matches.
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          field.current?.focus();
        }}
        // Taken here and no further. The window leaves the screen on Escape, and the key handed on
        // would close the whole form behind the dialog -- out from under a removal on its way.
        // Whether it closes the dialog is the same answer every other way out gets, above.
        onEscapeKeyDown={(event) => event.stopPropagation()}
      >
        {/* The primitive aligns its header left on a window this wide; the start of the line is
            the left in one language and the right in another. */}
        <AlertDialogHeader className="sm:group-data-[size=default]/alert-dialog-content:text-start">
          <AlertDialogTitle className="[overflow-wrap:anywhere]">
            {t("settings.removeAccountAsks", { email })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("settings.removeAccountExplains")}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="flex flex-col gap-1.5">
          <Label
            htmlFor="remove-account-address"
            className="leading-snug [overflow-wrap:anywhere]"
          >
            {t("settings.removeAccountTypeAddress", { email })}
          </Label>
          {/* Text rather than an email field: that one strips the spaces a person typed before
              the comparison ever sees them, and hands back a different string than is shown. */}
          <Input
            ref={field}
            id="remove-account-address"
            dir="ltr"
            type="text"
            inputMode="email"
            autoComplete="off"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            readOnly={removing}
            value={typed}
            onChange={(event) => setTyped(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || event.nativeEvent.isComposing) {
                return;
              }
              // Not a form of its own: React carries a submit out of the dialog to the account
              // form it was opened from, and that one would save the settings.
              event.preventDefault();
              remove();
            }}
          />
        </div>

        {failure !== null && (
          <div role="alert" className="flex items-start gap-2 text-sm">
            <CircleX
              aria-hidden="true"
              className="mt-0.5 size-4 shrink-0 text-destructive"
            />
            <span className="flex flex-col">
              <span className="text-destructive">
                {t("settings.removeAccountFailed")}
              </span>
              <span className="text-xs text-muted-foreground">
                {failure === "bridge"
                  ? t("error.bridgeUnavailable")
                  : describeError(failure, t, secretsInAFile)}
              </span>
            </span>
          </div>
        )}

        <AlertDialogFooter>
          <AlertDialogCancel disabled={removing}>
            {t("settings.removeAccountKeeps")}
          </AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={!confirmed || removing}
            onClick={(event) => {
              // Closed by the answer rather than by the press: the primitive closes on any press,
              // and a dialog gone before the core has answered cannot say that it refused.
              event.preventDefault();
              remove();
            }}
          >
            {removing
              ? t("settings.removeAccountRemoving")
              : t("settings.removeAccountConfirms")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
