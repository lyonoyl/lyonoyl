import { afterEach, describe, expect, it } from "vitest";

import { LANGUAGES, type Catalog, type TranslationKey } from "./catalog";
import { createTranslator, listOf } from "./translate";

/** Simulates a catalog shipped with a key missing, which the type system cannot express. */
function withoutKey(language: keyof typeof LANGUAGES, key: TranslationKey) {
  const catalog = LANGUAGES[language].catalog as Catalog;
  const original = catalog[key];
  delete (catalog as Record<string, unknown>)[key];
  return () => {
    catalog[key] = original;
  };
}

const restores: Array<() => void> = [];
afterEach(() => {
  while (restores.length) restores.pop()?.();
});

describe("a value inside a sentence written right to left", () => {
  it("is isolated, so a name in Latin letters keeps its own order", () => {
    const said = createTranslator("ar")("settings.thisVersion", { version: "1.2.3" });
    expect(said).toContain("\u20681.2.3\u2069");
  });

  it("is left as it is in a sentence written left to right", () => {
    const said = createTranslator("en")("settings.thisVersion", { version: "1.2.3" });
    expect(said).toBe("This is version 1.2.3");
  });
});

describe("the product's name", () => {
  it("is held together, so it never breaks across two lines", () => {
    expect(createTranslator("en")("onboarding.welcomeTitle")).toBe(
      "Welcome to Lyonoyl\u00a0Mail",
    );
    expect(createTranslator("ru")("onboarding.welcomeTitle")).toBe(
      "Добро пожаловать в Lyonoyl\u00a0Mail",
    );
  });

  it("is held together only in the catalogue's text, not in a value from the mail", () => {
    const said = createTranslator("en")("update.offered", {
      version: "Lyonoyl Mail 2",
      size: "1 MB",
    });
    expect(said).toBe(
      "A newer version of Lyonoyl\u00a0Mail is out — Lyonoyl Mail 2, 1 MB",
    );
  });
});

describe("createTranslator", () => {
  it("returns the text of the active language", () => {
    expect(createTranslator("en")("folder.inbox")).toBe("Inbox");
    expect(createTranslator("ru")("folder.inbox")).toBe("Входящие");
  });

  it("substitutes named values", () => {
    const t = createTranslator("en");
    expect(t("list.inConversation", { count: 3 })).toBe(
      "3 messages in this conversation",
    );
  });

  it("formats numbers for the active language rather than the OS", () => {
    const english = createTranslator("en")("list.inConversation", {
      count: 1234,
    });
    const russian = createTranslator("ru")("list.inConversation", {
      count: 1234,
    });

    expect(english).toContain("1,234");
    // Russian groups with a non-breaking space.
    expect(russian.replace(/\u00a0/g, " ")).toContain("1 234");
  });

  it("leaves an unfilled placeholder visible instead of blanking the sentence", () => {
    const t = createTranslator("en");
    expect(t("list.inConversation", { total: 3 })).toContain("{count}");
  });

  it("ignores values the string does not use", () => {
    const t = createTranslator("en");
    expect(t("folder.sent", { count: 5, whatever: "x" })).toBe("Sent");
  });

  it("selects the English plural forms", () => {
    const t = createTranslator("en");
    expect(t("list.inConversation", { count: 1 })).toBe(
      "1 message in this conversation",
    );
    expect(t("list.inConversation", { count: 2 })).toBe(
      "2 messages in this conversation",
    );
    expect(t("list.inConversation", { count: 0 })).toBe(
      "0 messages in this conversation",
    );
  });

  it("selects all three Russian plural forms", () => {
    // One, few and many are different words in Russian.
    const t = createTranslator("ru");
    expect(t("list.inConversation", { count: 1 })).toBe(
      "1 письмо в этой переписке",
    );
    expect(t("list.inConversation", { count: 2 })).toBe(
      "2 письма в этой переписке",
    );
    expect(t("list.inConversation", { count: 5 })).toBe(
      "5 писем в этой переписке",
    );
    expect(t("list.inConversation", { count: 21 })).toBe(
      "21 письмо в этой переписке",
    );
    // Eleven to fourteen take the many form although they end in one to four, and they are the
    // only counts that tell the rule from "look at the last digit".
    expect(t("list.inConversation", { count: 11 })).toBe(
      "11 писем в этой переписке",
    );
    expect(t("list.inConversation", { count: 12 })).toBe(
      "12 писем в этой переписке",
    );
  });

  it("falls back to the 'other' form when a caller forgets the count", () => {
    const t = createTranslator("en");
    expect(t("list.inConversation")).toBe("{count} messages in this conversation");
  });

  it("falls back to English when the active catalog lost a key", () => {
    restores.push(withoutKey("ru", "folder.drafts"));
    expect(createTranslator("ru")("folder.drafts")).toBe("Drafts");
  });

  it("returns the key itself when no catalog has the text", () => {
    restores.push(withoutKey("ru", "folder.drafts"));
    restores.push(withoutKey("en", "folder.drafts"));
    // A visible key names what is missing; an empty label does not.
    expect(createTranslator("ru")("folder.drafts")).toBe("folder.drafts");
  });
});

describe("a list of names", () => {
  it("is joined the way the language joins a list", () => {
    expect(listOf("en", ["zeta@example.com", "kappa@example.net"])).toBe(
      "zeta@example.com and kappa@example.net",
    );
  });

  it("isolates every name on its own in a language written right to left", () => {
    // Isolated as a whole, the list would take its direction from its first name, and a list that
    // opens with an address would run left to right with every conjunction beside the wrong name.
    expect(listOf("ar", ["zeta@example.com", "kappa@example.net"])).toBe(
      new Intl.ListFormat("ar").format([
        "\u2068zeta@example.com\u2069",
        "\u2068kappa@example.net\u2069",
      ]),
    );
  });

  it("adds nothing to a name in a language written left to right", () => {
    expect(listOf("ru", ["zeta@example.com"])).not.toMatch(/[\u2066-\u2069]/);
  });
});
