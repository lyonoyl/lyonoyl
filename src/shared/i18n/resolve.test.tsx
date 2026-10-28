import { act, render, screen, waitFor } from "@testing-library/react";
import { useEffect } from "react";
import { describe, expect, it } from "vitest";

import { isSupported } from "./catalog";
import { I18nProvider, useI18n } from "./context";
import { FOLLOW_SYSTEM, resolveLanguage } from "./resolve";

/** `zz` is unassigned in ISO 639, so it never gains a catalog. */
const NO_CATALOG = "zz-ZZ";

/**
 * Catalogs named outright, so each rule is asked about the languages it is about whatever this
 * build happens to ship.
 */
const SHIPPED = ["en", "ru", "pt-br", "zh-hans", "es"];

describe("resolveLanguage", () => {
  it("uses a tag no catalog will ever claim", () => {
    expect(isSupported("zz")).toBe(false);
  });

  it("honours an explicit choice", () => {
    expect(resolveLanguage("ru", ["en-US"], SHIPPED)).toBe("ru");
  });

  it("matches a regional tag to its base catalog", () => {
    expect(resolveLanguage("ru-RU", ["en-US"], SHIPPED)).toBe("ru");
    expect(resolveLanguage(FOLLOW_SYSTEM, ["en-GE"], SHIPPED)).toBe("en");
  });

  it("ignores case, because tags are matched case-insensitively", () => {
    expect(resolveLanguage("RU", [], SHIPPED)).toBe("ru");
    expect(resolveLanguage("PT-br", [], SHIPPED)).toBe("pt-br");
  });

  it("follows the OS preference order when set to system", () => {
    expect(resolveLanguage(FOLLOW_SYSTEM, ["ru-RU", "en-US"], SHIPPED)).toBe(
      "ru",
    );
    expect(resolveLanguage(FOLLOW_SYSTEM, ["en-US", "ru-RU"], SHIPPED)).toBe(
      "en",
    );
  });

  it("reads a person their second language when the first has no catalog", () => {
    // Catalan first and Spanish second: Spanish, not English.
    expect(resolveLanguage(FOLLOW_SYSTEM, ["ca-ES", "es-ES"], SHIPPED)).toBe(
      "es",
    );
    expect(resolveLanguage(FOLLOW_SYSTEM, [NO_CATALOG, "ru-RU"], SHIPPED)).toBe(
      "ru",
    );
  });

  it("gives a region the catalog for the same language in the same script", () => {
    // The same language in the same letters, some words chosen differently.
    expect(resolveLanguage(FOLLOW_SYSTEM, ["pt-PT"], SHIPPED)).toBe("pt-br");
    expect(resolveLanguage(FOLLOW_SYSTEM, ["pt"], SHIPPED)).toBe("pt-br");
    expect(resolveLanguage(FOLLOW_SYSTEM, ["es-419"], SHIPPED)).toBe("es");
  });

  it("gives Simplified Chinese to whoever writes it, by tag or by likely script", () => {
    for (const simplified of [
      "zh",
      "zh-CN",
      "zh-SG",
      "zh-Hans",
      "zh-Hans-HK",
    ]) {
      expect(
        resolveLanguage(FOLLOW_SYSTEM, [simplified], SHIPPED),
        simplified,
      ).toBe("zh-hans");
    }
  });

  it("does not hand Simplified characters to somebody who reads Traditional ones", () => {
    // Their next language is one they chose; Simplified is not.
    for (const traditional of [
      "zh-TW",
      "zh-HK",
      "zh-MO",
      "zh-Hant",
      "zh-Hant-CN",
    ]) {
      expect(
        resolveLanguage(FOLLOW_SYSTEM, [traditional, "ru-RU"], SHIPPED),
        traditional,
      ).toBe("ru");
    }
  });

  it("falls through to the OS when the stored choice has no catalog in this build", () => {
    // The core stores any well-formed tag, and the OS language is closer than English.
    expect(resolveLanguage(NO_CATALOG, ["ru-RU"], SHIPPED)).toBe("ru");
  });

  it("ends at English rather than failing", () => {
    expect(resolveLanguage(NO_CATALOG, [NO_CATALOG], SHIPPED)).toBe("en");
    expect(resolveLanguage(FOLLOW_SYSTEM, [], SHIPPED)).toBe("en");
  });

  it("does not crash on values that are not tags at all", () => {
    expect(resolveLanguage("", [], SHIPPED)).toBe("en");
    expect(resolveLanguage("../en", [], SHIPPED)).toBe("en");
    expect(resolveLanguage("en.json", [], SHIPPED)).toBe("en");
    expect(
      resolveLanguage(FOLLOW_SYSTEM, ["not a tag", "-", "ru"], SHIPPED),
    ).toBe("ru");
  });
});

/** Shows which language the provider settled on. */
function Settled() {
  return <span data-testid="language">{useI18n().language}</span>;
}

/**
 * Lets an answer the provider is waiting on arrive, and what it changes be drawn, before anything is
 * read: a test that reads first is reading the window from before the answer, whatever it says.
 */
async function settled(answer: Promise<unknown>) {
  await act(async () => {
    await answer.catch(() => {});
  });
}

describe("the provider", () => {
  it("reads the system's whole list once the core has it, ahead of the web view's one language", async () => {
    // The web view in a test reports English, as it does on a Mac whose first language is
    // English; the system's list says the person reads Russian after a language with no catalog.
    render(
      <I18nProvider askTheSystem={() => Promise.resolve([NO_CATALOG, "ru-RU"])}>
        <Settled />
      </I18nProvider>,
    );

    await waitFor(() =>
      expect(screen.getByTestId("language").textContent).toBe("ru"),
    );
  });

  it("turns the whole window round for a language written right to left, and names it", () => {
    document.documentElement.dir = "";
    document.documentElement.lang = "";
    let painted = { dir: "", lang: "" };
    // A child's own effect runs after the page is painted and before its parent's: what it reads is
    // what the first frame showed.
    function FirstFrame() {
      useEffect(() => {
        painted = {
          dir: document.documentElement.dir,
          lang: document.documentElement.lang,
        };
      }, []);
      return null;
    }

    render(
      <I18nProvider initial="ar">
        <FirstFrame />
      </I18nProvider>,
    );

    // Set before the browser paints, so the window is never drawn the other way round first.
    expect(painted).toEqual({ dir: "rtl", lang: "ar" });
  });

  it("keeps the web view's answer when the system cannot be asked", async () => {
    // A web view reporting a language other than English, so keeping its answer and falling back
    // to English are two different results.
    Object.defineProperty(navigator, "languages", {
      configurable: true,
      get: () => ["ru-RU"],
    });
    try {
      const refused = Promise.reject(new Error("no system here"));
      render(
        <I18nProvider askTheSystem={() => refused}>
          <Settled />
        </I18nProvider>,
      );

      await settled(refused);
      expect(screen.getByTestId("language").textContent).toBe("ru");
    } finally {
      delete (navigator as { languages?: unknown }).languages;
    }
  });

  it("puts a stored choice ahead of anything the system says", async () => {
    const answer = Promise.resolve(["en-US"]);
    render(
      <I18nProvider initial="ru" askTheSystem={() => answer}>
        <Settled />
      </I18nProvider>,
    );

    await settled(answer);
    expect(screen.getByTestId("language").textContent).toBe("ru");
  });
});
