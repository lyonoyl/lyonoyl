/**
 * Interface text.
 *
 * Components import from here rather than from the modules below it.
 */
export {
  LANGUAGES,
  LANGUAGES_IN_LIST_ORDER,
  SOURCE_LANGUAGE,
  SUPPORTED_LANGUAGES,
  isSupported,
  keyTheCoreSent,
  type Catalog,
  type LanguageEntry,
  type PluralForms,
  type SupportedLanguage,
  type TranslationKey,
} from "./catalog";
export { I18nProvider, useI18n, useTranslation } from "./context";
export { ERROR_MESSAGE_KEYS, describeError } from "./errors";
export { FOLLOW_SYSTEM, resolveLanguage, webViewLanguages } from "./resolve";
export {
  createTranslator,
  listOf,
  type Translator,
  type TranslationValues,
} from "./translate";
