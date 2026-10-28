import { describe, expect, it } from "vitest";

import { isPlainKey, mailChord, opensSettings, somebodyIsTyping } from "./keys";

describe("somebodyIsTyping", () => {
  function element(html: string): HTMLElement {
    const holder = document.createElement("div");
    holder.innerHTML = html;
    return holder.firstElementChild as HTMLElement;
  }

  it("says so for every field text goes into", () => {
    // A one-letter shortcut is a trap without this: `c` typed into a subject line would throw
    // away the message being written and open another.
    for (const html of [
      '<input type="text">',
      '<input type="email">',
      '<input type="password">',
      '<input type="search">',
      "<input>",
      "<textarea></textarea>",
      "<select></select>",
    ]) {
      expect(somebodyIsTyping(element(html)), html).toBe(true);
    }
  });

  it("says so for the composer's body, which is not a field at all", () => {
    const editable = element('<div contenteditable="true"></div>');

    expect(somebodyIsTyping(editable)).toBe(true);
  });

  it("does not say so for inputs that take no text", () => {
    // A button and a checkbox are inputs too. Treating them as typing would make the window's
    // keys dead wherever somebody had last clicked a control.
    for (const html of [
      '<input type="button">',
      '<input type="checkbox">',
      '<input type="radio">',
      '<input type="submit">',
      "<button></button>",
      "<div></div>",
    ]) {
      expect(somebodyIsTyping(element(html)), html).toBe(false);
    }
  });

  it("does not say so for nothing at all", () => {
    expect(somebodyIsTyping(null)).toBe(false);
  });
});

describe("isPlainKey", () => {
  it("refuses anything held with a system modifier", () => {
    // Cmd+C and Ctrl+C copy, on every one of the three systems. A window reading them as "write a
    // message" would take the clipboard away from everybody who uses it.
    for (const held of [
      { metaKey: true, ctrlKey: false, altKey: false },
      { metaKey: false, ctrlKey: true, altKey: false },
      { metaKey: false, ctrlKey: false, altKey: true },
    ]) {
      expect(isPlainKey(held)).toBe(false);
    }
  });

  it("accepts a letter pressed on its own", () => {
    expect(isPlainKey({ metaKey: false, ctrlKey: false, altKey: false })).toBe(
      true,
    );
  });
});

function pressingComma(
  over: Partial<Parameters<typeof opensSettings>[0]> = {},
) {
  return {
    code: "Comma",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...over,
  };
}

describe("opensSettings", () => {
  it("is the comma held with the key the system uses for a command", () => {
    expect(opensSettings(pressingComma({ metaKey: true }))).toBe(true);
    expect(opensSettings(pressingComma({ ctrlKey: true }))).toBe(true);
  });

  it("is not a comma somebody typed", () => {
    // Otherwise the settings open in the middle of a sentence.
    expect(opensSettings(pressingComma())).toBe(false);
  });

  it("is not the comma with something else held as well", () => {
    // Those belong to other commands, and answering them here takes one away.
    expect(
      opensSettings(pressingComma({ metaKey: true, shiftKey: true })),
    ).toBe(false);
    expect(opensSettings(pressingComma({ ctrlKey: true, altKey: true }))).toBe(
      false,
    );
  });

  it("asks which key was pressed rather than which letter it produced", () => {
    // On a Russian layout that key produces a Cyrillic letter. A person reaching for the same
    // place on the keyboard means the same thing, whatever their layout puts on it.
    expect(opensSettings(pressingComma({ metaKey: true }))).toBe(true);
    expect(opensSettings(pressingComma({ code: "KeyB", metaKey: true }))).toBe(
      false,
    );
  });

  it("is refused by the guard the window's other keys sit behind", () => {
    // Which is why it is answered before that guard rather than beside it: every other shortcut
    // here is a plain letter, and this one is the only one held with a command key.
    expect(isPlainKey(pressingComma({ metaKey: true }))).toBe(false);
  });
});

describe("mailChord", () => {
  const held = (
    code: string,
    { meta = false, ctrl = false, shift = false, alt = false } = {},
  ) => ({ code, metaKey: meta, ctrlKey: ctrl, shiftKey: shift, altKey: alt });

  it("reads the chords mail applications use, with Cmd and with Ctrl alike", () => {
    for (const command of [{ meta: true }, { ctrl: true }]) {
      expect(mailChord(held("KeyN", command))).toBe("compose");
      expect(mailChord(held("KeyR", command))).toBe("answer");
      expect(mailChord(held("KeyR", { ...command, shift: true }))).toBe("answerAll");
      expect(mailChord(held("KeyF", { ...command, shift: true }))).toBe("forward");
      expect(mailChord(held("Backspace", command))).toBe("throwAway");
      expect(mailChord(held("Delete", command))).toBe("throwAway");
    }
  });

  it("reads the physical key, so a keyboard in another language asks for the same", () => {
    // `code` names the key wherever it is: the letter a Russian layout puts there is not asked.
    expect(mailChord({ ...held("KeyR", { meta: true }) })).toBe("answer");
  });

  it("leaves alone what is not one of them", () => {
    // Plain keys are the letters' business, Cmd+F is finding, and Option changes what a key is.
    expect(mailChord(held("KeyR"))).toBeNull();
    expect(mailChord(held("Backspace"))).toBeNull();
    expect(mailChord(held("KeyF", { meta: true }))).toBeNull();
    expect(mailChord(held("KeyN", { meta: true, shift: true }))).toBeNull();
    expect(mailChord(held("KeyR", { meta: true, alt: true }))).toBeNull();
    expect(mailChord(held("Backspace", { meta: true, shift: true }))).toBeNull();
    expect(mailChord(held("KeyC", { meta: true }))).toBeNull();
    expect(mailChord(held("Comma", { meta: true }))).toBeNull();
  });
});
