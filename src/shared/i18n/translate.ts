import {
  LANGUAGES,
  SOURCE_LANGUAGE,
  type CatalogEntry,
  type SupportedLanguage,
  type TranslationKey,
} from "./catalog";

/**
 * Turns a key into the text shown to the user.
 *
 * Values are plain strings rendered as React text nodes. Nothing here produces markup, and the
 * result is never passed to `dangerouslySetInnerHTML`.
 */

export type TranslationValues = Record<string, string | number>;

export type Translator = (
  key: TranslationKey,
  values?: TranslationValues,
) => string;

const PLACEHOLDER = /\{(\w+)\}/g;

const PRODUCT_NAME = /Lyonoyl Mail/g;
const UNBROKEN_PRODUCT_NAME = "Lyonoyl\u00a0Mail";

export function createTranslator(language: SupportedLanguage): Translator {
  // The registry is built from whatever files exist, so an unknown tag falls back to the source.
  const catalog = (LANGUAGES[language] ?? LANGUAGES[SOURCE_LANGUAGE]).catalog;
  const source = LANGUAGES[SOURCE_LANGUAGE].catalog;
  // Constructed once per language: `Intl` objects are expensive and the list re-renders often.
  const plural = new Intl.PluralRules(language);
  const number = new Intl.NumberFormat(language);
  // In a sentence written right to left, a value is isolated: a name, an address or a subject in
  // Latin letters keeps its own order, and the punctuation around it stays where the sentence put
  // it. Left to right, nothing is added.
  const isolated =
    (LANGUAGES[language] ?? LANGUAGES[SOURCE_LANGUAGE]).dir === "rtl";

  return (key, values) => {
    const entry: CatalogEntry | undefined = catalog[key] ?? source[key];
    if (entry === undefined) {
      // Reached only if a catalog drifted at runtime. The key names what is missing.
      return key;
    }

    const template =
      typeof entry === "string"
        ? entry
        : selectPluralForm(entry, values, plural);
    // The product's name is one word to the eye: it never breaks across two lines. Only the
    // catalogue's own text is touched, not a value from the mail.
    return interpolate(
      template.replace(PRODUCT_NAME, UNBROKEN_PRODUCT_NAME),
      values,
      number,
      isolated,
    );
  };
}

/**
 * Names joined the way the language joins a list: `A, B and C` in English, with `و` in Arabic.
 *
 * Right to left, each name is isolated before the list is joined. Isolated as a whole, the list
 * would take its direction from the first name, and one opening with a name in Latin letters would
 * run left to right with every conjunction beside the wrong name.
 */
export function listOf(
  language: SupportedLanguage,
  names: readonly string[],
): string {
  const isolated =
    (LANGUAGES[language] ?? LANGUAGES[SOURCE_LANGUAGE]).dir === "rtl";
  return new Intl.ListFormat(language).format(
    isolated ? names.map((name) => `\u2068${name}\u2069`) : names,
  );
}

function selectPluralForm(
  forms: Exclude<CatalogEntry, string>,
  values: TranslationValues | undefined,
  plural: Intl.PluralRules,
): string {
  const count = values?.count;
  if (typeof count !== "number") {
    // A plural entry without a count falls back to `other`.
    return forms.other;
  }
  return forms[plural.select(count)] ?? forms.other;
}

function interpolate(
  template: string,
  values: TranslationValues | undefined,
  number: Intl.NumberFormat,
  isolated: boolean,
): string {
  if (!values) return template;

  return template.replace(PLACEHOLDER, (token, name: string) => {
    const value = values[name];
    if (value === undefined) {
      // The placeholder stays visible rather than leaving a gap in the sentence.
      return token;
    }
    // Numbers follow the interface language: 1,234 in English, 1 234 in Russian.
    const written = typeof value === "number" ? number.format(value) : value;
    // First-strong isolate and its pop: invisible, and they read as nothing to a screen reader.
    return isolated ? `\u2068${written}\u2069` : written;
  });
}
