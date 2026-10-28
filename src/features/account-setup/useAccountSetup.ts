import { useCallback, useState } from "react";

import {
  addAccount,
  changeAccount,
  defaultPortFor,
  isCommandError,
  testAccountConnection,
} from "@/shared/ipc/client";
import type { AccountSettingsDto } from "@/shared/ipc/generated/AccountSettingsDto";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { EncryptionDto } from "@/shared/ipc/generated/EncryptionDto";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";

/** A failure the core reported, or the bridge itself not answering. */
export type Failure =
  { kind: "command"; error: CommandError } | { kind: "bridge" };

export type Outcome =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "tested"; mailboxCount: number }
  | { status: "adding" }
  | { status: "added"; account: MailAccountSummaryDto }
  | { status: "failed"; failure: Failure };

export type Draft = {
  email: string;
  displayName: string;
  imapHost: string;
  /** Held as text: an empty field and a zero are different things to the person typing. */
  imapPort: string;
  imapEncryption: EncryptionDto;
  imapUsername: string;
  /**
   * Where messages written from this account are handed over.
   *
   * Left blank is an ordinary answer: reading and sending are separate servers, and an account
   * nobody has given a sending server still receives mail. It just cannot answer any of it, and
   * says so instead of losing what was written.
   */
  smtpHost: string;
  smtpPort: string;
  smtpEncryption: EncryptionDto;
  smtpUsername: string;
  password: string;
};

export const EMPTY_DRAFT: Draft = {
  email: "",
  displayName: "",
  imapHost: "",
  imapPort: "993",
  imapEncryption: "tls",
  imapUsername: "",
  // Encrypted from the first byte, for sending as well as for reading. STARTTLS begins in the
  // clear and asks to be upgraded, which is a request a machine in the middle can decline; 465 and
  // 993 never offer it the chance. RFC 8314 says to prefer these, and a default is what almost
  // nobody changes.
  smtpHost: "",
  smtpPort: "465",
  smtpEncryption: "tls",
  smtpUsername: "",
  password: "",
};

/**
 * The draft with the servers somebody found written into it.
 *
 * A plain function over values rather than something that only happens inside `setDraft`: what is
 * found is wanted twice in the same breath -- on screen, and in the connection that is about to be
 * tried -- and a state update has not happened yet at the moment the second one needs it.
 */
export function withServers(draft: Draft, found: AccountSettingsDto): Draft {
  return {
    ...draft,
    imapHost: found.imapHost,
    imapPort: String(found.imapPort),
    imapEncryption: found.imapEncryption,
    imapUsername: found.imapUsername,
    smtpHost: found.submission?.host ?? draft.smtpHost,
    smtpPort: found.submission ? String(found.submission.port) : draft.smtpPort,
    smtpEncryption: found.submission?.encryption ?? draft.smtpEncryption,
    smtpUsername: found.submission?.username ?? draft.smtpUsername,
  };
}

/** The user name defaults to the address, which is what most servers expect. */
export function settingsOf(draft: Draft): AccountSettingsDto {
  return {
    email: draft.email.trim(),
    displayName: draft.displayName.trim(),
    imapHost: draft.imapHost.trim(),
    imapPort: Number.parseInt(draft.imapPort, 10) || 0,
    imapEncryption: draft.imapEncryption,
    imapUsername: draft.imapUsername.trim() || draft.email.trim(),
    // The host is what decides. Somebody who has not typed one has not said where to send, and a
    // port on its own is not a server.
    submission: draft.smtpHost.trim()
      ? {
          host: draft.smtpHost.trim(),
          port: Number.parseInt(draft.smtpPort, 10) || 0,
          encryption: draft.smtpEncryption,
          username:
            draft.smtpUsername.trim() ||
            draft.imapUsername.trim() ||
            draft.email.trim(),
        }
      : null,
  };
}

function failureOf(reason: unknown): Failure {
  return isCommandError(reason)
    ? { kind: "command", error: reason }
    : { kind: "bridge" };
}

/**
 * The form's asynchronous half: the two round trips to the core and what came back.
 *
 * The password is held in component state for as long as the form is open and is passed to the
 * core as an argument. It is never put into the settings object, which is what ends up in logs and
 * error messages.
 */
export function useAccountSetup(
  onAdded: (account: MailAccountSummaryDto) => void,
) {
  const [outcome, setOutcome] = useState<Outcome>({ status: "idle" });
  const busy = outcome.status === "testing" || outcome.status === "adding";

  const test = useCallback((draft: Draft) => {
    setOutcome({ status: "testing" });
    testAccountConnection(settingsOf(draft), draft.password)
      .then((report) =>
        setOutcome({ status: "tested", mailboxCount: report.mailboxCount }),
      )
      .catch((reason: unknown) =>
        setOutcome({ status: "failed", failure: failureOf(reason) }),
      );
  }, []);

  /**
   * Stores the account, adding it or correcting one that is already here.
   *
   * Which of the two is decided by whether the caller named an account, not by asking the store:
   * a screen that opened on an existing account knows which one it opened on, and asking again
   * would be a second answer to a question already answered.
   */
  const add = useCallback(
    (draft: Draft, correcting?: string) => {
      setOutcome({ status: "adding" });
      const stored =
        correcting === undefined
          ? addAccount(settingsOf(draft), draft.password)
          : changeAccount(correcting, settingsOf(draft), draft.password);

      stored
        .then((account) => {
          setOutcome({ status: "added", account });
          onAdded(account);
        })
        .catch((reason: unknown) =>
          setOutcome({ status: "failed", failure: failureOf(reason) }),
        );
    },
    [onAdded],
  );

  /**
   * The port that goes with an encryption, asked of the core rather than repeated here: two
   * places that both know the defaults eventually disagree.
   */
  const portFor = useCallback(
    (encryption: EncryptionDto) => defaultPortFor(encryption).catch(() => null),
    [],
  );

  const reset = useCallback(() => setOutcome({ status: "idle" }), []);

  return { outcome, busy, test, add, portFor, reset };
}
