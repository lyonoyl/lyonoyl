import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { SupportedLanguage } from "@/shared/i18n";
import { WhatChanged } from "./WhatChanged";

function show(
  version: string,
  notes: string | Record<string, string>,
  language: SupportedLanguage = "en",
) {
  render(
    <I18nProvider initial={language}>
      <WhatChanged
        changed={{
          version,
          notes: typeof notes === "string" ? { en: notes } : notes,
        }}
      />
    </I18nProvider>,
  );
}

describe("what changed", () => {
  it("names the version and says what the release said", () => {
    show("1.1.0", "Pictures load again.\nThe list is faster.");

    expect(screen.getByRole("heading").textContent).toContain("1.1.0");
    expect(screen.getByText(/Pictures load again/)).toBeTruthy();
  });

  /// A release that went out without notes says so rather than showing an empty pane.
  it("says plainly when a version went out without notes", () => {
    show("1.1.0", "   ");

    expect(screen.getByText(/went out without notes/)).toBeTruthy();
  });

  /// The note is read in the language the person chose, not in whichever one the release wrote
  /// first.
  it("shows the note written for the language in use", () => {
    show("1.1.0", { en: "In English.", ru: "In the second language." }, "ru");

    expect(screen.getByText("In the second language.")).toBeTruthy();
    expect(screen.queryByText("In English.")).toBeNull();
  });

  /// The release names a note after its catalogue file, `pt-BR`; the window names the language
  /// in lower case. The note is still the one for the language in use.
  it("finds the note whatever case the release wrote its language in", () => {
    show(
      "1.1.0",
      { en: "In English.", "pt-BR": "In the second language." },
      "pt-br",
    );

    expect(screen.getByText("In the second language.")).toBeTruthy();
    expect(screen.queryByText("In English.")).toBeNull();
  });

  /// English, which every catalogue is kept level with, where a release wrote nothing in the
  /// language in use.
  it("falls back to English where the release wrote nothing in the language in use", () => {
    show("1.1.0", { en: "In English." }, "ru");

    expect(screen.getByText("In English.")).toBeTruthy();
  });

  /// A note left blank in the language in use is no note in it, and English stands in.
  it("falls back to English where the note in the language in use is blank", () => {
    show("1.1.0", { en: "In English.", ru: "  " }, "ru");

    expect(screen.getByText("In English.")).toBeTruthy();
  });

  /// No note in any language a person can read is no note at all.
  it("says there are no notes when none is written in a language the window has", () => {
    show("1.1.0", { xx: "Somewhere else." });

    expect(screen.getByText(/went out without notes/)).toBeTruthy();
    expect(screen.queryByText("Somewhere else.")).toBeNull();
  });

  /// What arrived over a network is text, and is shown as text.
  ///
  /// The day this project's own release process is the thing that was tampered with is exactly the
  /// day a note that could carry markup would matter.
  it("shows markup in a note as the characters it is", () => {
    show("1.1.0", "<img src=x onerror=alert(1)> and <b>bold</b>");

    expect(screen.getByText(/<img src=x onerror=alert\(1\)>/)).toBeTruthy();
    expect(document.querySelector("img")).toBeNull();
    expect(document.querySelector("b")).toBeNull();
  });
});
