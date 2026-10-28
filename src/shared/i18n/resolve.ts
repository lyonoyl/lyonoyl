import {
  SOURCE_LANGUAGE,
  SUPPORTED_LANGUAGES,
  type SupportedLanguage,
} from "./catalog";

import type { LanguageDto } from "@/shared/ipc/generated/LanguageDto";

/** The stored value meaning "follow the OS". */
export const FOLLOW_SYSTEM = "system";

/**
 * Picks the catalog to render with.
 *
 * The stored preference may name a language this build does not ship, so resolution falls through
 * to the OS languages and then to English instead of failing. `supported` is every catalog this
 * build has, and is a parameter so the matching can be asked about catalogs it does not ship.
 */
export function resolveLanguage(
  preference: LanguageDto,
  systemPreferences: readonly string[],
  supported: readonly SupportedLanguage[] = SUPPORTED_LANGUAGES,
): SupportedLanguage {
  if (preference !== FOLLOW_SYSTEM) {
    const chosen = matchCatalog(preference, supported);
    if (chosen) return chosen;
  }

  for (const candidate of systemPreferences) {
    const matched = matchCatalog(candidate, supported);
    if (matched) return matched;
  }

  return SOURCE_LANGUAGE;
}

/**
 * The catalog for a tag: the tag itself, or one in the same language written in the same script.
 *
 * Script before region, because the script is what a person can read. `pt-PT` takes a `pt-BR`
 * catalog — the same language in the same letters, with some words chosen differently — and
 * `en-GB` takes `en`. `zh-TW` does not take `zh-Hans`: Traditional and Simplified Chinese are
 * written in different characters, and a person who reads one is not served by being handed the
 * other when the next language on their list may be one they chose. The script is the likely one
 * where a tag leaves it out, as `Intl.Locale` fills it in: `zh` and `zh-CN` are Simplified,
 * `zh-HK` is Traditional.
 */
function matchCatalog(
  tag: string,
  supported: readonly SupportedLanguage[],
): SupportedLanguage | null {
  const normalized = tag.toLowerCase();
  if (supported.includes(normalized)) return normalized;

  const wanted = likely(normalized);
  if (!wanted) return null;

  const readable = supported.filter((catalog) => {
    const offered = likely(catalog);
    return (
      offered !== null &&
      offered.language === wanted.language &&
      offered.script === wanted.script
    );
  });

  // The catalog for the person's own region where there is one, and otherwise the first.
  return (
    readable.find((catalog) => likely(catalog)?.region === wanted.region) ??
    readable[0] ??
    null
  );
}

/** Language, script and region as `Intl.Locale` fills them in, or `null` for a tag that is not one. */
function likely(
  tag: string,
): { language: string; script?: string; region?: string } | null {
  try {
    const full = new Intl.Locale(tag).maximize();
    // Nastaliq is a way of writing the Arabic script, and some engines fill in its own code for
    // Urdu: `ur` comes out `ur-Aran`, while a system reporting `ur-Arab-PK` means the same letters.
    const script = full.script === "Aran" ? "Arab" : full.script;
    return { language: full.language, script, region: full.region };
  } catch {
    return null;
  }
}

/**
 * What the web view reports, in preference order. Empty in a non-DOM environment.
 *
 * On macOS that is the person's first language and nothing after it, so it is where resolution
 * starts rather than all it has: the core reads the whole list from the system, and the provider
 * puts that in front once it arrives.
 */
export function webViewLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages ?? [navigator.language];
}
