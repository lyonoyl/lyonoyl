import type { TranslationKey } from "@/shared/i18n";

/**
 * Every part of the settings, in the order they stand on the screen.
 *
 * Two things read this and must agree: the screen, which draws them, and the list beside it, which
 * names them and scrolls to them. Written twice they would drift — a heading renamed in one place
 * and not the other, or a part added to the screen that the list beside it does not know about.
 * `SettingsScreen.test.tsx` compares what the screen actually renders against this.
 */
export const SETTINGS_SECTIONS: ReadonlyArray<{
  id: string;
  title: TranslationKey;
}> = [
  // Before the mailboxes, because each mailbox row offers the groups made here.
  { id: "groups", title: "settings.groups" },
  { id: "accounts", title: "settings.accounts" },
  { id: "notifications", title: "settings.notifications" },
  { id: "storage", title: "settings.storage" },
  { id: "appearance", title: "settings.appearance" },
  { id: "reading", title: "settings.reading" },
  { id: "language", title: "settings.language" },
  { id: "updates", title: "settings.updates" },
  // Always last: a reference read once if ever, below everything a person comes here to change.
  // `SettingsSections.test.tsx` refuses anything placed after it.
  { id: "licences", title: "settings.licences" },
];

/** What the section with this name is called in the document, for scrolling to and highlighting. */
export function anchorFor(id: string): string {
  return `settings-${id}`;
}
