import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Chooser } from "./Chooser";

// Names made up for the test, so nothing here is a group anybody has. What matters is how they
// begin: each starts with a letter the window answers as a shortcut.
const OPTIONS = [
  { value: null, label: "Nothing" },
  { value: "g1", label: "Clients" },
  { value: "g2", label: "Receipts" },
  { value: "g3", label: "Questions" },
];

// Languages as the settings offer them: each under the name it gives itself, with its code drawn
// before it. Real ones, because what matters is a code and a name that begin differently.
const LANGUAGES = [
  { value: "system", label: "As the system" },
  { value: "de", label: "Deutsch", code: "de" },
  { value: "en", label: "English", code: "en" },
  { value: "tr", label: "Türkçe", code: "tr" },
  { value: "ru", label: "Русский", code: "ru" },
  { value: "th", label: "ไทย", code: "th" },
  { value: "ko", label: "한국어", code: "ko" },
  { value: "ja", label: "日本語", code: "ja" },
];

// Two made-up names alike up to their second word.
const ALIKE = [
  { value: null, label: "Nothing" },
  { value: "w1", label: "Work mail" },
  { value: "w2", label: "Work trips" },
];

/** Every key that reached the window, the way the window's own shortcuts listen for them. */
let windowSaw: string[] = [];
const listen = (event: KeyboardEvent) => windowSaw.push(event.key);

beforeEach(() => {
  windowSaw = [];
  window.addEventListener("keydown", listen);
});

afterEach(() => {
  window.removeEventListener("keydown", listen);
});

function show(value: string | null = null) {
  const chose = vi.fn();
  render(
    <Chooser
      label="Group"
      value={value}
      options={OPTIONS}
      onChoose={chose}
    />,
  );
  return { chose, trigger: screen.getByRole("combobox", { name: "Group" }) };
}

/** A chooser that shows what was chosen, as every screen using one does. */
function showChoosing(
  options: ReadonlyArray<{ value: string | null; label: string; code?: string }>,
  first: string | null,
) {
  const chose = vi.fn();
  function Shown() {
    const [value, setValue] = useState(first);
    return (
      <Chooser
        label="Choice"
        value={value}
        options={options}
        onChoose={(next) => {
          setValue(next);
          chose(next);
        }}
      />
    );
  }
  render(<Shown />);
  return { chose, trigger: screen.getByRole("combobox", { name: "Choice" }) };
}

describe("Chooser", () => {
  it("scrolls a long list with no arrows at its ends", async () => {
    // The primitive's list draws an arrow at an end there is more to scroll to, and takes it away
    // at the end itself; the rows jump by the arrow's height each time. Without layout nothing
    // here has a height, so the list is given one that overflows.
    const tall = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(2000);
    const short = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(300);
    try {
      render(
        <Chooser
          label="Group"
          value="g0"
          options={Array.from({ length: 40 }, (_, at) => ({ value: `g${at}`, label: `Group ${at}` }))}
          onChoose={() => {}}
        />,
      );
      const trigger = screen.getByRole("combobox", { name: "Group" });
      trigger.focus();
      fireEvent.keyDown(trigger, { key: "Enter" });
      const list = await screen.findByRole("listbox");
      await screen.findByRole("option", { name: "Group 39" });

      expect(within(list).getAllByRole("option")).toHaveLength(40);
      expect(document.querySelector('[data-slot="select-scroll-down-button"]')).toBeNull();
      expect(document.querySelector('[data-slot="select-scroll-up-button"]')).toBeNull();
    } finally {
      tall.mockRestore();
      short.mockRestore();
    }
  });

  it("keeps a chosen name in its own direction where it is asked to", async () => {
    // A group somebody named "Work." keeps its full stop at the end in a window that runs right
    // to left; an entry not asked to is left to the list's direction.
    render(
      <Chooser
        label="Group"
        value={null}
        options={[
          { value: null, label: "Nothing" },
          { value: "g1", label: "Work.", dir: "auto" },
        ]}
        onChoose={() => {}}
      />,
    );
    const trigger = screen.getByRole("combobox", { name: "Group" });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const list = await screen.findByRole("listbox");

    expect(within(list).getByText("Work.").getAttribute("dir")).toBe("auto");
    expect(within(list).getByText("Nothing").getAttribute("dir")).toBeNull();
  });

  it("jumps to an entry by its first letter, and the window never hears the letter", () => {
    // The window opens a message on `c`. The same key typed to reach "Clients" must do one thing.
    const { chose, trigger } = show();
    trigger.focus();

    fireEvent.keyDown(trigger, { key: "c" });

    expect(chose).toHaveBeenCalledWith("g1");
    expect(windowSaw).toEqual([]);
  });

  it("keeps every letter typed into the open list to the list", async () => {
    const { trigger } = show();
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
    const list = await screen.findByRole("listbox");
    const first = within(list).getAllByRole("option")[0];
    // Enter opened it, and that key is nobody's shortcut; counted from here.
    windowSaw = [];

    for (const key of ["r", "c", "/", "?"]) {
      fireEvent.keyDown(first, { key });
      fireEvent.keyDown(list, { key });
    }

    expect(windowSaw).toEqual([]);
  });

  it("lets a letter held with a system key through, because that one is the window's", () => {
    // `Cmd+,` opens the settings on every system, and `Cmd+C` copies. Neither is typing.
    const { trigger } = show();
    trigger.focus();

    fireEvent.keyDown(trigger, { key: ",", metaKey: true });
    fireEvent.keyDown(trigger, { key: "c", ctrlKey: true });

    expect(windowSaw).toEqual([",", "c"]);
  });

  it("lets a key that is not a letter through when the list is closed", () => {
    // Escape on the closed control is how the screen it stands on is left, and Tab is how the
    // person moves on from it; neither belongs to the list.
    const { trigger } = show();
    trigger.focus();

    fireEvent.keyDown(trigger, { key: "Escape" });
    fireEvent.keyDown(trigger, { key: "Tab" });

    expect(windowSaw).toEqual(["Escape", "Tab"]);
  });

  describe("typing", () => {
    // When each key is pressed, in milliseconds: the pause between two keys decides whether the
    // second goes on with the search the first began.
    let now = 0;
    beforeEach(() => {
      now = 0;
      vi.spyOn(Date, "now").mockImplementation(() => now);
    });
    afterEach(() => {
      vi.restoreAllMocks();
    });

    /** A key pressed at a moment, where a keyboard sends it: to whatever holds the focus. */
    const press = (key: string, at: number) => {
      now = at;
      fireEvent.keyDown(document.activeElement ?? document.body, { key });
    };

    it("finds an entry by the code drawn before it", () => {
      // `th` begins no name, only the code before ไทย, and `t` alone begins Türkçe.
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();

      press("t", 0);
      press("h", 100);

      expect(chose).toHaveBeenLastCalledWith("th");
    });

    it("finds an entry by its name, in the script the name is written in", () => {
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();

      press("р", 0);
      press("у", 100);

      expect(chose.mock.calls).toEqual([["ru"]]);
    });

    it("starts a new search after a pause, and goes on with the same one without", () => {
      // KO, a look at what it found, and JA two seconds later: a second search, not `koja`,
      // which begins nothing.
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();

      press("k", 0);
      press("o", 100);
      press("j", 2100);
      press("a", 2200);
      expect(chose.mock.calls).toEqual([["ko"], ["ja"]]);

      // A letter straight after is more of the same search, and `jad` begins nothing.
      press("d", 2300);
      expect(chose.mock.calls).toEqual([["ko"], ["ja"]]);
    });

    it("moves on to the next entry the same letter begins, each time it is typed", () => {
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();

      press("t", 0);
      press("t", 100);
      press("t", 200);

      expect(chose.mock.calls).toEqual([["tr"], ["th"], ["tr"]]);
    });

    it("moves along the open list to the entry typed, and takes it only on Enter", async () => {
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();
      fireEvent.keyDown(trigger, { key: "Enter" });
      await screen.findByRole("listbox");

      press("k", 0);
      press("o", 100);
      expect(document.activeElement).toBe(
        screen.getByRole("option", { name: "한국어" }),
      );
      expect(chose).not.toHaveBeenCalled();

      press("Enter", 200);
      expect(chose.mock.calls).toEqual([["ko"]]);
    });

    it("takes a space inside a name being typed as part of it, at the closed control", () => {
      // Rather than as the key that opens the list.
      const { chose, trigger } = showChoosing(ALIKE, null);
      trigger.focus();

      for (const [at, key] of [..."work t"].entries()) press(key, at * 100);

      expect(chose).toHaveBeenLastCalledWith("w2");
      expect(trigger.textContent).toContain("Work trips");
      expect(screen.queryByRole("listbox")).toBeNull();
    });

    it("takes a space inside a name being typed as part of it, in the open list", async () => {
      // Rather than as the key that picks the entry in focus.
      const { chose, trigger } = showChoosing(ALIKE, null);
      trigger.focus();
      fireEvent.keyDown(trigger, { key: "Enter" });
      await screen.findByRole("listbox");

      for (const [at, key] of [..."work t"].entries()) press(key, at * 100);

      expect(document.activeElement).toBe(
        screen.getByRole("option", { name: "Work trips" }),
      );
      expect(screen.getByRole("listbox")).toBeTruthy();
      expect(chose).not.toHaveBeenCalled();
    });

    it("opens the list with a space, and picks the entry in focus with one, while nothing is being typed", async () => {
      const { chose, trigger } = showChoosing(ALIKE, null);
      trigger.focus();

      press(" ", 0);
      await screen.findByRole("listbox");
      press("w", 100);
      press(" ", 2100);

      expect(chose.mock.calls).toEqual([["w1"]]);
    });

    it("moves nothing when no entry begins with what was typed, and the window hears none of it", () => {
      const { chose, trigger } = showChoosing(LANGUAGES, "en");
      trigger.focus();

      press("q", 0);
      press("x", 100);

      expect(chose).not.toHaveBeenCalled();
      expect(trigger.textContent).toContain("English");
      expect(windowSaw).toEqual([]);
    });
  });
});
