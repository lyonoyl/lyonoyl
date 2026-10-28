import type { TranslationKey } from "./catalog";
import type { Translator } from "./translate";

import { secretStoreName } from "@/shared/platform";

import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { ErrorCode } from "@/shared/ipc/generated/ErrorCode";

/**
 * The core reports failures as codes; the text for each code comes from the active catalog.
 *
 * The map is exhaustive: adding a variant to `ErrorCode` in Rust regenerates the union, and the
 * build fails until the new code has text.
 */
export const ERROR_MESSAGE_KEYS: Record<ErrorCode, TranslationKey> = {
  storageUnavailable: "error.storageUnavailable",
  storageKeyUnavailable: "error.storageKeyUnavailable",
  storageFromANewerBuild: "error.storageFromANewerBuild",
  storageCorrupted: "error.storageCorrupted",
  invalidRequest: "error.invalidRequest",
  alreadyReading: "error.alreadyReading",
  internal: "error.internal",
  offline: "error.offline",
  unauthorized: "error.unauthorized",
  providerProtocol: "error.providerProtocol",
  notOursToReplace: "update.notOursToReplace",
  cancelled: "error.cancelled",
  credentialsUnavailable: "error.credentialsUnavailable",
  credentialsMissing: "error.credentialsMissing",
  invalidEmail: "error.invalidEmail",
  invalidHost: "error.invalidHost",
  invalidPort: "error.invalidPort",
  invalidUsername: "error.invalidUsername",
  accountAlreadyAdded: "error.accountAlreadyAdded",
  noNumbersLeftForAnAccount: "error.noNumbersLeftForAnAccount",
  masterPasswordRejected: "error.masterPasswordRejected",
  folderCannotReceive: "error.folderCannotReceive",
  noPlaceToSave: "error.noPlaceToSave",
  fileNotWritten: "error.fileNotWritten",
  fileTooLarge: "error.fileTooLarge",
  fileNotRead: "error.fileNotRead",
};

/**
 * Translates a failure for display.
 *
 * The `message` the core sends is never shown: it is written in English at the place it is raised
 * and would appear untranslated in front of a reader who chose another language. A code with no
 * text of its own means the two halves of one build disagree, which is a defect, not a language.
 *
 * `secretsInAFile` is whether this device keeps its secrets in the file its master password opens
 * rather than in a store of the system's own. That file has no name a system gave it, so a refusal
 * about it is a sentence of its own rather than another `{store}`. The account screens pass it:
 * they are where secrets are written, and an open file answers every read, so they are where the
 * refusal meets a device that has one.
 */
export function describeError(
  error: CommandError,
  t: Translator,
  secretsInAFile = false,
): string {
  if (secretsInAFile && error.code === "credentialsUnavailable") {
    return t("error.credentialsUnavailableFile");
  }

  // The name of this system's own place for secrets, handed to every refusal rather than only to
  // the ones that use it: a sentence that uses it is a sentence written by a translator, and
  // the one place that decides which refusals may name it is the catalog. What no sentence uses,
  // no sentence shows.
  return t(ERROR_MESSAGE_KEYS[error.code] ?? "error.unknown", {
    store: t(secretStoreName()),
  });
}
