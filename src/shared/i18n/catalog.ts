import en from "./locales/en.json";

/**
 * The registry of interface languages.
 *
 * A language is added by dropping `<tag>.json` into `locales/`: the glob below picks it up and no
 * other place in the application names it.
 */

/** Plural categories as `Intl.PluralRules` names them. `other` is required and used as fallback. */
export interface PluralForms {
  zero?: string;
  one?: string;
  two?: string;
  few?: string;
  many?: string;
  other: string;
}

export type CatalogEntry = string | PluralForms;

/**
 * Keys come from English, the source language: a key used in code but absent from `en.json` fails
 * `npm run typecheck`. Keys starting with `$` are file metadata, not text.
 *
 * Completeness of a translation is checked in `catalog.test.ts`, since catalogs are discovered by
 * glob and the compiler does not see them.
 */
export type TranslationKey = Exclude<keyof typeof en, `$${string}`>;

export type Catalog = Record<TranslationKey, CatalogEntry>;

/**
 * A catalogue key the core handed over, if the catalogue actually holds it.
 *
 * Invariant 17: Rust carries a key rather than a sentence, and the window turns it into words. What
 * crosses the boundary is a plain string, so the compiler cannot vouch for it the way it does for
 * a key written here — this is where that gap is closed. A key nothing in the catalogue answers
 * gives nothing rather than the key itself: a person reading `account.needsAppPassword` on their
 * screen is worse off than a person reading nothing, and it is the kind of thing that ships.
 */
export function keyTheCoreSent(value: string | null): TranslationKey | null {
  if (value === null) return null;

  return Object.prototype.hasOwnProperty.call(en, value)
    ? (value as TranslationKey)
    : null;
}

export interface LanguageEntry {
  /** The language's name in itself, as shown in a language list. */
  readonly name: string;
  readonly dir: "ltr" | "rtl";
  readonly catalog: Catalog;
}

/** The language every fallback chain ends at. */
export const SOURCE_LANGUAGE = "en";

/** A language tag that has a catalog in this build. */
export type SupportedLanguage = string;

const FILES = import.meta.glob<Record<string, unknown>>("./locales/*.json", {
  eager: true,
  import: "default",
});

export const LANGUAGES: Readonly<Record<string, LanguageEntry>> =
  Object.fromEntries(
    Object.entries(FILES).map(([path, contents]) => [
      tagOf(path),
      read(path, contents),
    ]),
  );

export const SUPPORTED_LANGUAGES = Object.keys(LANGUAGES).sort();

/**
 * Every language, in the order a list of them is shown: alphabetically by the name each gives
 * itself.
 *
 * Unicode's root collation, the one no language tailors: names in Latin letters from A to Z, then
 * Cyrillic, Arabic, Devanagari, Bengali, Thai, Hangul and the Han characters, each script in its
 * own order. A person looking for their language finds it among the ones written like it.
 *
 * Asked of English, which applies the root order unchanged. `und` would say the same in principle,
 * and an engine resolves it to the system's own language instead — on a Russian system the
 * Cyrillic names would come first.
 */
const ROOT_ORDER = new Intl.Collator("en");

export const LANGUAGES_IN_LIST_ORDER: readonly SupportedLanguage[] = [
  ...SUPPORTED_LANGUAGES,
].sort((one, other) =>
  ROOT_ORDER.compare(LANGUAGES[one].name, LANGUAGES[other].name),
);

if (!Object.prototype.hasOwnProperty.call(LANGUAGES, SOURCE_LANGUAGE)) {
  // Every fallback chain ends here.
  throw new Error(
    `i18n: the source catalog locales/${SOURCE_LANGUAGE}.json is missing`,
  );
}

export function isSupported(tag: string): boolean {
  return Object.prototype.hasOwnProperty.call(LANGUAGES, tag);
}

function tagOf(path: string): string {
  // "./locales/pt-BR.json" → "pt-br"; storage and matching are lowercase.
  return path.slice(path.lastIndexOf("/") + 1, -".json".length).toLowerCase();
}

/**
 * Reads one catalog file. A malformed file throws at startup; the files are bundled at build time,
 * so this surfaces in `npm test` rather than on a user's machine.
 */
function read(path: string, contents: Record<string, unknown>): LanguageEntry {
  const name = contents.$name;
  if (typeof name !== "string" || name.trim() === "") {
    throw new Error(
      `i18n: ${path} must declare "$name" — the language's name in itself`,
    );
  }

  const dir = contents.$dir ?? "ltr";
  if (dir !== "ltr" && dir !== "rtl") {
    throw new Error(
      `i18n: ${path} declares "$dir": ${JSON.stringify(dir)}; expected "ltr" or "rtl"`,
    );
  }

  const catalog = Object.fromEntries(
    Object.entries(contents).filter(([key]) => !key.startsWith("$")),
  ) as Catalog;

  return { name, dir, catalog };
}
