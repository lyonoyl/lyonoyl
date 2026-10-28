import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  LANGUAGES,
  LANGUAGES_IN_LIST_ORDER,
  SOURCE_LANGUAGE,
  SUPPORTED_LANGUAGES,
  type Catalog,
  type PluralForms,
} from "./catalog";

/**
 * The compiler checks that code uses only keys the source catalog has. Translations are discovered
 * by glob and are checked here instead: a missing key, an extra key, blank text or a plural form
 * the language selects but the catalog omits fails the test run.
 */

const LOCALES_DIR = join(__dirname, "locales");

const sourceKeys = Object.keys(LANGUAGES[SOURCE_LANGUAGE].catalog).sort();

/** Flattens a catalog to `key` / `key.pluralForm` pairs, so plural forms are checked too. */
function texts(catalog: Catalog): Array<[string, unknown]> {
  const flat: Array<[string, unknown]> = [];
  for (const [key, entry] of Object.entries(catalog)) {
    // Anything but text or a set of forms is named as it is, for the blank-text test to report.
    if (typeof entry !== "object" || entry === null) {
      flat.push([key, entry]);
      continue;
    }
    for (const [form, text] of Object.entries(entry)) {
      flat.push([`${key}.${form}`, text]);
    }
  }
  return flat;
}

/** The entries that carry plural forms, already narrowed away from plain strings. */
function pluralEntries(catalog: Catalog): Array<[string, PluralForms]> {
  const plurals: Array<[string, PluralForms]> = [];
  for (const [key, entry] of Object.entries(catalog)) {
    if (typeof entry === "object" && entry !== null) plurals.push([key, entry]);
  }
  return plurals;
}

/** The placeholders a text names, as `{name}` writes them. */
function placeholdersIn(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((found) => found[1]).sort();
}

/** The same, for a whole entry: every placeholder any of its forms names. */
function placeholdersOf(entry: Catalog[keyof Catalog] | undefined): string[] {
  if (entry === undefined || entry === null) return [];
  const forms = typeof entry === "string" ? [entry] : Object.values(entry);
  return [
    ...new Set(forms.flatMap((text) => placeholdersIn(text ?? ""))),
  ].sort();
}

/**
 * The whole numbers up to a few hundred that a plural category selects in a language.
 *
 * A form standing for more than one of them has to say which: Russian `one` is 1, 21, 31 and 101,
 * and French and Hindi `one` is 0 and 1, so "One message came back" written there is said to
 * twenty-one messages too.
 */
function numbersIn(language: string, category: string): number[] {
  const rules = new Intl.PluralRules(language);
  // The small numbers, and the round millions French, Spanish and Portuguese count as `many`.
  const sample = [
    ...Array.from({ length: 301 }, (_, at) => at),
    1_000,
    10_000,
    100_000,
    1_000_000,
    2_000_000,
    1_000_000_000,
    2_000_000_000,
  ];
  return sample.filter((count) => rules.select(count) === category);
}

/** Keys whose text is the same in every language: names nobody translates. */
const SAME_IN_EVERY_LANGUAGE = new Set([
  "account.encryptionStartTls",
  "account.encryptionTls",
  "account.hostExample",
  "settings.schemeCatppuccinLatte",
  "settings.schemeDracula",
  "settings.schemeGruvbox",
  "settings.schemeNord",
  "settings.schemeOneDark",
  "settings.schemeRosePineDawn",
  "settings.schemeSolarized",
]);

/**
 * The names a keyboard prints on its keys, where a language's own words are shown everywhere but on
 * a Mac. Keyboards print them in Latin letters whatever the script, so a language written in other
 * letters may keep them as English does. A language written in Latin letters keeps one only where
 * an arbiter wrote down in `SAME_AS_ENGLISH` that its keyboards print that word — German keyboards
 * print Strg and Entf, French ones Suppr and Échap.
 */
const KEYBOARD_LABELS = new Set([
  "keys.nameEnter",
  "keys.nameDelete",
  "keys.nameEscape",
  "keys.nameControl",
  "keys.nameShift",
]);

/**
 * Keys a language writes exactly as English does, because the word is the same in it. Each entry
 * is a decision an arbiter made about that language, not a gap: a key that is merely untranslated
 * fails the test below. Keyed by the tag as the registry has it — lowercase, `pt-br`.
 */
const SAME_AS_ENGLISH: Readonly<Record<string, readonly string[]>> = {
  de: [
    "account.port",
    "settings.updates",
    "keys.nameEnter",
    "keys.nameEscape",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
  es: [
    "compose.forwardedCc",
    "keys.nameEnter",
    "keys.nameEscape",
    "keys.nameControl",
    "size.b",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
  fr: [
    "account.port",
    "compose.body",
    "list.label",
    "reading.asSource",
    "settings.licences",
    "settings.notifications",
    "settings.schemeGraphite",
    "keys.nameControl",
  ],
  id: [
    "account.port",
    "keys.nameEnter",
    "keys.nameDelete",
    "keys.nameEscape",
    "keys.nameControl",
    "keys.nameShift",
    "size.b",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
  "pt-br": [
    "compose.forwardedCc",
    "keys.nameEnter",
    "keys.nameDelete",
    "keys.nameEscape",
    "keys.nameControl",
    "keys.nameShift",
    "size.b",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
  tr: [
    "account.port",
    "keys.nameEnter",
    "keys.nameDelete",
    "keys.nameEscape",
    "keys.nameControl",
    "keys.nameShift",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
  vi: [
    "compose.forwardedCc",
    "keys.nameEnter",
    "keys.nameDelete",
    "keys.nameEscape",
    "keys.nameControl",
    "keys.nameShift",
    "size.b",
    "size.kb",
    "size.mb",
    "size.gb",
    "size.tb",
    "size.pb",
  ],
};

/**
 * The units a size is written in. B, kB, MB and the rest are international symbols, written in
 * Latin letters in every script, so a language written in other letters may keep them. A language
 * written in Latin letters keeps one only where an arbiter wrote it down, since some write their
 * own — French ko and Mo.
 */
const UNIT_SYMBOLS = new Set([
  "size.b",
  "size.kb",
  "size.mb",
  "size.gb",
  "size.tb",
  "size.pb",
]);

/**
 * Words that stay in Latin letters in every script: the product, the protocols, the systems and
 * the services a person sees by those names, and the units a size is written in.
 */
const LATIN_EVERYWHERE = new Set([
  "Lyonoyl",
  "Mail",
  "GitHub",
  "IMAP",
  "SMTP",
  "TLS",
  "STARTTLS",
  "SSL",
  "OAuth",
  "macOS",
  "Windows",
  "Linux",
  "Gmail",
  "iCloud",
  "Outlook",
  "Fastmail",
  "Proton",
  "Bridge",
  "Cc",
  "Bcc",
  "B",
  "kB",
  "KB",
  "MB",
  "GB",
  "TB",
  "PB",
]);

/** Which letters a language is written in, as Unicode names the script. */
function scriptPattern(language: string): RegExp {
  const script = new Intl.Locale(language).maximize().script;
  switch (script) {
    case "Jpan":
      return /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
    case "Kore":
      return /[\p{Script=Hangul}\p{Script=Han}]/u;
    case "Hans":
    case "Hant":
      return /\p{Script=Han}/u;
    default:
      return new RegExp(`\\p{Script=${script}}`, "u");
  }
}

/**
 * Whether a line of a language not written in Latin letters is in its own script: no Latin word
 * but the names everybody writes in Latin, and letters of its own wherever it has letters besides.
 * Placeholders, web addresses and a scheme written out are not words.
 */
function inItsOwnLetters(text: string, own: RegExp): boolean {
  // Only the characters an address is made of: a script written without spaces runs straight on
  // from one, and taking everything up to the next space would take the sentence with it.
  const bare = text
    .replace(/\{\w+\}/g, " ")
    .replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+/g, " ")
    .replace(
      /(?:\b[a-z][a-z0-9+.-]*:\/\/)?\b[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+(?:\/[A-Za-z0-9._~/-]*)?/gi,
      " ",
    )
    .replace(/\b[a-z][a-z0-9+.-]*:\/\//gi, " ");
  const words = bare.match(/[A-Za-z]+/g) ?? [];
  if (words.some((word) => !LATIN_EVERYWHERE.has(word))) return false;
  const rest = words.reduce((left, word) => left.replace(word, " "), bare);
  return !/\p{L}/u.test(rest) || own.test(rest);
}

/** A language written in Latin letters, where an English word can be a word of its own. */
function writtenInLatin(language: string): boolean {
  return new Intl.Locale(language).maximize().script === "Latn";
}

/**
 * Every line of code that could name a catalogue key, as one string.
 *
 * **Both halves of the application.** The core names a key where it has to say *which* sentence
 * without saying the sentence — a provider preset's note about app passwords is chosen in Rust and
 * read here — so a scan of the window alone reports those as text nobody displays. That is a key
 * being deleted for being used from the other side.
 *
 * The catalogue files are left out: a key is defined there, and finding it there would make every
 * key look used. Tests are left out too — a key kept alive only by a test that asserts on it is a
 * key nothing shows.
 */
function readEverySourceFile(): string {
  const read = (where: string): string[] =>
    readdirSync(where, { withFileTypes: true }).flatMap((entry) => {
      const at = join(where, entry.name);
      if (entry.isDirectory()) {
        return entry.name === "node_modules" || entry.name === "target"
          ? []
          : read(at);
      }
      if (!/\.(tsx?|rs)$/.test(entry.name)) return [];
      if (entry.name.includes(".test.")) return [];
      if (at.includes("locales")) return [];
      return [readFileSync(at, "utf8")];
    });

  return [...read("src"), ...read("src-tauri/src"), ...read("crates")].join(
    "\n",
  );
}

describe("catalogs", () => {
  it("registers exactly the language files on disk, with no code change per language", () => {
    const onDisk = readdirSync(LOCALES_DIR)
      .filter((file) => file.endsWith(".json"))
      .map((file) => file.slice(0, -".json".length).toLowerCase())
      .sort();

    // One file is all a language needs.
    expect(SUPPORTED_LANGUAGES).toEqual(onDisk);
  });

  it("keeps the source language, where every fallback chain ends", () => {
    expect(SUPPORTED_LANGUAGES).toContain(SOURCE_LANGUAGE);
  });

  it.each(SUPPORTED_LANGUAGES)(
    "%s has exactly the keys of the source language",
    (language) => {
      // An extra key is text nobody displays and a translator keeps maintaining.
      expect(Object.keys(LANGUAGES[language].catalog).sort()).toEqual(
        sourceKeys,
      );
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s writes a count in forms wherever English does, and only there",
    (language) => {
      // A count written as one line passes every rule about forms -- it has none to check -- and
      // reads the same to one message and to five.
      const source = LANGUAGES[SOURCE_LANGUAGE].catalog;
      const reshaped = Object.entries(LANGUAGES[language].catalog)
        .filter(
          ([key, entry]) =>
            (typeof entry === "string") !==
            (typeof source[key as keyof Catalog] === "string"),
        )
        .map(([key]) => key);

      expect(reshaped).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s keeps file metadata out of the text",
    (language) => {
      const leaked = Object.keys(LANGUAGES[language].catalog).filter((key) =>
        key.startsWith("$"),
      );
      expect(leaked).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)("%s has no blank text", (language) => {
    const blanks = texts(LANGUAGES[language].catalog)
      .filter(([, text]) => typeof text !== "string" || text.trim() === "")
      .map(([key]) => key);

    expect(blanks).toEqual([]);
  });

  it.each(SUPPORTED_LANGUAGES)(
    "%s ends its sentences without a full stop",
    (language) => {
      // Interface text is not prose. A line that ends in a full stop reads as being told something
      // rather than as being shown it, and next to a file name that already ends in one — "Saved as
      // report.eml." — it reads as a typo. The stop between two sentences stays; the one at the very
      // end goes, in every script's own full stop: the Latin one, the ideographic one, the danda
      // and the Urdu one.
      const stopped = texts(LANGUAGES[language].catalog)
        .filter(
          ([, text]) => typeof text === "string" && /[.。．｡।॥۔]$/u.test(text),
        )
        .map(([key]) => key);

      expect(stopped).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)("%s never exclaims", (language) => {
    // The interface says what happened and what can be done about it, in an even voice, in every
    // language: an exclamation is a raised voice, and in a mail client it is usually an alarm
    // about nothing.
    const raised = texts(LANGUAGES[language].catalog)
      .filter(([, text]) => typeof text === "string" && /[!！¡]/u.test(text))
      .map(([key]) => key);

    expect(raised).toEqual([]);
  });

  it.each(SUPPORTED_LANGUAGES)(
    "%s keeps every placeholder of the source and adds none",
    (language) => {
      // A placeholder translated, dropped or invented is a sentence that shows `{count}` to a
      // person, or loses the name it was about.
      const source = LANGUAGES[SOURCE_LANGUAGE].catalog;
      const differing = Object.entries(LANGUAGES[language].catalog).flatMap(
        ([key, entry]) => {
          const wanted = placeholdersOf(source[key as keyof Catalog]);
          // A brace that is not a placeholder the source names is shown to a person as it is:
          // `{عدد}` or `{ count }` is text, and nothing fills it in.
          const forms =
            typeof entry === "string" ? [entry] : Object.values(entry);
          const strange = forms.some((text) =>
            (String(text).match(/\{[^{}]*\}/g) ?? []).some((brace) => {
              const name = /^\{(\w+)\}$/.exec(brace)?.[1];
              return name === undefined || !wanted.includes(name);
            }),
          );
          if (strange) return [key];
          if (typeof entry === "string") {
            // Each placeholder at least once, however often a language needs to say it.
            const named = [...new Set(placeholdersIn(entry))].sort();
            return named.join() === wanted.join() ? [] : [key];
          }
          // A plural form may leave the count out where its category is one number; it may not
          // leave out anything else, or name anything the source does not.
          return Object.entries(entry).flatMap(([form, text]) => {
            const named = placeholdersIn(text ?? "");
            const extra = named.filter((name) => !wanted.includes(name));
            const missing = wanted.filter(
              (name) => name !== "count" && !named.includes(name),
            );
            return extra.length > 0 || missing.length > 0
              ? [`${key}.${form}`]
              : [];
          });
        },
      );

      expect(differing).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s keeps every web address and host name of the source letter for letter",
    (language) => {
      // An address a person is told to open, or a host they are shown as an example, is the same
      // in every language; one changed by a letter is a page that does not exist.
      const source = LANGUAGES[SOURCE_LANGUAGE].catalog;
      const addressesIn = (text: string) =>
        text.match(
          /\b(?:https?:\/\/)?[a-z0-9-]+(?:\.[a-z0-9-]+)+(?:\/[\w./-]*)?/gi,
        ) ?? [];
      const changed = Object.entries(LANGUAGES[language].catalog).flatMap(
        ([key, entry]) => {
          const english = source[key as keyof Catalog];
          if (typeof english !== "string" || typeof entry !== "string")
            return [];
          return addressesIn(english)
            .filter((address) => !entry.includes(address))
            .map((address) => `${key}: ${address}`);
        },
      );

      expect(changed).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s says the number wherever a plural form stands for more than one",
    (language) => {
      const silent = pluralEntries(LANGUAGES[language].catalog).flatMap(
        ([key, forms]) =>
          Object.entries(forms)
            .filter(
              ([form, text]) =>
                numbersIn(language, form).length > 1 &&
                !placeholdersIn(text ?? "").includes("count"),
            )
            .map(([form]) => `${key}.${form}`),
      );

      expect(silent).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s carries no plural form its language never selects",
    (language) => {
      // A form nothing selects is text a translator keeps in step for nobody.
      const categories = new Intl.PluralRules(language).resolvedOptions()
        .pluralCategories as readonly string[];
      const unused = pluralEntries(LANGUAGES[language].catalog).flatMap(
        ([key, forms]) =>
          Object.keys(forms)
            .filter((form) => !categories.includes(form))
            .map((form) => `${key}.${form}`),
      );

      expect(unused).toEqual([]);
    },
  );

  it.each(
    SUPPORTED_LANGUAGES.filter((language) => language !== SOURCE_LANGUAGE),
  )(
    "%s is written in its own letters, with nothing left in English",
    (language) => {
      const source = LANGUAGES[SOURCE_LANGUAGE].catalog;
      const own = scriptPattern(language);
      const allowed = new Set(SAME_AS_ENGLISH[language] ?? []);
      const latin = writtenInLatin(language);

      const leftover = Object.entries(LANGUAGES[language].catalog).flatMap(
        ([key, entry]) => {
          if (SAME_IN_EVERY_LANGUAGE.has(key)) return [];
          if (!latin && (KEYBOARD_LABELS.has(key) || UNIT_SYMBOLS.has(key)))
            return [];
          const english = source[key as keyof Catalog];
          // Each text with the English it stands for: the whole string, or the same plural form.
          const pairs: Array<[string, string, string | undefined]> =
            typeof entry === "string"
              ? [
                  [
                    key,
                    entry,
                    typeof english === "string" ? english : undefined,
                  ],
                ]
              : Object.entries(entry).map(([form, text]) => [
                  `${key}.${form}`,
                  text ?? "",
                  typeof english === "string"
                    ? undefined
                    : english[form as keyof PluralForms],
                ]);

          return pairs.flatMap(([where, text, same]) => {
            // In Latin letters an English word can be a word of this language too, so only a whole
            // line the same as English is caught, and an arbiter's word decides it.
            if (latin) return same === text && !allowed.has(key) ? [where] : [];
            return inItsOwnLetters(text, own) ? [] : [where];
          });
        },
      );

      expect(leftover).toEqual([]);
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s covers every plural form its language selects",
    (language) => {
      const categories = new Intl.PluralRules(language).resolvedOptions()
        .pluralCategories;

      const missing = pluralEntries(LANGUAGES[language].catalog).flatMap(
        ([key, forms]) =>
          categories
            .filter((category) => !(category in forms))
            .map((category) => `${key}.${category}`),
      );

      // Russian selects one/few/many/other; a catalog with only one/other reads wrong for most
      // numbers.
      expect(missing).toEqual([]);
    },
  );

  it("has no text nobody displays", () => {
    // A key nobody uses is a sentence a translator keeps in step with a product that never shows
    // it. The other direction — a key used and missing — is already a compile error, because
    // `TranslationKey` is the catalogue's own keys; this is the direction the compiler cannot see.
    //
    // Matched with its quotes on both sides, so `shell.collapse` is not counted as used because
    // `shell.collapseNamed` exists. Both of those were dead and both looked alive.
    const source = readEverySourceFile();

    const unused = Object.keys(LANGUAGES[SOURCE_LANGUAGE].catalog).filter(
      (key) => !source.includes(`"${key}"`),
    );

    expect(unused).toEqual([]);
  });

  it.each(SUPPORTED_LANGUAGES)(
    "%s names itself in its own language",
    (language) => {
      const name = LANGUAGES[language].name;
      // The name the language gives itself, as the Unicode locale data has it — a list of
      // languages written in one of them is a list a person who cannot read that one cannot use.
      // Bahasa Indonesia is the name the language goes by in a list of languages; the data has the
      // country's.
      const own =
        OWN_NAMES[language] ??
        new Intl.DisplayNames([language], { type: "language" }).of(language);
      expect(name.toLocaleLowerCase(language)).toBe(
        own?.toLocaleLowerCase(language),
      );
      // Capitalised where its letters have case, so the list reads as a list.
      const first = name.charAt(0);
      expect(first).toBe(first.toLocaleUpperCase(language));
    },
  );

  it.each(SUPPORTED_LANGUAGES)(
    "%s runs in the direction its script does",
    (language) => {
      const locale = new Intl.Locale(language) as Intl.Locale & {
        textInfo?: { direction: string };
        getTextInfo?: () => { direction: string };
      };
      const info = locale.getTextInfo?.() ?? locale.textInfo;
      expect(LANGUAGES[language].dir).toBe(info?.direction);
    },
  );

  it("lists the languages alphabetically by the names they give themselves", () => {
    // Latin names first and from A to Z, then the other scripts in Unicode's root order, each
    // script's names in their own order.
    // Written out rather than computed: a check that sorted with the same collator as the code
    // would prove only that the code agrees with itself.
    const names = LANGUAGES_IN_LIST_ORDER.map(
      (language) => LANGUAGES[language].name,
    );
    const literally = [
      "Bahasa Indonesia",
      "Deutsch",
      "English",
      "Español",
      "Français",
      "Português (Brasil)",
      "Tiếng Việt",
      "Türkçe",
      "Русский",
      "اردو",
      "العربية",
      "हिन्दी",
      "বাংলা",
      "ไทย",
      "한국어",
      "日本語",
      "简体中文",
    ];
    expect(names).toEqual(literally.filter((name) => names.includes(name)));
    expect(names.every((name) => literally.includes(name))).toBe(true);
    expect([...LANGUAGES_IN_LIST_ORDER].sort()).toEqual(SUPPORTED_LANGUAGES);
    const latin = names.map((name) => /^\p{Script=Latin}/u.test(name));
    expect(
      latin.indexOf(false) === -1 ||
        !latin.slice(latin.indexOf(false)).includes(true),
    ).toBe(true);
  });
});

/** A language whose name in a list differs from the one the locale data gives it, keyed as the registry keys it. */
const OWN_NAMES: Readonly<Record<string, string>> = {
  id: "Bahasa Indonesia",
};
