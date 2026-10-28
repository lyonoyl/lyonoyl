import { describe, expect, it } from "vitest";

import { LANGUAGES, SUPPORTED_LANGUAGES } from "./catalog";
import { ERROR_MESSAGE_KEYS, describeError } from "./errors";
import { createTranslator } from "./translate";

import { secretStoreName } from "@/shared/platform";

import type { CommandError } from "@/shared/ipc/generated/CommandError";
import type { ErrorCode } from "@/shared/ipc/generated/ErrorCode";

const failure = (
  code: ErrorCode,
  message = "raw diagnostic",
): CommandError => ({
  code,
  message,
  path: null,
});

describe("error text", () => {
  it.each(SUPPORTED_LANGUAGES)(
    "%s has text for every error code",
    (language) => {
      const missing = Object.values(ERROR_MESSAGE_KEYS).filter(
        (key) => !(key in LANGUAGES[language].catalog),
      );
      expect(missing).toEqual([]);
    },
  );

  it("renders a failure in the active language, not in the language of the core", () => {
    expect(
      describeError(failure("storageUnavailable"), createTranslator("ru")),
    ).toBe("Не удалось открыть локальную базу данных");
    expect(
      describeError(failure("storageUnavailable"), createTranslator("en")),
    ).toBe("The local database could not be opened");
  });

  it("names the file a master password opens, on a device whose secrets are in one", () => {
    // That file has no name a system gave it, so the refusal is a sentence of its own. The system's
    // store named on a device that keeps its secrets in a file sends a person looking for a place
    // this device does not have.
    const en = createTranslator("en");
    const named = LANGUAGES.en.catalog[secretStoreName()] as string;

    expect(describeError(failure("credentialsUnavailable"), en, true)).toBe(
      "Could not access the file your master password opens on this device",
    );
    expect(describeError(failure("credentialsUnavailable"), en)).toBe(
      `Could not reach ${named} on this device`,
    );
  });

  it("says what it says anywhere else about a refusal that names no store", () => {
    expect(
      describeError(failure("storageUnavailable"), createTranslator("en"), true),
    ).toBe("The local database could not be opened");
  });

  it("never shows the raw diagnostic for a code it knows", () => {
    const rendered = describeError(
      failure("internal", "thread 'main' panicked"),
      createTranslator("en"),
    );
    expect(rendered).not.toContain("panicked");
  });

  it("never shows the message of a code it does not know either", () => {
    // The message is written in English where it is raised. Showing it would put English in front
    // of a reader who chose another language, which is the one thing the catalog exists to prevent.
    const unknown = {
      code: "quotaExceeded",
      message: "Mailbox is full.",
      path: null,
    } as unknown as CommandError;
    expect(describeError(unknown, createTranslator("ru"))).toBe(
      "Произошёл неожиданный сбой",
    );
  });
});
