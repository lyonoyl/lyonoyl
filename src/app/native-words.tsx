import { useEffect } from "react";

import { useI18n } from "@/shared/i18n";
import { setMenuWords, setNotificationWords } from "@/shared/ipc/client";

/**
 * Hands over the words the core shows by itself, in the language this window is showing: what a
 * notification says, and what the application menu's settings item says.
 *
 * Renders nothing. It exists because both are drawn by the system on the core's behalf — a
 * notification at a moment this window may be asleep, a menu that belongs to the system — and Rust
 * holds no interface text: every user-facing string lives in one catalogue. So the words are
 * handed over when the window starts and again whenever the language changes, and until they have
 * been, nothing is shown at all.
 *
 * Mounted beside the application rather than inside it, so that a shell which fails to render — a
 * database that will not open — still leaves the core able to say that mail arrived, and the menu
 * able to open the settings.
 */
export function NativeWords() {
  const { t, language } = useI18n();

  useEffect(() => {
    void setNotificationWords(t("notify.newMail")).catch(() => {
      /* Nothing to say about it here. The core simply stays silent, which is the safe answer. */
    });
    void setMenuWords(t("menu.settings")).catch(() => {
      /* The menu goes without the item, and the window's own keys still open the settings. */
    });
  }, [t, language]);

  return null;
}
