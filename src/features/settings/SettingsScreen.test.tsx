import { readdirSync } from "node:fs";
import { join } from "node:path";

import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ScreenPane } from "@/app/ScreenPane";
import { SETTINGS_SECTIONS, anchorFor } from "./sections";
import { I18nProvider } from "@/shared/i18n";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import { inRoundNumbers } from "@/shared/ui/bytes";
import { createTranslator } from "@/shared/i18n";
import { SettingsScreen } from "./SettingsScreen";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    setTheme: vi.fn(),
    setDarkScheme: vi.fn(),
    setLightScheme: vi.fn(),
    setDensity: vi.fn(),
    setLanguage: vi.fn(),
    setNewMailNotice: vi.fn(),
    setNoticeShows: vi.fn(),
    setShowPictures: vi.fn(),
    setLookForUpdates: vi.fn(),
    setShareAnonymousData: vi.fn(),
    showWhereTheJournalWrites: vi.fn(),
    openTheSupportPage: vi.fn(),
    thisVersion: vi.fn(),
    listAccountGroups: vi.fn(),
    createAccountGroup: vi.fn(),
    renameAccountGroup: vi.fn(),
    removeAccountGroup: vi.fn(),
    placeAccountInGroup: vi.fn(),
    whatIsHeld: vi.fn(),
    licences: vi.fn(),
    appIconsWearable: vi.fn(),
    setAppIcon: vi.fn(),
  };
});

/**
 * Twenty languages on top of the ones the build holds.
 *
 * The chooser is built for a list that grows with every catalogue dropped into `locales/`, and two
 * entries prove nothing about a list of twenty. Made up rather than real: a tag from the private-use
 * range and a name that says what it is, so nothing here stands for a language somebody reads.
 * Only the screen's view of the registry changes — the provider reads the catalogues itself — so
 * every other test in this file still runs in the real English.
 */
const { MADE_UP } = vi.hoisted(() => ({
  MADE_UP: Array.from(
    { length: 20 },
    (_, at) => `x-made-up-${String(at + 1).padStart(2, "0")}`,
  ),
}));

vi.mock("@/shared/i18n", async () => {
  const actual =
    await vi.importActual<typeof import("@/shared/i18n")>("@/shared/i18n");
  const english = actual.LANGUAGES[actual.SOURCE_LANGUAGE];
  const LANGUAGES = {
    ...actual.LANGUAGES,
    ...Object.fromEntries(
      MADE_UP.map((tag, at) => [
        tag,
        { ...english, name: `Made-up language ${at + 1}` },
      ]),
    ),
  };
  const SUPPORTED_LANGUAGES = [...actual.SUPPORTED_LANGUAGES, ...MADE_UP];
  const order = new Intl.Collator("en");
  return {
    ...actual,
    SUPPORTED_LANGUAGES,
    LANGUAGES,
    LANGUAGES_IN_LIST_ORDER: [...SUPPORTED_LANGUAGES].sort((one, other) =>
      order.compare(LANGUAGES[one].name, LANGUAGES[other].name),
    ),
  };
});

const client = await import("@/shared/ipc/client");
const theme = vi.mocked(client.setTheme);
const scheme = vi.mocked(client.setDarkScheme);
const lightScheme = vi.mocked(client.setLightScheme);
const density = vi.mocked(client.setDensity);
const language = vi.mocked(client.setLanguage);
const notice = vi.mocked(client.setNewMailNotice);
const parts = vi.mocked(client.setNoticeShows);
const pictures = vi.mocked(client.setShowPictures);
const sharing = vi.mocked(client.setShareAnonymousData);
const lookingFor = vi.mocked(client.setLookForUpdates);
const theJournal = vi.mocked(client.showWhereTheJournalWrites);
const supportPage = vi.mocked(client.openTheSupportPage);
const builtAs = vi.mocked(client.thisVersion);
const arrangement = vi.mocked(client.listAccountGroups);
const makeGroup = vi.mocked(client.createAccountGroup);
const renameGroup = vi.mocked(client.renameAccountGroup);
const dropGroup = vi.mocked(client.removeAccountGroup);
const placeAccount = vi.mocked(client.placeAccountInGroup);
const holding = vi.mocked(client.whatIsHeld);
const notices = vi.mocked(client.licences);
const wearable = vi.mocked(client.appIconsWearable);
const wearing = vi.mocked(client.setAppIcon);

/**
 * What the system says about being dark, and a way to change it the way a person changing the
 * system's appearance does. Every other question asked of the window is answered as it was.
 */
function theSystemIs(dark: boolean) {
  const listeners = new Set<() => void>();
  const media = {
    matches: dark,
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) =>
      listeners.delete(listener),
  };
  const asked = window.matchMedia.bind(window);
  vi.spyOn(window, "matchMedia").mockImplementation((query: string) =>
    query === "(prefers-color-scheme: dark)"
      ? (media as unknown as MediaQueryList)
      : asked(query),
  );

  return {
    turns(darkNow: boolean) {
      media.matches = darkNow;
      for (const listener of listeners) listener();
    },
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

const STORED: PreferencesDto = {
  theme: "system",
  darkScheme: "graphite",
  lightScheme: "paper",
  appIcon: "ember",
  density: "comfortable",
  language: "en",
  newMailNotice: "inbox",
  noticeShows: { sender: true, subject: true, preview: true },
  showPictures: "whenAsked",
  lookForUpdates: "notAsked",
  shareAnonymousData: false,
  welcomed: true,
};

function renderSettings(
  stored: PreferencesDto = STORED,
  accounts: MailAccountSummaryDto[] = [],
) {
  const changed = vi.fn();
  const opened = vi.fn();
  const rearranged = vi.fn();
  const closed = vi.fn();
  render(
    <I18nProvider initial="en">
      {/* In the pane it ships in: the way out of this screen is the pane's cross, and what that
          press does to a half-typed field is part of what is being tested. */}
      <ScreenPane
        paneRef={{ current: null }}
        closeLabel="Close this screen"
        onClose={closed}
      >
        <SettingsScreen
          preferences={stored}
          onChanged={changed}
          accounts={accounts}
          onOpenAccount={opened}
          onArrangementChanged={rearranged}
        />
      </ScreenPane>
    </I18nProvider>,
  );
  return { changed, opened, rearranged, closed };
}

/** Opens a dropdown list the way a pointer does, and hands back the list it opened. */
async function openChooser(name: string | RegExp) {
  fireEvent.click(await screen.findByRole("combobox", { name }));
  return screen.findByRole("listbox");
}

/** What a dropdown list offers, in its order. */
/** What an entry or a closed control says, its code aside: the code is drawn for the eye only. */
function said(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('[aria-hidden="true"]').forEach((hidden) => hidden.remove());
  return copy.textContent;
}

function offered(list: HTMLElement) {
  return within(list).getAllByRole("option").map(said);
}

beforeEach(() => {
  for (const write of [
    theme,
    scheme,
    lightScheme,
    density,
    language,
    notice,
    pictures,
    sharing,
  ])
    write.mockReset();
  theJournal.mockReset();
  theJournal.mockResolvedValue(undefined);
  supportPage.mockReset();
  supportPage.mockResolvedValue(undefined);
  builtAs.mockReset();
  builtAs.mockResolvedValue("1.2.3");
  lookingFor.mockReset();
  lookingFor.mockResolvedValue({ ...STORED, lookForUpdates: "onceADay" });
  // A build from a checkout, which is what every test here runs as unless it says otherwise.
  // Reset rather than left standing: a call count that survives into the next test turns
  // "asked once" into a claim about the order the tests happened to run in.
  for (const write of [makeGroup, renameGroup, dropGroup, placeAccount]) {
    write.mockReset();
    write.mockResolvedValue(undefined as never);
  }
  arrangement.mockReset();
  arrangement.mockResolvedValue([]);
  holding.mockReset();
  holding.mockResolvedValue([]);
  notices.mockReset();
  notices.mockResolvedValue([]);
  theme.mockResolvedValue({ ...STORED, theme: "dark" });
  scheme.mockResolvedValue({ ...STORED, darkScheme: "nord" });
  lightScheme.mockResolvedValue({ ...STORED, lightScheme: "catppuccinLatte" });
  density.mockResolvedValue({ ...STORED, density: "compact" });
  language.mockResolvedValue({ ...STORED, language: "ru" });
  notice.mockResolvedValue({ ...STORED, newMailNotice: "never" });
  parts.mockReset();
  parts.mockResolvedValue({
    ...STORED,
    noticeShows: { sender: false, subject: true, preview: true },
  });
  pictures.mockResolvedValue({ ...STORED, showPictures: "always" });
  sharing.mockResolvedValue({ ...STORED, shareAnonymousData: true });
  // A system that cannot change the picture of a running application, unless a test says otherwise.
  wearable.mockReset();
  wearable.mockResolvedValue(false);
  wearing.mockReset();
  wearing.mockResolvedValue({ ...STORED, appIcon: "rust" });
});

/** The pictures offered under the heading that names them, once the system has said it can. */
const offeredPictures = async () =>
  within(await screen.findByRole("group", { name: "App icon" })).getAllByRole(
    "button",
  );

/** Which of the pictures are pressed, by what they are. */
const pressedPictures = (offered: HTMLElement[]) =>
  offered
    .filter((one) => one.getAttribute("aria-pressed") === "true")
    .map((one) => one.getAttribute("data-app-icon"));

describe("SettingsScreen", () => {
  it("offers every mailbox from here, whether there is one of them or many", async () => {
    // Not from the account row in the sidebar: that row is only drawn when there is more than
    // one account, so a person with a single mailbox would have had nowhere to reach it from.
    const { opened } = renderSettings(STORED, [
      {
        id: "acc-1",
        email: "only@example.com",
        displayName: "Only",
        provider: "imap",
        canSend: false,
      },
    ]);

    fireEvent.click(await screen.findByRole("button", { name: "Edit Only" }));

    expect(opened).toHaveBeenCalledWith("acc-1");
    // A name somebody chose keeps its own direction.
    expect(screen.getAllByText("Only").every((name) => name.getAttribute("dir") === "auto")).toBe(true);
  });

  it("gives each mailbox a pencil of its own, named after it", async () => {
    // Two rows, and the second one pressed: a pencil wired to the first mailbox in the list would
    // pass a test that only ever had one.
    const { opened } = renderSettings(STORED, [
      {
        id: "acc-1",
        email: "zeta@example.com",
        displayName: "Zeta Quorin",
        provider: "imap",
        canSend: true,
      },
      {
        id: "acc-2",
        email: "kappa@example.net",
        // Blank, so the row is named by its address -- the same thing the row itself shows.
        displayName: "  ",
        provider: "imap",
        canSend: true,
      },
    ]);

    const pencil = await screen.findByRole("button", {
      name: "Edit kappa@example.net",
    });
    fireEvent.click(pencil);

    expect(opened).toHaveBeenCalledTimes(1);
    expect(opened).toHaveBeenCalledWith("acc-2");
    // A pencil and nothing else, with the name as its hint for somebody using a pointer.
    expect(pencil.textContent).toBe("");
    expect(pencil.querySelector("svg")).not.toBeNull();
    expect(pencil.getAttribute("title")).toBe("Edit kappa@example.net");
    // And no word-button left beside it saying the same thing.
    expect(screen.queryByRole("button", { name: "Open" })).toBeNull();
  });

  it("offers everything this application lets a person decide", () => {
    renderSettings();

    for (const control of [
      "Light or dark",
      "Light colors",
      "How tightly mail is listed",
      "Interface language",
    ]) {
      expect(screen.getByText(control), control).toBeTruthy();
    }
  });

  it("has no save button, because a choice is the saving", async () => {
    // A screen that can be left in an unsaved state is a screen that can lie about what it is
    // showing.
    renderSettings();

    for (const button of screen.getAllByRole("button")) {
      expect(button.textContent?.toLowerCase()).not.toContain("save");
    }
  });

  it("writes a theme the moment it is chosen, and wears what came back", async () => {
    const { changed } = renderSettings();

    fireEvent.click(screen.getByRole("button", { name: "Dark" }));

    expect(theme).toHaveBeenCalledWith("dark");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, theme: "dark" }),
    );
  });

  it("offers a white scheme and five dark ones people already know", () => {
    renderSettings({ ...STORED, theme: "dark" });

    for (const named of [
      "Graphite",
      "Dracula",
      "Nord",
      "Solarized",
      "Gruvbox",
      "One Dark",
    ]) {
      expect(
        screen.getByRole("button", { name: new RegExp(named) }),
        named,
      ).toBeTruthy();
    }
  });

  it("offers the light colours alone while the window is light", () => {
    // These are what light looks like. The dark ones offered beside them would be a swatch that
    // moves when chosen while nothing on the screen does -- a control that reads as broken.
    renderSettings({ ...STORED, theme: "light" });

    const colours = screen.getByRole("group", { name: "Light colors" });
    expect(within(colours).getAllByRole("button")).toHaveLength(5);
    expect(screen.queryByRole("group", { name: "Dark colors" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Nord/ })).toBeNull();
  });

  it("offers the dark colours alone while the window is dark", () => {
    renderSettings({ ...STORED, theme: "dark" });

    const colours = screen.getByRole("group", { name: "Dark colors" });
    expect(within(colours).getAllByRole("button")).toHaveLength(6);
    expect(screen.queryByRole("group", { name: "Light colors" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Catppuccin/ })).toBeNull();
  });

  it("follows the system: offered while it is dark, gone when it turns light", () => {
    const system = theSystemIs(true);
    renderSettings({ ...STORED, theme: "system" });
    expect(screen.getByRole("button", { name: /Nord/ })).toBeTruthy();

    act(() => system.turns(false));

    expect(screen.queryByRole("button", { name: /Nord/ })).toBeNull();

    act(() => system.turns(true));

    expect(screen.getByRole("button", { name: /Nord/ })).toBeTruthy();
  });

  it("leaves the theme following the system when a scheme is chosen", async () => {
    // Offered only while the window is dark, so the scheme chosen is the one seen: a theme moved
    // to "dark" with it would stop the window following the system without anybody asking.
    theSystemIs(true);
    const { changed } = renderSettings({ ...STORED, theme: "system" });

    fireEvent.click(screen.getByRole("button", { name: /Nord/ }));

    expect(scheme).toHaveBeenCalledWith("nord");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, darkScheme: "nord" }),
    );
    expect(theme).not.toHaveBeenCalled();
  });

  it("leaves the theme alone when it is already dark", async () => {
    // Nothing to switch, and a second write would be a write to the disk for no change at all.
    const { changed } = renderSettings({ ...STORED, theme: "dark" });

    fireEvent.click(screen.getByRole("button", { name: /Nord/ }));

    expect(scheme).toHaveBeenCalledWith("nord");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, darkScheme: "nord" }),
    );
    expect(theme).not.toHaveBeenCalled();
  });

  it("shows which scheme is worn, so a person can see where they are", () => {
    renderSettings({ ...STORED, theme: "dark", darkScheme: "gruvboxDark" });

    expect(
      screen
        .getByRole("button", { name: /Gruvbox/ })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen
        .getByRole("button", { name: /Dracula/ })
        .getAttribute("aria-pressed"),
    ).toBe("false");
  });

  it("offers paper and four light schemes people already know", () => {
    renderSettings({ ...STORED, theme: "light" });

    for (const named of [
      "Paper",
      "Solarized",
      "Gruvbox",
      "Catppuccin Latte",
      "Rosé Pine Dawn",
    ]) {
      expect(
        screen.getByRole("button", { name: new RegExp(named) }),
        named,
      ).toBeTruthy();
    }
  });

  it("follows the system from one side's colours to the other's", () => {
    const system = theSystemIs(false);
    renderSettings({ ...STORED, theme: "system" });
    expect(screen.getByRole("button", { name: /Catppuccin/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Nord/ })).toBeNull();

    act(() => system.turns(true));

    expect(screen.queryByRole("button", { name: /Catppuccin/ })).toBeNull();
    expect(screen.getByRole("button", { name: /Nord/ })).toBeTruthy();
  });

  it("keeps a light scheme chosen and leaves the theme and the dark scheme alone", async () => {
    theSystemIs(false);
    const { changed } = renderSettings({ ...STORED, theme: "system" });

    fireEvent.click(screen.getByRole("button", { name: /Catppuccin/ }));

    expect(lightScheme).toHaveBeenCalledWith("catppuccinLatte");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...STORED,
        lightScheme: "catppuccinLatte",
        appIcon: "ember",
      }),
    );
    expect(theme).not.toHaveBeenCalled();
    expect(scheme).not.toHaveBeenCalled();
  });

  it("keeps one block for both sides, so turning dark changes what it holds and not where it stands", () => {
    // A block per side, one folding away while the other unfolds, shows both headings at once and
    // moves everything below it while it does.
    const system = theSystemIs(false);
    renderSettings({ ...STORED, theme: "system" });
    const light = screen.getByRole("group", { name: "Light colors" });
    const section = light.parentElement;

    act(() => system.turns(true));

    const dark = screen.getByRole("group", { name: "Dark colors" });
    expect(dark).toBe(light);
    expect(dark.parentElement).toBe(section);
    // Its heading, light or dark, and the colours: nothing beside them folded away and taking room.
    expect(section?.children).toHaveLength(3);
  });

  it("fills as many rows with either side's colours, two to a row or three", () => {
    // Five light palettes and six dark ones take three rows of two or two rows of three alike, so
    // the block keeps its height when the window turns. A seventh on one side would not.
    const system = theSystemIs(false);
    renderSettings({ ...STORED, theme: "system" });
    const rows = within(screen.getByRole("group", { name: "Light colors" }))
      .getAllByRole("button")[0].parentElement!;
    const light = rows.children.length;

    act(() => system.turns(true));
    const dark = rows.children.length;

    expect(rows.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["grid-cols-2", "sm:grid-cols-3"]),
    );
    expect(Math.ceil(light / 2)).toBe(Math.ceil(dark / 2));
    expect(Math.ceil(light / 3)).toBe(Math.ceil(dark / 3));
  });

  it("draws its groups as named blocks rather than fieldsets", () => {
    // WebKit counts a legend's height twice in its fieldset when the screen is first drawn and
    // drops it at the next change inside, so a section of fieldsets stands taller than it is and
    // shrinks the first time anything in it is pressed.
    renderSettings();

    expect(document.querySelectorAll("fieldset")).toHaveLength(0);
    expect(screen.getByRole("group", { name: "Light or dark" })).toBeTruthy();
    expect(
      screen.getByRole("group", { name: "How tightly mail is listed" }),
    ).toBeTruthy();
  });

  it("shows which light scheme is worn", () => {
    renderSettings({ ...STORED, theme: "light", lightScheme: "rosePineDawn" });

    expect(
      screen
        .getByRole("button", { name: /Rosé Pine Dawn/ })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen.getByRole("button", { name: /Paper/ }).getAttribute("aria-pressed"),
    ).toBe("false");
  });

  it("shares nothing until the box is ticked, and writes the box the moment it is", async () => {
    // Off for somebody who never opened this screen: nothing leaves the device unasked.
    const { changed } = renderSettings();
    const box = screen.getByRole("checkbox", {
      name: "Share anonymous error reports",
    }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    // What is shared and what never is, read with the box.
    expect(
      document.getElementById(box.getAttribute("aria-describedby") ?? "")
        ?.textContent,
    ).toBe(
      "If something goes wrong, an anonymous report from Lyonoyl\u00a0Mail helps us fix it sooner. The report never includes your mail, addresses or passwords",
    );

    fireEvent.click(box);

    expect(sharing).toHaveBeenCalledWith(true);
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...STORED,
        shareAnonymousData: true,
      }),
    );
  });

  it("shows the box ticked for somebody who said yes, and unticks it on asking", () => {
    sharing.mockResolvedValue({ ...STORED, shareAnonymousData: false });
    renderSettings({ ...STORED, shareAnonymousData: true });
    const box = screen.getByRole("checkbox", {
      name: "Share anonymous error reports",
    }) as HTMLInputElement;
    expect(box.checked).toBe(true);

    fireEvent.click(box);

    expect(sharing).toHaveBeenCalledWith(false);
  });

  it("leaves the box as it was when the write did not happen", async () => {
    sharing.mockRejectedValue(new Error("no"));
    const { changed } = renderSettings();
    const box = screen.getByRole("checkbox", {
      name: "Share anonymous error reports",
    }) as HTMLInputElement;

    fireEvent.click(box);

    await vi.waitFor(() => expect(sharing).toHaveBeenCalled());
    expect(changed).not.toHaveBeenCalled();
    expect(box.checked).toBe(false);
  });

  it("leaves the light control where it was when the write did not happen", async () => {
    lightScheme.mockRejectedValue(new Error("no"));
    const { changed } = renderSettings({ ...STORED, theme: "light" });

    fireEvent.click(screen.getByRole("button", { name: /Rosé Pine Dawn/ }));

    await vi.waitFor(() => expect(lightScheme).toHaveBeenCalled());
    expect(changed).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: /Paper/ }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("names every language in itself, so somebody who cannot read this one can find theirs", async () => {
    renderSettings();

    const list = await openChooser("Interface language");

    const russian = within(list).getByRole("option", { name: "Русский" });
    expect(within(list).getByRole("option", { name: "English" })).toBeTruthy();
    // Said in its own voice by a screen reader, rather than spelt out in English phonetics.
    expect(russian.getAttribute("lang")).toBe("ru");
  });

  it("shows the language in use while the list is closed", async () => {
    renderSettings({ ...STORED, language: "ru" });

    const chooser = await screen.findByRole("combobox", {
      name: "Interface language",
    });

    expect(said(chooser)).toBe("Русский");
    expect(within(chooser).getByText("ru")).toBeTruthy();
  });

  it("shows following the system as the choice when that is what is stored", async () => {
    renderSettings({ ...STORED, language: "system" });

    const chooser = await screen.findByRole("combobox", {
      name: "Interface language",
    });

    expect(chooser.textContent).toBe("System");
  });

  it("offers every language there is, following the system first", async () => {
    // Every catalogue this build holds, and twenty made up for the test on top.
    const { changed } = renderSettings();

    const names = offered(await openChooser("Interface language"));
    const held = readdirSync(join(__dirname, "../../shared/i18n/locales")).filter(
      (file) => file.endsWith(".json"),
    ).length;

    expect(names[0]).toBe("System");
    expect(names).toHaveLength(1 + held + MADE_UP.length);
    expect(names).toContain("English");
    expect(names).toContain("Русский");
    expect(names).toContain("العربية");
    expect(names).toContain("日本語");
    for (let at = 1; at <= MADE_UP.length; at += 1) {
      expect(names, `language ${at}`).toContain(`Made-up language ${at}`);
    }
    // Alphabetically by the names they give themselves: the Latin ones first, from A to Z, and
    // the other scripts after them, the Han characters last. Each with its code before its name.
    const order = new Intl.Collator("en");
    const languages = names.slice(1).map(String);
    expect(languages).toEqual([...languages].sort(order.compare));
    expect(languages.indexOf("Русский")).toBeGreaterThan(
      languages.indexOf("Türkçe"),
    );
    expect(languages[languages.length - 1]).toBe("简体中文");
    const russian = screen.getByRole("option", { name: "Русский" });
    expect(within(russian).getByText("ru")).toBeTruthy();

    // The last of the made-up ones, which a list running off the window would have hidden.
    fireEvent.click(
      screen.getByRole("option", { name: "Made-up language 9" }),
    );

    expect(language).toHaveBeenCalledWith("x-made-up-09");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, language: "ru" }),
    );
  });

  it("changes the language at once", async () => {
    const { changed } = renderSettings();

    await openChooser("Interface language");
    fireEvent.click(screen.getByRole("option", { name: "Русский" }));

    expect(language).toHaveBeenCalledWith("ru");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, language: "ru" }),
    );
  });

  it("reaches a language from the keyboard by typing its first letter", async () => {
    // Twenty-odd entries are a long way to arrow through, and the person looking for their own
    // language knows how its name begins.
    renderSettings();

    const chooser = await screen.findByRole("combobox", {
      name: "Interface language",
    });
    chooser.focus();
    fireEvent.keyDown(chooser, { key: "Enter" });
    const list = await screen.findByRole("listbox");

    fireEvent.keyDown(list, { key: "Р" });
    const russian = within(list).getByRole("option", { name: "Русский" });
    await waitFor(() => expect(document.activeElement).toBe(russian));

    fireEvent.keyDown(russian, { key: "Enter" });

    expect(language).toHaveBeenCalledWith("ru");
  });

  it("writes nothing when the language already in use is chosen again", async () => {
    renderSettings();

    await openChooser("Interface language");
    fireEvent.click(screen.getByRole("option", { name: "English" }));

    expect(language).not.toHaveBeenCalled();
  });

  it("keeps the list inside the window, however short the window is", async () => {
    // Asserted as the rule rather than as the effect: this runs without layout, so nothing here
    // has a height. The list opens beside the control, is held to the room the window has on
    // that side, and scrolls inside itself past that.
    renderSettings();

    const list = await openChooser("Interface language");

    expect(list.getAttribute("data-side")).not.toBeNull();
    expect(list.className).toContain(
      "var(--radix-select-content-available-height)",
    );
    expect(list.className).toContain("overflow-y-auto");
  });

  it("closes the list on Escape and leaves the screen where it is", async () => {
    // The window leaves the screen on Escape. Shutting the list and the settings behind it with
    // one press would throw away everything somebody was looking at to take back that press.
    renderSettings();
    const list = await openChooser("Interface language");

    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    fireEvent.keyDown(within(list).getAllByRole("option")[0], {
      key: "Escape",
      bubbles: true,
    });
    window.removeEventListener("keydown", behind);

    expect(behind, "the key reached the window behind the list").not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
  });

  it("goes on showing the language in use when the write did not happen", async () => {
    language.mockRejectedValue(new Error("no"));
    const { changed } = renderSettings();

    await openChooser("Interface language");
    fireEvent.click(screen.getByRole("option", { name: "Русский" }));

    await vi.waitFor(() => expect(language).toHaveBeenCalled());
    expect(changed).not.toHaveBeenCalled();
    expect(
      said(screen.getByRole("combobox", { name: "Interface language" })),
    ).toBe("English");
  });

  it("changes how tightly mail is listed at once", async () => {
    const { changed } = renderSettings();

    fireEvent.click(screen.getByRole("button", { name: "Compact" }));

    expect(density).toHaveBeenCalledWith("compact");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, density: "compact" }),
    );
  });

  it("leaves the control where it was when the write did not happen", async () => {
    // Nothing reached the disk, so nothing changes on screen. Moving the control anyway would tell
    // somebody their choice was kept when it was not.
    scheme.mockRejectedValue(new Error("no"));
    const { changed } = renderSettings({ ...STORED, theme: "dark" });

    fireEvent.click(screen.getByRole("button", { name: /Dracula/ }));

    await vi.waitFor(() => expect(scheme).toHaveBeenCalled());
    expect(changed).not.toHaveBeenCalled();
    expect(
      screen
        .getByRole("button", { name: /Graphite/ })
        .getAttribute("aria-pressed"),
    ).toBe("true");
  });
});

describe("SettingsScreen, arranging mailboxes into groups", () => {
  const ACCOUNT: MailAccountSummaryDto = {
    id: "acc-1",
    email: "one@example.com",
    displayName: "One",
    provider: "imap",
    canSend: false,
  };

  it("refuses a group with no name before anything is asked of the core", async () => {
    renderSettings();

    const add = await screen.findByRole("button", { name: "Add group" });

    // Refused by the button being unavailable rather than by an error afterwards: a name that
    // draws as nothing is a row in the sidebar nobody can tell from the gap above it.
    expect((add as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("Add group"), {
      target: { value: "   " },
    });
    expect((add as HTMLButtonElement).disabled).toBe(true);
    expect(makeGroup).not.toHaveBeenCalled();
  });

  it("makes a group and tells the sidebar to draw itself again", async () => {
    const { rearranged } = renderSettings();

    fireEvent.change(await screen.findByLabelText("Add group"), {
      target: { value: "Work" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add group" }));

    await waitFor(() => expect(makeGroup).toHaveBeenCalledWith("Work"));
    // The sidebar is drawn beside this screen rather than under it, so a group made here reaches
    // it only if it is told. Without this, a new group appeared after a restart and not before.
    await waitFor(() => expect(rearranged).toHaveBeenCalled());
  });

  it("removes a group only once its name is typed, never on a near miss", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);
    const { rearranged } = renderSettings();

    fireEvent.click(
      await screen.findByRole("button", { name: "Remove the group Work" }),
    );
    const panel = await screen.findByRole("dialog", { name: "Remove group" });
    const field = within(panel).getByRole("textbox", { name: "Group name" });
    const remove = within(panel).getByRole("button", {
      name: "Remove",
    }) as HTMLButtonElement;

    fireEvent.change(field, { target: { value: "work" } });
    expect(remove.disabled).toBe(true);
    fireEvent.submit(field.closest("form")!);
    expect(dropGroup).not.toHaveBeenCalled();

    arrangement.mockResolvedValue([]);
    fireEvent.change(field, { target: { value: "Work" } });
    fireEvent.click(remove);

    await waitFor(() => expect(dropGroup).toHaveBeenCalledWith("g1"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByDisplayValue("Work")).toBeNull();
    expect(rearranged).toHaveBeenCalled();
  });

  it("says where the mailboxes go only for a group holding some of this device's", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: ["acc-1"] },
      // Names only a mailbox this device no longer holds, so there is nothing to move.
      { id: "g2", name: "Old", place: 1, collapsed: false, accounts: ["gone"] },
    ]);
    renderSettings(STORED, [
      {
        id: "acc-1",
        email: "only@example.com",
        displayName: "Only",
        provider: "imap",
        canSend: false,
      },
    ]);
    const moving = "The mailboxes in this group stay in the sidebar where the group was, with all their mail";

    fireEvent.click(
      await screen.findByRole("button", { name: "Remove the group Work" }),
    );
    expect(
      within(await screen.findByRole("dialog")).getByText(moving),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    fireEvent.click(screen.getByRole("button", { name: "Remove the group Old" }));
    const panel = await screen.findByRole("dialog");
    expect(within(panel).queryByText(moving)).toBeNull();
    expect(within(panel).getByText("To remove the group, type its name")).toBeTruthy();
  });

  it("keeps a group the panel was left without removing, back on its Remove", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);
    renderSettings();
    const asked = await screen.findByRole("button", {
      name: "Remove the group Work",
    });
    asked.focus();

    fireEvent.click(asked);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(dropGroup).not.toHaveBeenCalled();
    expect(screen.getByDisplayValue("Work")).toBeTruthy();
    expect(document.activeElement).toBe(asked);
  });

  it("reads the groups again when one was made from the sidebar", async () => {
    const shown = (madeInTheSidebar: number) => (
      <I18nProvider initial="en">
        <ScreenPane
          paneRef={{ current: null }}
          closeLabel="Close this screen"
          onClose={vi.fn()}
        >
          <SettingsScreen
            preferences={STORED}
            onChanged={vi.fn()}
            accounts={[]}
            onOpenAccount={vi.fn()}
            arrangedElsewhere={madeInTheSidebar}
          />
        </ScreenPane>
      </I18nProvider>
    );
    const { rerender } = render(shown(0));
    await waitFor(() => expect(arrangement).toHaveBeenCalled());
    const readBefore = arrangement.mock.calls.length;
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);

    rerender(shown(1));

    expect(await screen.findByDisplayValue("Work")).toBeTruthy();
    expect(arrangement.mock.calls.length).toBe(readBefore + 1);
  });

  it("empties the field once the group is made, so the next one starts blank", async () => {
    renderSettings();

    const field = await screen.findByLabelText("Add group");
    // Empty, it runs the window's way; with a letter in it, the letters decide.
    expect(field.getAttribute("dir")).toBeNull();
    fireEvent.change(field, { target: { value: "Work" } });
    expect(field.getAttribute("dir")).toBe("auto");
    fireEvent.click(screen.getByRole("button", { name: "Add group" }));

    await waitFor(() => expect((field as HTMLInputElement).value).toBe(""));
  });

  it("gives up an edited name on Escape and keeps the key to itself", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);
    renderSettings();

    const field = await screen.findByLabelText("Group name");
    fireEvent.change(field, { target: { value: "Projects" } });

    // The window listens on `window`, so what matters is whether the key ever gets there.
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    fireEvent.keyDown(field, { key: "Escape", bubbles: true });
    window.removeEventListener("keydown", behind);

    expect((field as HTMLInputElement).value).toBe("Work");
    expect(behind, "the key reached the window behind it").not.toHaveBeenCalled();
    expect(renameGroup).not.toHaveBeenCalled();
  });

  it("lets Escape past when there is no edit to give up", async () => {
    // Taken unconditionally it would be a dead key in one field of one screen: nothing reverts,
    // and the screen it is on does not close either.
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);
    renderSettings();

    const field = await screen.findByLabelText("Group name");
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    fireEvent.keyDown(field, { key: "Escape", bubbles: true });
    window.removeEventListener("keydown", behind);

    expect(behind, "the key was stopped with nothing to give up").toHaveBeenCalled();
  });

  it("writes a new name down when the field is left, and not on every letter", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);

    renderSettings();

    const field = await screen.findByLabelText("Group name");
    fireEvent.change(field, { target: { value: "Projects" } });

    // Nothing yet: a rename per keystroke is a row of half-typed names in the sidebar beside it.
    expect(renameGroup).not.toHaveBeenCalled();

    fireEvent.blur(field);
    await waitFor(() =>
      expect(renameGroup).toHaveBeenCalledWith("g1", "Projects"),
    );
  });

  it("keeps room after the last part so it can come to the top", async () => {
    // The parts below the fold are reached by pressing their name in the column beside this
    // screen, and a screen that stops with the last of them still low down cannot put them where
    // that column is looking. Half a pane, so the last part comes to rest around the middle
    // rather than being thrown against the top. Asserted as the rule rather than as the effect:
    // this runs without layout, so nothing here can scroll.
    renderSettings();
    await screen.findByRole("heading", { name: "Settings" });

    const body = document.querySelector("[data-settings-body]");

    expect(body?.className).toContain("pb-[50dvh]");
  });

  it("is made of exactly the parts the list beside it names, in that order", async () => {
    // The list in the column beside this screen is drawn from `SETTINGS_SECTIONS`, and scrolls to
    // what it names. A part added here and not added there is a part of the settings that list
    // cannot reach; one renamed on one side only is two names for the same thing.
    renderSettings();
    await screen.findByRole("heading", { name: "Settings" });

    const drawn = [
      ...document.querySelectorAll("section[id^='settings-']"),
    ].map((part) => part.id);

    expect(drawn).toEqual(SETTINGS_SECTIONS.map((one) => anchorFor(one.id)));
    // And the licences at the end of what is drawn, whatever the list says.
    expect(drawn[drawn.length - 1]).toBe(anchorFor("licences"));
  });

  it("writes a half-typed name down on the way out", async () => {
    // Leaving by the cross is leaving. The field writes what it holds when it loses the focus,
    // and a button does not always take the focus when it is pressed -- on macOS it does not --
    // so without the pane taking it away by hand the new name went nowhere and the screen closed
    // over it.
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);

    const { closed } = renderSettings();

    const field = await screen.findByLabelText("Group name");
    field.focus();
    fireEvent.change(field, { target: { value: "Projects" } });
    expect(renameGroup).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Close this screen" }));

    await waitFor(() =>
      expect(renameGroup).toHaveBeenCalledWith("g1", "Projects"),
    );
    expect(closed).toHaveBeenCalled();
  });

  it("keeps the stored name when somebody empties the field and leaves it", async () => {
    arrangement.mockResolvedValue([
      { id: "g1", name: "Work", place: 0, collapsed: false, accounts: [] },
    ]);

    renderSettings();

    const field = await screen.findByLabelText("Group name");
    fireEvent.change(field, { target: { value: "  " } });
    fireEvent.blur(field);

    expect(renameGroup).not.toHaveBeenCalled();
    // And the field shows the name that is stored, rather than the blank that was refused.
    expect((field as HTMLInputElement).value).toBe("Work");
  });

  /** A core that remembers where each mailbox was put, so the screen reads back what it wrote. */
  function arrangedAs(
    groups: Array<{ id: string; name: string }>,
    placed: Record<string, string | null> = {},
  ) {
    arrangement.mockImplementation(async () =>
      groups.map((group) => ({
        ...group,
        place: 0,
        collapsed: false,
        accounts: Object.keys(placed).filter(
          (account) => placed[account] === group.id,
        ),
      })),
    );
    placeAccount.mockImplementation(async (account, group) => {
      placed[account] = group;
    });
  }

  it("puts a mailbox in a group, and takes it out again", async () => {
    arrangedAs([{ id: "g1", name: "Work" }]);

    renderSettings(STORED, [ACCOUNT]);

    await openChooser("The group One is in");
    fireEvent.click(screen.getByRole("option", { name: "Work" }));

    await waitFor(() =>
      expect(placeAccount).toHaveBeenCalledWith("acc-1", "g1"),
    );
    // And the closed control says where it is now, from what the core read back.
    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "The group One is in" }).textContent,
      ).toBe("Work"),
    );

    // Out of every group rather than into a particular one: a mailbox is in at most one group,
    // so there is no state where it is in both and none where it is in neither by accident.
    await openChooser("The group One is in");
    fireEvent.click(screen.getByRole("option", { name: "No group" }));
    await waitFor(() =>
      expect(placeAccount).toHaveBeenCalledWith("acc-1", null),
    );
  });

  it("moves the mailbox of the row it stands in, into the group chosen", async () => {
    // Two of each, and the second of each chosen: a chooser wired to the first row or to the
    // first group would pass a test that only ever had one.
    arrangedAs(
      [
        { id: "g1", name: "Work" },
        { id: "g2", name: "Home" },
      ],
      { "acc-1": "g1" },
    );

    renderSettings(STORED, [
      ACCOUNT,
      {
        id: "acc-2",
        email: "sigma@example.org",
        displayName: "Sigma Ordrin",
        provider: "imap",
        canSend: true,
      },
    ]);

    const list = await openChooser("The group Sigma Ordrin is in");
    // Nothing to be in yet comes first, then the groups in their own order.
    expect(offered(list)).toEqual(["No group", "Work", "Home"]);
    fireEvent.click(within(list).getByRole("option", { name: "Home" }));

    await waitFor(() =>
      expect(placeAccount).toHaveBeenCalledWith("acc-2", "g2"),
    );
    expect(placeAccount).toHaveBeenCalledTimes(1);
  });

  it("writes nothing when the group a mailbox is already in is chosen again", async () => {
    arrangedAs([{ id: "g1", name: "Work" }], { "acc-1": "g1" });

    renderSettings(STORED, [ACCOUNT]);

    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "The group One is in" }).textContent,
      ).toBe("Work"),
    );
    await openChooser("The group One is in");
    fireEvent.click(screen.getByRole("option", { name: "Work" }));

    expect(placeAccount).not.toHaveBeenCalled();
  });

  it("chooses a mailbox's group in the mailbox's own row, just before its pencil", async () => {
    arrangedAs([{ id: "g1", name: "Work" }]);

    renderSettings(STORED, [ACCOUNT]);

    const chooser = await screen.findByRole("combobox", {
      name: "The group One is in",
    });
    const pencil = screen.getByRole("button", { name: "Edit One" });
    const row = pencil.closest("li");

    expect(row?.contains(chooser), "not in the mailbox's row").toBe(true);
    expect(
      chooser.compareDocumentPosition(pencil) &
        Node.DOCUMENT_POSITION_FOLLOWING,
      "the pencil does not come after the chooser",
    ).toBeTruthy();
  });

  it("keeps the groups to themselves, without a second list of the mailboxes", async () => {
    arrangedAs([{ id: "g1", name: "Work" }]);

    renderSettings(STORED, [ACCOUNT]);

    const groups = await screen.findByRole("region", { name: "Groups" });
    await within(groups).findByLabelText("Group name");

    expect(within(groups).queryByText("One")).toBeNull();
    expect(within(groups).queryAllByRole("combobox")).toEqual([]);
  });

  it("says a refused move beside the mailboxes, where it was asked from", async () => {
    // The chooser stands in the mailbox's row. Said at the top of the groups, above, the refusal
    // is out of sight of somebody scrolled down to the mailboxes, who sees a choice snap back.
    arrangedAs([{ id: "g1", name: "Work" }]);
    placeAccount.mockRejectedValueOnce(new Error("no"));

    renderSettings(STORED, [ACCOUNT]);

    await openChooser("The group One is in");
    fireEvent.click(screen.getByRole("option", { name: "Work" }));

    const mailboxes = screen.getByRole("region", { name: "Mailboxes" });
    const said = await within(mailboxes).findByRole("alert");
    expect(said.textContent).toBe("The change could not be saved");
    const groups = screen.getByRole("region", { name: "Groups" });
    expect(within(groups).queryByRole("alert")).toBeNull();

    // Gone once a move goes through, rather than left standing over a mailbox that moved.
    await openChooser("The group One is in");
    fireEvent.click(screen.getByRole("option", { name: "Work" }));
    await waitFor(() =>
      expect(within(mailboxes).queryByRole("alert")).toBeNull(),
    );
    expect(placeAccount).toHaveBeenCalledTimes(2);
  });

  it("says a refused group change in the groups, where it was asked from", async () => {
    makeGroup.mockRejectedValueOnce(new Error("no"));

    renderSettings(STORED, [ACCOUNT]);

    fireEvent.change(await screen.findByLabelText("Add group"), {
      target: { value: "Work" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add group" }));

    const groups = screen.getByRole("region", { name: "Groups" });
    expect((await within(groups).findByRole("alert")).textContent).toBe(
      "The change could not be saved",
    );
    const mailboxes = screen.getByRole("region", { name: "Mailboxes" });
    expect(within(mailboxes).queryByRole("alert")).toBeNull();
  });

  it("says the groups could not be read rather than showing none", async () => {
    arrangement.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });

    renderSettings(STORED, [ACCOUNT]);

    // An empty list here would be a screen saying somebody has no groups, in front of somebody
    // who has several and cannot see them.
    expect(await screen.findByText(/groups could not be read/i)).toBeTruthy();
  });

  it("offers no chooser when there is no group to choose", async () => {
    renderSettings(STORED, [ACCOUNT]);

    await screen.findByRole("button", { name: "Add group" });

    // A menu whose only entry is "No group" is a control that cannot do anything.
    expect(screen.getByRole("button", { name: "Edit One" })).toBeTruthy();
    expect(screen.queryByRole("combobox", { name: "The group One is in" })).toBeNull();
  });
});

describe("inRoundNumbers", () => {
  // The real translator, so the unit shown is the one the catalog holds rather than one written
  // here: a test with its own copy of the units would pass while the catalog said something else.
  const english = createTranslator("en");

  it("says bytes the way a person says them", () => {
    expect(inRoundNumbers("0", "en", english)).toBe("0 B");
    expect(inRoundNumbers("999", "en", english)).toBe("999 B");
    // Powers of a thousand, because that is what the operating system shows beside the same file
    // on two of the three systems this ships on.
    expect(inRoundNumbers("1000", "en", english)).toBe("1 kB");
    expect(inRoundNumbers("1500", "en", english)).toBe("1.5 kB");
    expect(inRoundNumbers("2500000", "en", english)).toBe("2.5 MB");
  });

  it("holds a number larger than a JavaScript number holds exactly", () => {
    // Ninety million million bytes is past 2^53. Read as a number on the way in, this would be a
    // figure that is quietly wrong — which is why the count crosses as text.
    const huge = "9007199254740993";

    expect(inRoundNumbers(huge, "en", english)).toBe("9 PB");
  });

  it("stops at the largest unit it knows rather than inventing one", () => {
    expect(inRoundNumbers("1000000000000000000000000", "en", english)).toMatch(/PB$/);
  });
});

describe("SettingsScreen, how much is held", () => {
  const HELD = [
    {
      accountId: "acc-b",
      email: "b@example.com",
      displayName: "Work",
      bytes: "2500000",
    },
    {
      accountId: "acc-a",
      email: "a@example.com",
      displayName: "",
      bytes: "1500000",
    },
  ];

  it("shows what each mailbox takes, and what they take together", async () => {
    holding.mockResolvedValue(HELD);

    renderSettings();

    // Nothing is claimed before the answer arrives.
    expect(screen.queryByText("How much is held could not be read")).toBeNull();

    expect(await screen.findByText("2.5 MB")).toBeTruthy();
    expect(screen.getByText("1.5 MB")).toBeTruthy();
    // The total is the point of the screen: a person deciding whether to clear anything is asking
    // about the whole, not about one mailbox. Four, which is neither of the two rows — so this
    // cannot pass by finding one of them again.
    expect(screen.getByText(/Altogether/)).toBeTruthy();
    expect(screen.getByText("4 MB")).toBeTruthy();
  });

  it("says it could not be read rather than showing nothing held", async () => {
    // Zero in front of somebody whose mailbox plainly has mail in it is the screen lying about
    // the one number it exists to show.
    holding.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });

    renderSettings();

    // Named exactly. Four strings in the catalogue end "could not be read", and an assertion that
    // matched any of them passed while this screen said something else entirely.
    expect(
      await screen.findByText("How much is held could not be read"),
    ).toBeTruthy();
  });
});

describe("SettingsScreen, being told that mail arrived", () => {
  it("turns it off, and the screen shows what the core stored", async () => {
    const { changed } = renderSettings();

    fireEvent.click(await screen.findByRole("button", { name: "Never" }));

    await waitFor(() => expect(notice).toHaveBeenCalledWith("never"));
    // The whole of the settings as the core stored them, not what the button assumed: a screen
    // that showed the press rather than the answer would lie whenever a write failed.
    await waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...STORED,
        newMailNotice: "never",
      }),
    );
  });

  it("writes down the answer about pictures once, for every message after it", async () => {
    const { changed } = renderSettings();

    fireEvent.click(await screen.findByRole("button", { name: "Show them" }));

    await waitFor(() => expect(pictures).toHaveBeenCalledWith("always"));
    await waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...STORED,
        showPictures: "always",
      }),
    );
  });

  it("offers looking for updates as one box, empty for somebody nobody asked", async () => {
    // Nobody having been asked looks for nothing, and the box says what happens rather than what
    // might be proposed.
    const { changed } = renderSettings();
    const box = (await screen.findByRole("checkbox", {
      name: "Check for updates automatically",
    })) as HTMLInputElement;
    expect(box.checked).toBe(false);
    expect(
      document.getElementById(box.getAttribute("aria-describedby") ?? "")
        ?.textContent,
    ).toBe(
      "Lyonoyl\u00a0Mail checks for new versions on its own and installs one only when you say so",
    );

    fireEvent.click(box);

    await waitFor(() => expect(lookingFor).toHaveBeenCalledWith("onceADay"));
    await waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...STORED,
        lookForUpdates: "onceADay",
        shareAnonymousData: false,
        welcomed: true,
      }),
    );
  });

  it("stops looking when the box is ticked off", async () => {
    lookingFor.mockResolvedValue({ ...STORED, lookForUpdates: "never" });
    renderSettings({ ...STORED, lookForUpdates: "onceADay" });
    const box = (await screen.findByRole("checkbox", {
      name: "Check for updates automatically",
    })) as HTMLInputElement;
    expect(box.checked).toBe(true);

    fireEvent.click(box);

    await waitFor(() => expect(lookingFor).toHaveBeenCalledWith("never"));
  });

  it("offers the box in every build, with nothing said about how it was built", async () => {
    renderSettings();
    await screen.findByRole("heading", { name: "Updates" });

    expect(
      screen.getByRole("checkbox", { name: "Check for updates automatically" }),
    ).toBeTruthy();
    expect(screen.queryByText(/built without the key/i)).toBeNull();
  });

  it("says what fetching a picture tells the machine it is fetched from", async () => {
    // The cost of the answer, in front of the person choosing it. Without it "show them" reads as
    // a convenience rather than as agreeing to be counted by whoever sent the message.
    renderSettings();

    const hint = await screen.findByText(/somebody else.s machine/i);
    expect(hint.textContent).toMatch(/opened just now/i);
    // And the half that stays true either way, so the choice is not read as one about safety.
    expect(hint.textContent).toMatch(/never reaches the network/i);
  });

  it("shows the log folder rather than naming it", async () => {
    // A log whose folder somebody has to know the name of is a log nobody sends -- and on two of
    // the three systems that folder is inside one the system hides. The path is never handed to
    // this screen either: which folder it is belongs to the core, and a screen that knew would be
    // a screen that could be told.
    renderSettings();

    fireEvent.click(
      await screen.findByRole("button", { name: "Show the log folder" }),
    );

    await waitFor(() => expect(theJournal).toHaveBeenCalledTimes(1));
    expect(theJournal).toHaveBeenCalledWith();
  });

  it("says so when the system will not open the folder", async () => {
    // A press that does nothing and says nothing is a dead control, and this one can fail: a
    // system with nothing registered for a folder answers with a refusal and no window.
    theJournal.mockRejectedValue(new Error("nothing opens folders here"));
    renderSettings();

    fireEvent.click(
      await screen.findByRole("button", { name: "Show the log folder" }),
    );

    const said = await screen.findByRole("alert");
    expect(said.textContent).toMatch(/would not open that folder/i);
  });

  it("opens the project's page for problems and questions", async () => {
    renderSettings();

    fireEvent.click(
      await screen.findByRole("button", { name: "Write a report" }),
    );
    await waitFor(() => expect(supportPage).toHaveBeenCalledTimes(1));
    // The window names nothing to open; the address is the core's.
    expect(supportPage).toHaveBeenCalledWith();
  });

  it("names the version this copy is", async () => {
    renderSettings();

    expect(await screen.findByText("This is version 1.2.3")).toBeTruthy();
  });

  it("draws no version at all when the build cannot say which it is", async () => {
    let refuse: (why: Error) => void = () => {};
    builtAs.mockReturnValue(
      new Promise<string>((_, reject) => {
        refuse = reject;
      }),
    );
    renderSettings();
    await screen.findByRole("heading", { name: "Updates" });

    // The refusal handled before anything is asserted, so what is checked is what is drawn after.
    await act(async () => refuse(new Error("no version")));
    expect(screen.queryByText(/This is version/)).toBeNull();
    expect(screen.getByRole("heading", { name: "Updates" })).toBeTruthy();
  });

  it("says what the log holds and what it never holds", async () => {
    // Somebody is about to attach it to a report. What stops that being a leak is that there is
    // nothing in it to leak, and they are owed that sentence before they send it rather than after.
    renderSettings();

    const hint = await screen.findByText(/record of what happened/i);
    expect(hint.textContent).toMatch(/never holds an address, a subject or a word of a message/i);
    expect(hint.textContent).toMatch(/only ever on this device/i);
  });

  it("says who wrote, what about and how it opens, and each can be turned off", async () => {
    // What a notice says by default. One saying only that mail arrived does not answer the
    // question somebody looks at it to answer, so they open the application to find out — which is
    // the interruption it was there to save them.
    renderSettings();

    for (const name of [/who it is from/i, /what it is about/i, /how the message begins/i]) {
      const part = await screen.findByRole("button", { name });
      expect(part.getAttribute("aria-pressed"), `${name}`).toBe("true");
    }
  });

  it("says where a notification is drawn, because that is what the switches are for", async () => {
    // A locked screen is the whole reason somebody would turn one of these off, and it is not a
    // thing they can be expected to think of while reading the word "sender".
    renderSettings();

    expect(
      await screen.findByText(/locked screen/i),
      "nothing said why a person might want one of these off",
    ).toBeTruthy();
  });

  it("turning one part off leaves the other two alone", async () => {
    // The three travel together on the way out, so the two that were not pressed have to be sent
    // back as they were. Sent as a bare {sender: false} they would read as off.
    // One of the two is already off, and that is the whole fixture: with all three on, a screen
    // that sent its own three defaults instead of carrying the current ones would send the same
    // thing and this would pass about nothing.
    const held = {
      ...STORED,
      noticeShows: { sender: true, subject: false, preview: true },
    };
    parts.mockResolvedValue({
      ...held,
      noticeShows: { sender: false, subject: false, preview: true },
    });

    const { changed } = renderSettings(held);

    fireEvent.click(
      await screen.findByRole("button", { name: /who it is from/i }),
    );

    expect(parts).toHaveBeenCalledWith({
      sender: false,
      subject: false,
      preview: true,
    });

    // And what came back is handed on, so the screen wears what was stored rather than its own
    // idea of it.
    await waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...held,
        noticeShows: { sender: false, subject: false, preview: true },
      }),
    );
  });

  it("does not offer the parts of a notice nobody will ever see", async () => {
    // With notices off these decide the wording of something that never appears, which is a dead
    // control dressed as a live one.
    renderSettings({ ...STORED, newMailNotice: "never" });

    await screen.findByRole("button", { name: /never/i });

    expect(screen.queryByRole("button", { name: /who it is from/i })).toBeNull();
  });
});

describe("SettingsScreen, the licences", () => {
  const MIT = {
    licence: "MIT License",
    text: "MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy...",
    used: ["react", "react-dom"],
  };

  it("does not read four hundred kilobytes into a screen nobody asked it for", async () => {
    renderSettings();
    await screen.findByRole("button", { name: /show the licenses/i });

    expect(notices).not.toHaveBeenCalled();
  });

  it("shows every notice word for word, and what each one covers", async () => {
    // Word for word because that is the only thing every one of these licences asks for. A
    // summary is not a licence.
    notices.mockResolvedValue([MIT]);

    renderSettings();
    fireEvent.click(
      await screen.findByRole("button", { name: /show the licenses/i }),
    );

    expect(
      await screen.findByText(/Permission is hereby granted/),
    ).toBeTruthy();
    expect(screen.getByText("react, react-dom")).toBeTruthy();
    expect(screen.getByText(/Packages: 2\. License texts: 1/)).toBeTruthy();
    // In English as written, so left to right in a window that runs the other way.
    expect(
      screen.getByText(/Permission is hereby granted/).closest("li")?.getAttribute("dir"),
    ).toBe("ltr");
  });

  it("counts a package named under two licences once", async () => {
    // Packages, not the places a package is named: one offered under two licences is one package.
    notices.mockResolvedValue([
      MIT,
      {
        licence: "Apache License 2.0",
        text: "Apache License\nVersion 2.0, January 2004",
        used: ["react", "scheduler"],
      },
    ]);

    renderSettings();
    fireEvent.click(
      await screen.findByRole("button", { name: /show the licenses/i }),
    );

    expect(
      await screen.findByText(/Packages: 3\. License texts: 2/),
    ).toBeTruthy();
  });

  it("offers the pictures under the look where the system can change them, the stored one chosen", async () => {
    wearable.mockResolvedValue(true);
    renderSettings({ ...STORED, theme: "light" });

    const offered = await offeredPictures();

    expect(offered.map((one) => one.getAttribute("data-app-icon"))).toEqual([
      "ink",
      "indigo",
      "dusk",
      "plum",
      "graphite",
      "sky",
      "paper",
      "ember",
      "rust",
    ]);
    expect(pressedPictures(offered)).toEqual(["ember"]);
    for (const one of offered) expect(one.textContent).toBe("");
    // Under the look, after its colours.
    const look = document.getElementById(anchorFor("appearance"))!;
    expect(look.contains(offered[0])).toBe(true);
    const colours = screen.getByRole("group", { name: "Light colors" });
    expect(
      colours.compareDocumentPosition(offered[0]) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it.each([
    ["cannot change the picture", () => wearable.mockResolvedValue(false)],
    ["cannot say whether it can", () => wearable.mockRejectedValue(new Error("no answer"))],
  ])("offers no pictures on a system that %s", async (_, answer) => {
    answer();
    renderSettings();
    await vi.waitFor(() => expect(wearable).toHaveBeenCalled());
    await act(() => new Promise<void>((done) => setTimeout(done, 0)));

    // The look is drawn, and the pictures are not part of it.
    expect(screen.getByRole("group", { name: "Light or dark" })).toBeTruthy();
    expect(screen.queryByRole("group", { name: "App icon" })).toBeNull();
    expect(screen.queryByText("App icon")).toBeNull();
    expect(document.querySelector("[data-app-icon]")).toBeNull();
  });

  it("writes a picture the moment it is chosen, and wears what came back", async () => {
    wearable.mockResolvedValue(true);
    const { changed } = renderSettings();

    fireEvent.click(
      (await offeredPictures()).find(
        (one) => one.getAttribute("data-app-icon") === "rust",
      )!,
    );

    expect(wearing).toHaveBeenCalledTimes(1);
    expect(wearing).toHaveBeenCalledWith("rust");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({ ...STORED, appIcon: "rust" }),
    );
    expect(theme).not.toHaveBeenCalled();
  });

  it("leaves the picture as it was when the core cannot keep the one chosen", async () => {
    wearable.mockResolvedValue(true);
    wearing.mockRejectedValue(new Error("refused"));
    const { changed } = renderSettings();

    fireEvent.click(
      (await offeredPictures()).find(
        (one) => one.getAttribute("data-app-icon") === "rust",
      )!,
    );
    await act(() => new Promise<void>((done) => setTimeout(done, 0)));

    expect(changed).not.toHaveBeenCalled();
    expect(pressedPictures(await offeredPictures())).toEqual(["ember"]);
  });

  it("draws the same pictures when the system turns from dark to light", async () => {
    wearable.mockResolvedValue(true);
    const system = theSystemIs(true);
    renderSettings({ ...STORED, theme: "system" });

    const sources = async () =>
      (await offeredPictures()).map((one) =>
        one.querySelector("img")!.getAttribute("src"),
      );
    const dark = await sources();
    expect(dark).toHaveLength(9);
    expect(dark[7]).toMatch(/\/ember\.png$/);

    act(() => system.turns(false));

    expect(await sources()).toEqual(dark);
  });
});
