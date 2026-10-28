import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";

import { ARRIVAL_SHOWS_FOR_MS, SettingsSections } from "./SettingsSections";
import { SETTINGS_SECTIONS, anchorFor } from "./sections";

/** The callback the component handed the observer, so a test can say what is on screen. */
let report:
  | ((entries: Array<{ target: Element; isIntersecting: boolean }>) => void)
  | null;
let watched: Element[];

/** What it was asked to watch through, so a test can say the root is a line and not a band. */
let through: IntersectionObserverInit | undefined;

class FakeObserver {
  constructor(
    given: (
      entries: Array<{ target: Element; isIntersecting: boolean }>,
    ) => void,
    options?: IntersectionObserverInit,
  ) {
    report = given;
    through = options;
  }
  observe(element: Element) {
    watched.push(element);
  }
  disconnect() {
    report = null;
  }
}

/** The screen itself, reduced to the one thing this list needs from it: parts with names. */
function plantTheScreen() {
  for (const one of SETTINGS_SECTIONS) {
    const part = document.createElement("section");
    part.id = anchorFor(one.id);
    part.scrollIntoView = vi.fn();
    document.body.append(part);
  }
}

/**
 * What the screen says is on the pane right now.
 *
 * Reported in the order the ids are given rather than in the screen's own order, and the tests
 * give them out of order on purpose: an observer hands over whatever changed, in whatever order it
 * noticed, and a list that trusted that order would name whichever part happened to be first.
 */
function nowShowing(...ids: string[]) {
  const rest = SETTINGS_SECTIONS.map((one) => one.id).filter(
    (id) => !ids.includes(id),
  );
  act(() => {
    report?.(
      [...ids, ...rest].map((id) => ({
        target: document.getElementById(anchorFor(id)) as Element,
        isIntersecting: ids.includes(id),
      })),
    );
  });
}

beforeEach(() => {
  report = null;
  through = undefined;
  watched = [];
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  plantTheScreen();
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.replaceChildren();
});

function renderSections() {
  render(
    <I18nProvider initial="en">
      <SettingsSections />
    </I18nProvider>,
  );
}

describe("SettingsSections", () => {
  it("names every part of the settings, in the order they stand in", () => {
    renderSections();

    const named = screen
      .getAllByRole("button")
      .map((one) => one.textContent?.trim());

    expect(named).toEqual([
      "Groups",
      "Mailboxes",
      "Notifications",
      "Storage",
      "Appearance",
      "Reading",
      "Language",
      "Updates",
      "Licenses",
    ]);
  });

  it("keeps the licences last, whatever is added after them", () => {
    // A reference read once if ever, below everything a person comes here to change. Asked of the
    // list itself rather than of a copy of it, so a part added at the end of `SETTINGS_SECTIONS`
    // fails here -- and the screen is held to the same list by its own test.
    renderSections();

    const named = screen.getAllByRole("button");

    expect(SETTINGS_SECTIONS[SETTINGS_SECTIONS.length - 1]?.id).toBe(
      "licences",
    );
    expect(named[named.length - 1]?.textContent?.trim()).toBe("Licenses");
  });

  it("puts the groups before the mailboxes that are placed in them", () => {
    const order = SETTINGS_SECTIONS.map((one) => one.id);

    expect(order.indexOf("groups")).toBeLessThan(order.indexOf("accounts"));
  });

  it("scrolls to the part that was pressed", () => {
    renderSections();

    fireEvent.click(screen.getByRole("button", { name: "Storage" }));

    const part = document.getElementById(anchorFor("storage"));
    expect(part?.scrollIntoView).toHaveBeenCalledWith(
      expect.objectContaining({ block: "start" }),
    );
    // Answered at once rather than waiting for the scrolling to arrive somewhere.
    expect(
      screen
        .getByRole("button", { name: "Storage" })
        .getAttribute("aria-current"),
    ).toBe("true");
  });

  it("says on the screen itself where the press landed, and stops saying it", () => {
    // The settings are a long column of the same shapes. A scroll that ends somewhere in it
    // leaves a person hunting for the part they asked for, so the part says so for a moment --
    // and then stops, because a mark that stayed would be a second selection.
    vi.useFakeTimers();
    try {
      renderSections();

      fireEvent.click(screen.getByRole("button", { name: "Reading" }));

      const part = document.getElementById(anchorFor("reading"));
      expect(part?.dataset.arrived).toBe("");
      // The stylesheet is told how long by the same number that takes the mark off, so the fade
      // and its removal cannot drift apart.
      expect(part?.style.getPropertyValue("--arrived-for")).toBe(
        `${ARRIVAL_SHOWS_FOR_MS}ms`,
      );

      act(() => {
        vi.advanceTimersByTime(ARRIVAL_SHOWS_FOR_MS - 1);
      });
      expect(part?.dataset.arrived, "still there while it is fading").toBe("");

      act(() => {
        vi.advanceTimersByTime(2);
      });
      expect(part?.dataset.arrived).toBeUndefined();
      expect(part?.style.getPropertyValue("--arrived-for")).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });

  it("reads the screen through a line, not through a band down from the top", () => {
    // A band catches the part above as well: a tall one still hangs into it, and being earlier it
    // always won -- so the list named the part before the one that had just been scrolled to, and
    // the last part could never be named at all. A line is crossed by one part, which is the one
    // being read.
    renderSections();

    const margin = through?.rootMargin ?? "";
    const [top, , bottom] = margin.split(/\s+/);
    const height =
      100 + Number.parseFloat(top ?? "0") + Number.parseFloat(bottom ?? "0");

    expect(margin, "given in percentages, so it follows the pane").toMatch(/%/);
    expect(height, "the root is a line across the pane").toBe(0);
  });

  it("follows what is on the screen rather than the last press", () => {
    // Somebody scrolling instead of pressing has to be told where they are too. Left following
    // presses alone, the list would go on naming a part that had gone off the top of the pane.
    renderSections();
    expect(watched).toHaveLength(SETTINGS_SECTIONS.length);

    nowShowing("appearance", "reading");

    expect(
      screen
        .getByRole("button", { name: "Appearance" })
        .getAttribute("aria-current"),
    ).toBe("true");
    expect(
      screen
        .getByRole("button", { name: "Storage" })
        .getAttribute("aria-current"),
    ).toBeNull();
  });

  it("names the first in the screen's order when more than one is reported", () => {
    // One part crosses the line at a time, but the two either side of a crossing are reported
    // together -- and in whatever order the observer noticed them, which is not the order they
    // are read in.
    renderSections();

    nowShowing("language", "reading", "appearance");

    expect(
      screen
        .getByRole("button", { name: "Appearance" })
        .getAttribute("aria-current"),
    ).toBe("true");
  });
});
