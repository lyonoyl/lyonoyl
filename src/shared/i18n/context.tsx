import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { LANGUAGES, type SupportedLanguage } from "./catalog";
import { FOLLOW_SYSTEM, resolveLanguage, webViewLanguages } from "./resolve";
import { createTranslator, type Translator } from "./translate";

import type { LanguageDto } from "@/shared/ipc/generated/LanguageDto";

/**
 * Which language is active and how to change it.
 *
 * The provider does not read the preference itself: the shell requests preferences once on startup
 * and passes the stored value in through `setPreference`.
 */
interface I18n {
  readonly t: Translator;
  readonly language: SupportedLanguage;
  readonly dir: "ltr" | "rtl";
  readonly setPreference: (preference: LanguageDto) => void;
}

const I18nContext = createContext<I18n | null>(null);

export function I18nProvider({
  children,
  initial = FOLLOW_SYSTEM,
  askTheSystem,
}: {
  children: ReactNode;
  initial?: LanguageDto;
  /**
   * Where the person's languages come from, in the order they set them. The web view reports the
   * first and nothing after it; without the whole list a person whose first language has no
   * catalogue is read English rather than their second.
   */
  askTheSystem?: () => Promise<readonly string[]>;
}) {
  // Until preferences arrive, follow the OS.
  const [preference, setPreference] = useState<LanguageDto>(initial);
  // What the web view reports until the system's own list arrives, and then that list in front.
  const [system, setSystem] = useState<readonly string[]>(webViewLanguages);
  useEffect(() => {
    if (!askTheSystem) return;
    let stillHere = true;
    askTheSystem()
      .then((listed) => {
        if (stillHere) setSystem([...listed, ...webViewLanguages()]);
      })
      .catch(() => {
        /* The web view's answer stands. */
      });
    return () => {
      stillHere = false;
    };
  }, [askTheSystem]);
  const language = useMemo(
    () => resolveLanguage(preference, system),
    [preference, system],
  );

  const value = useMemo<I18n>(
    () => ({
      t: createTranslator(language),
      language,
      dir: LANGUAGES[language]?.dir ?? "ltr",
      setPreference,
    }),
    [language],
  );

  // Assistive technology picks its voice from `lang`, and layout direction is read from `dir`.
  // Before the browser paints, so a mirrored window is never drawn the other way round first.
  useLayoutEffect(() => {
    document.documentElement.lang = language;
    document.documentElement.dir = LANGUAGES[language]?.dir ?? "ltr";
  }, [language]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18n {
  const value = useContext(I18nContext);
  if (!value) {
    throw new Error("useI18n was called outside <I18nProvider>");
  }
  return value;
}

/** For components that need text and nothing else. */
export function useTranslation(): Translator {
  return useI18n().t;
}
