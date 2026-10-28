import { describe, expect, it } from "vitest";

import {
  LANGUAGES,
  SUPPORTED_LANGUAGES,
  createTranslator,
} from "@/shared/i18n";

import { chordName, keyNames, secretStoreName, systemHere } from "./platform";

describe("the names of the keys a shortcut is held with", () => {
  it("are the symbols a Mac prints on its keys, in every language", () => {
    const t = createTranslator("ru");
    expect(keyNames(t, "mac")).toEqual({
      enter: "\u21a9",
      delete: "\u232b",
      escape: "\u238b",
      command: "\u2318",
      shift: "\u21e7",
    });
  });

  it("are words of the interface's language everywhere else", () => {
    // What a keyboard prints in that language: the catalogue is where those words live.
    const named = keyNames(createTranslator("en"), "windows");
    expect(named).toEqual({
      enter: "Enter",
      delete: "Delete",
      escape: "Esc",
      command: "Ctrl",
      shift: "Shift",
    });
    const asked: string[] = [];
    keyNames(
      ((key: string) => {
        asked.push(key);
        return key;
      }) as never,
      "linux",
    );
    expect(asked.sort()).toEqual([
      "keys.nameControl",
      "keys.nameDelete",
      "keys.nameEnter",
      "keys.nameEscape",
      "keys.nameShift",
    ]);
  });

  it("write a chord as each system's menus write one", () => {
    const mac = keyNames(createTranslator("en"), "mac");
    expect(chordName(mac, "N", false, "mac")).toBe("\u2318N");
    expect(chordName(mac, "R", true, "mac")).toBe("\u21e7\u2318R");
    expect(chordName(mac, mac.delete, false, "mac")).toBe("\u2318\u232b");

    const windows = keyNames(createTranslator("en"), "windows");
    expect(chordName(windows, "N", false, "windows")).toBe("Ctrl+N");
    expect(chordName(windows, "R", true, "windows")).toBe("Ctrl+Shift+R");
    expect(chordName(windows, windows.delete, false, "linux")).toBe(
      "Ctrl+Delete",
    );
  });
});

describe("the system this window is on", () => {
  it.each([
    ["Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "mac"],
    ["Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "windows"],
    ["Mozilla/5.0 (X11; Linux x86_64)", "linux"],
  ])("reads %s as %s", (agent, expected) => {
    expect(systemHere(agent)).toBe(expected);
  });

  it("answers with a system rather than nothing for an agent it does not know", () => {
    // Being wrong costs the name of one thing in one sentence. Having no answer would cost the
    // sentence, which is the one telling a person where their password went.
    expect(systemHere("something nobody has shipped")).toBe("linux");
  });

  it("names the store on every system, in every language", () => {
    // A key with no text is the sentence about a person's password showing them a key instead.
    for (const system of ["mac", "windows", "linux"] as const) {
      for (const language of SUPPORTED_LANGUAGES) {
        const named = createTranslator(language)(secretStoreName(system));

        expect(named, `${system} in ${language}`).not.toBe("");
        expect(named).not.toContain("secrets.");
      }
    }
  });

  it("gives each system a different name", () => {
    // One name for all three would be the generic wording this replaced, wearing a parameter.
    const named = (["mac", "windows", "linux"] as const).map(
      (system) => LANGUAGES.en.catalog[secretStoreName(system)],
    );

    expect(new Set(named).size).toBe(3);
  });
});
