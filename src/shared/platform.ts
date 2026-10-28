import type { TranslationKey } from "@/shared/i18n";

/** Which system this window is running on, as far as it can tell from here. */
export type System = "mac" | "windows" | "linux";

/**
 * Read from the user agent, which is the one thing available before anything else loads.
 *
 * Being wrong about it costs the name of one thing in one sentence, and on the screen for a key
 * the system store would not hand over, the advice and whether Start fresh is offered beside it.
 * Every engine this ships on names its system in the agent — WebKit on macOS, WebView2 on Windows
 * and WebKitGTK on Linux — so a wrong answer comes only from somewhere that is not one of them.
 * The core is not asked because the answer is the same for the life of the window.
 */
export function systemHere(agent = navigator.userAgent): System {
  if (/Mac|iPhone|iPad/i.test(agent)) return "mac";
  if (/Windows|Win32|Win64/i.test(agent)) return "windows";
  return "linux";
}

/**
 * What this system calls the place it keeps secrets.
 *
 * A person is told where their password went, and "the credential store" is not what any of these
 * systems calls it — so it is a sentence they cannot check. Keychain, Credential Manager and the
 * system keyring are names they can go and look at.
 */
export function secretStoreName(system: System = systemHere()): TranslationKey {
  switch (system) {
    case "mac":
      return "secrets.keychain";
    case "windows":
      return "secrets.credentialManager";
    case "linux":
      return "secrets.keyring";
  }
}

/**
 * How this system writes the keys a shortcut list names.
 *
 * Symbols on macOS, which print them on its keys and in its menus the same in every language, and
 * words everywhere else, which a keyboard prints in its own language: Strg and Entf on a German
 * one, Suppr and Échap on a French one. The words are catalogue keys for that reason.
 */
export function keyNames(
  t: (key: KeyName) => string,
  system: System = systemHere(),
): KeyNames {
  return system === "mac"
    ? {
        enter: "\u21a9",
        delete: "\u232b",
        escape: "\u238b",
        command: "\u2318",
        shift: "\u21e7",
      }
    : {
        enter: t("keys.nameEnter"),
        delete: t("keys.nameDelete"),
        escape: t("keys.nameEscape"),
        command: t("keys.nameControl"),
        shift: t("keys.nameShift"),
      };
}

/**
 * A chord as each system writes one: `⇧⌘R` on a Mac, with the symbols run together in the order
 * its menus print them, and `Ctrl+Shift+R` everywhere else.
 */
export function chordName(
  named: KeyNames,
  key: string,
  shift = false,
  system: System = systemHere(),
): string {
  return system === "mac"
    ? `${shift ? named.shift : ""}${named.command}${key}`
    : [named.command, ...(shift ? [named.shift] : []), key].join("+");
}

type KeyNames = {
  enter: string;
  delete: string;
  escape: string;
  command: string;
  shift: string;
};

type KeyName =
  | "keys.nameEnter"
  | "keys.nameDelete"
  | "keys.nameEscape"
  | "keys.nameControl"
  | "keys.nameShift";
