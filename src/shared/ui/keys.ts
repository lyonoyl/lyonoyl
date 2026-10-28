/**
 * Whether a key belongs to whoever is typing rather than to the window.
 *
 * A one-letter shortcut in a mail client is a trap without this: `c` typed into a subject line
 * would throw away the message being written and open another. So the window's keys are inert
 * wherever text is being entered, and that includes the composer's own body, which is a
 * contenteditable rather than a field.
 */
export function somebodyIsTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;

  if (target.isContentEditable) return true;

  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;

  // A button and a checkbox are inputs too, and neither takes text. Only the ones that do.
  if (tag !== "INPUT") return false;

  const kind = (target as HTMLInputElement).type;
  return !["button", "checkbox", "radio", "range", "reset", "submit"].includes(
    kind,
  );
}

/**
 * Whether this keystroke is a plain letter rather than a system-wide command.
 *
 * `Cmd+C` and `Ctrl+C` copy, on every one of the three systems, and a window that read them as
 * "write a message" would take the clipboard away from everybody who uses it.
 */
export function isPlainKey(event: {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): boolean {
  return !event.metaKey && !event.ctrlKey && !event.altKey;
}

/** What a chord asks for: each is also asked for by a letter. */
export type MailChord =
  | "compose"
  | "answer"
  | "answerAll"
  | "forward"
  | "throwAway";

/**
 * What this keystroke asks for, where it is one of the chords mail applications answer to: `Cmd`
 * on macOS and `Ctrl` on Windows and Linux, with `N` for a new message, `R` to answer, `Shift` and
 * `R` to answer everybody, `Shift` and `F` to pass a message on, and Delete or Backspace to throw
 * it away. `null` for anything else.
 *
 * `code` rather than `key`, for the reason `opensSettings` gives.
 */
export function mailChord(event: {
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): MailChord | null {
  if (!(event.metaKey || event.ctrlKey) || event.altKey) return null;

  switch (event.code) {
    case "KeyN":
      return event.shiftKey ? null : "compose";
    case "KeyR":
      return event.shiftKey ? "answerAll" : "answer";
    case "KeyF":
      return event.shiftKey ? "forward" : null;
    case "Backspace":
    case "Delete":
      return event.shiftKey ? null : "throwAway";
    default:
      return null;
  }
}

/**
 * Whether this keystroke is the one that opens the settings.
 *
 * `Cmd+,` on macOS and `Ctrl+,` on Windows and Linux — the same place in every application on
 * each of them, which is the whole reason it is the one shortcut here that is not a plain letter.
 *
 * `code` rather than `key`, because `key` is whatever the layout puts on that key: on a Russian
 * layout it is a Cyrillic letter, and a person whose keyboard is in Russian reaches for the same
 * physical key and expects the same thing to happen.
 */
export function opensSettings(event: {
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}): boolean {
  return (
    event.code === "Comma" &&
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    !event.shiftKey
  );
}
