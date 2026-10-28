import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { AppIconDto } from "@/shared/ipc/generated/AppIconDto";
import type { LookForUpdatesDto } from "@/shared/ipc/generated/LookForUpdatesDto";
import type { PreferencesDto } from "@/shared/ipc/generated/PreferencesDto";
import { Onboarding, widestLine } from "./Onboarding";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    setShareAnonymousData: vi.fn(),
    setLookForUpdates: vi.fn(),
    setWelcomed: vi.fn(),
    appIconsWearable: vi.fn(),
    setAppIcon: vi.fn(),
  };
});

const client = await import("@/shared/ipc/client");
const sharing = vi.mocked(client.setShareAnonymousData);
const looking = vi.mocked(client.setLookForUpdates);
const welcoming = vi.mocked(client.setWelcomed);
const wearable = vi.mocked(client.appIconsWearable);
const wearing = vi.mocked(client.setAppIcon);

/** A profile nobody has answered anything in: nothing shared, nobody asked about updates. */
const FRESH: PreferencesDto = {
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
  welcomed: false,
};

/**
 * What the core holds, as the core would answer: every write comes back as the whole of the
 * settings, the earlier writes included.
 */
let held: PreferencesDto = FRESH;

beforeEach(() => {
  held = FRESH;
  sharing.mockReset();
  looking.mockReset();
  sharing.mockImplementation(async (shareAnonymousData: boolean) => {
    held = { ...held, shareAnonymousData };
    return held;
  });
  looking.mockImplementation(async (lookForUpdates: LookForUpdatesDto) => {
    held = { ...held, lookForUpdates };
    return held;
  });
  welcoming.mockReset();
  welcoming.mockImplementation(async (welcomed: boolean) => {
    held = { ...held, welcomed };
    return held;
  });
  // A system that cannot change the picture of a running application, unless a test says otherwise.
  wearable.mockReset();
  wearable.mockResolvedValue(false);
  wearing.mockReset();
  wearing.mockImplementation(async (appIcon: AppIconDto) => {
    held = { ...held, appIcon };
    return held;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

function renderOnboarding(stored: PreferencesDto | null = FRESH) {
  if (stored !== null) held = stored;
  const changed = vi.fn();
  const done = vi.fn();
  const drawn = (preferences: PreferencesDto | null) => (
    <I18nProvider initial="en">
      <Onboarding preferences={preferences} onChanged={changed} onDone={done} />
    </I18nProvider>
  );
  const { rerender } = render(drawn(stored));
  return {
    changed,
    done,
    arrive: (preferences: PreferencesDto) => rerender(drawn(preferences)),
  };
}

/** The cover over the whole window. */
const cover = () => document.querySelector("[data-onboarding]");

const press = (name: string) =>
  fireEvent.click(screen.getByRole("button", { name }));

const toTheLastStep = () => {
  press("Next");
  press("Next");
  press("Next");
};

/** The onboarding on a system that lets the picture change, once that system has said so. */
async function renderWearable(stored: PreferencesDto = { ...FRESH, theme: "light" }) {
  wearable.mockResolvedValue(true);
  const drawn = renderOnboarding(stored);
  await screen.findByText("Step 1 of 5");
  return drawn;
}

const toThePicture = () => {
  toTheLastStep();
  press("Next");
};

/** The pictures offered, in their order. */
const tiles = () =>
  within(screen.getByRole("group", { name: "Pick an icon" })).getAllByRole(
    "button",
  );

const tile = (value: AppIconDto) =>
  tiles().find((one) => one.getAttribute("data-app-icon") === value)!;

/** Which of the pictures are pressed, by what they are. */
const pressed = () =>
  tiles()
    .filter((one) => one.getAttribute("aria-pressed") === "true")
    .map((one) => one.getAttribute("data-app-icon"));

/** The picture over the heading of the step shown. */
const overTheHeading = () =>
  document.querySelector('section[data-step="shown"]')!.firstElementChild!;

/** A box as the engine reports one, from its left edge to its right. */
const box_ = (left: number, right: number) =>
  ({
    left,
    right,
    x: left,
    width: right - left,
    top: 0,
    bottom: 20,
    y: 0,
    height: 20,
    toJSON: () => ({}),
  }) as DOMRect;

/**
 * Lays out the text the way an engine would: each text, by what it says, gets the boxes of its lines.
 * happy-dom lays out nothing, so without this every line is nowhere.
 */
function linesOf(boxes: (text: string) => DOMRect[]) {
  const asked = function (this: Range) {
    return boxes(this.startContainer.textContent ?? "") as unknown as DOMRectList;
  };
  if (typeof Range.prototype.getClientRects === "function") {
    vi.spyOn(Range.prototype, "getClientRects").mockImplementation(asked);
  } else {
    Range.prototype.getClientRects = asked;
  }
}

/** Lets every answer already given be handled, and what it changed be drawn. */
const settle = () => act(() => new Promise<void>((done) => setTimeout(done, 0)));

const box = (name: string) =>
  screen.getByRole("checkbox", { name }) as HTMLInputElement;

const KEEP_TO_MYSELF = "I do not want to share anonymous error reports";
const LOOK_FOR_UPDATES = "Check for updates automatically";

describe("Onboarding", () => {
  it("opens on the welcome, named by it, with nothing in the focus", () => {
    // Not opened from the keyboard, so nothing wears a ring at the start; Tab reaches the buttons.
    renderOnboarding();

    expect(
      screen.getByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" }),
    ).toBeTruthy();
    expect(document.activeElement).toBe(document.body);
    expect(screen.getByText("Step 1 of 4")).toBeTruthy();
    // Nothing to go back to from the first step.
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
  });

  it("walks through the four steps one at a time, and back", () => {
    renderOnboarding();

    press("Next");
    expect(
      screen.getByRole("dialog", { name: "How Lyonoyl\u00a0Mail protects you" }),
    ).toBeTruthy();
    expect(screen.getByText("Step 2 of 4")).toBeTruthy();

    press("Next");
    expect(
      screen.getByRole("dialog", { name: "Anonymous error reports" }),
    ).toBeTruthy();

    press("Next");
    expect(screen.getByRole("dialog", { name: "Stay up to date" })).toBeTruthy();
    expect(screen.getByText("Step 4 of 4")).toBeTruthy();
    // The last step finishes rather than going on.
    expect(screen.queryByRole("button", { name: "Next" })).toBeNull();
    expect(screen.getByRole("button", { name: "Finish" })).toBeTruthy();

    press("Back");
    expect(
      screen.getByRole("dialog", { name: "Anonymous error reports" }),
    ).toBeTruthy();
  });

  it("gives the focus back to the way forward when the way back goes out of sight", () => {
    renderOnboarding();
    const forward = screen.getByRole("button", { name: "Next" });

    press("Next");
    // Pressed from the keyboard, so the focus is on it when it goes out of sight.
    const back = screen.getByRole("button", { name: "Back" });
    back.focus();
    fireEvent.click(back);

    expect(document.activeElement).toBe(forward);
  });

  it("leaves the focus where it was when the way back is pressed by the pointer", () => {
    renderOnboarding();

    press("Next");
    press("Back");

    expect(document.activeElement).toBe(document.body);
  });

  it("says eight things Lyonoyl Mail does for a person, in order", () => {
    renderOnboarding();
    press("Next");

    const step = screen.getByRole("region", {
      name: "How Lyonoyl\u00a0Mail protects you",
    });
    expect(
      within(step)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual([
      "Your mail is kept on your computer, so you can read it offline",
      "Lyonoyl\u00a0Mail never runs code that comes in a message",
      "Pictures from the web, which can track you, load only when you ask",
      "Before a link opens, you see the real address behind it",
      "Your passwords are kept locked away on your computer",
      "Your mail is stored encrypted, so a stolen disk cannot reveal it",
      "Nothing about you is sent anywhere without your permission",
      "We watch for security flaws and fix them as soon as we can",
    ]);
  });

  it("stands the promises in the middle as one block, each line from the same left edge", () => {
    // A list as wide as the card leaves its lines against the card's left side, with the room on
    // the right empty.
    renderOnboarding();
    press("Next");

    const promises = within(
      screen.getByRole("region", { name: "How Lyonoyl\u00a0Mail protects you" }),
    ).getByRole("list");
    const classes = promises.className.split(/\s+/);
    expect(classes).toEqual(expect.arrayContaining(["w-fit", "max-w-full", "text-start"]));
    expect(classes).not.toContain("w-full");
    expect(promises.parentElement?.className.split(/\s+/)).toContain("items-center");
    expect(within(promises).getAllByRole("listitem")).toHaveLength(8);
  });

  it("thanks a person for their trust on the welcome", () => {
    renderOnboarding();

    expect(
      within(
        screen.getByRole("region", { name: "Welcome to Lyonoyl\u00a0Mail" }),
      ).getByText(
        "Thank you for choosing Lyonoyl Mail. We are glad to have you with us",
      ),
    ).toBeTruthy();
  });

  it("says what looking for updates means without naming where it looks", () => {
    renderOnboarding();
    toTheLastStep();

    const step = screen.getByRole("region", { name: "Stay up to date" });
    expect(
      within(step).getByText(
        "Lyonoyl Mail checks for new versions on its own and installs one only when you say so",
      ),
    ).toBeTruthy();
    expect(step.textContent).not.toMatch(/lyonoyl\.com|GitHub/);
  });

  it("starts a fresh profile sharing nothing and not looking for updates", () => {
    renderOnboarding();

    press("Next");
    press("Next");
    expect(box(KEEP_TO_MYSELF).checked).toBe(true);

    press("Next");
    expect(box(LOOK_FOR_UPDATES).checked).toBe(false);
  });

  it("finishing as it stands looks for nothing and shares nothing", async () => {
    const { changed, done } = renderOnboarding();

    toTheLastStep();
    press("Finish");

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(sharing).not.toHaveBeenCalled();
    expect(looking).toHaveBeenCalledWith("never");
    // Finished, so it is not shown again: written last, after the answers.
    expect(welcoming).toHaveBeenCalledWith(true);
    expect(changed).toHaveBeenCalledWith({
      ...FRESH,
      lookForUpdates: "never",
      welcomed: true,
    });
  });

  it("hands the settings to the window only once it has gone", async () => {
    // Handed over at the press, the settings would say it was finished while it was still fading,
    // and the window would take it away in the middle of going.
    const { changed, done } = renderOnboarding();

    toTheLastStep();
    press("Finish");
    await vi.waitFor(() =>
      expect(cover()?.getAttribute("data-onboarding")).toBe("leaving"),
    );
    expect(changed).not.toHaveBeenCalled();

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(changed).toHaveBeenCalledTimes(1);
  });

  it("writes both answers when both boxes were changed", async () => {
    const { changed, done } = renderOnboarding();

    press("Next");
    press("Next");
    fireEvent.click(box(KEEP_TO_MYSELF));
    press("Next");
    fireEvent.click(box(LOOK_FOR_UPDATES));
    press("Finish");

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(sharing).toHaveBeenCalledWith(true);
    expect(looking).toHaveBeenCalledWith("onceADay");
    expect(changed).toHaveBeenLastCalledWith({
      ...FRESH,
      shareAnonymousData: true,
      lookForUpdates: "onceADay",
      welcomed: true,
    });
  });

  it("starts each box as the setting stands, so finishing writes no answer nobody changed", async () => {
    // Shown at every start: an answer given in the settings must survive going through it again.
    const stored: PreferencesDto = {
      ...FRESH,
      shareAnonymousData: true,
      lookForUpdates: "never",
    };
    const { changed, done } = renderOnboarding(stored);

    press("Next");
    press("Next");
    expect(box(KEEP_TO_MYSELF).checked).toBe(false);
    press("Next");
    expect(box(LOOK_FOR_UPDATES).checked).toBe(false);
    press("Finish");

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(sharing).not.toHaveBeenCalled();
    expect(looking).not.toHaveBeenCalled();
    expect(changed).toHaveBeenCalledWith({ ...stored, welcomed: true });
  });

  it("cannot be left before it is finished, by a cross or by Escape", () => {
    const { changed, done } = renderOnboarding();
    press("Next");

    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    expect(done).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
    expect(
      screen.getByRole("dialog", { name: "How Lyonoyl\u00a0Mail protects you" }),
    ).toBeTruthy();
  });

  it("covers the window alone until the settings arrive, then greets", () => {
    const { arrive } = renderOnboarding(null);

    expect(cover()).not.toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();

    arrive({ ...FRESH, shareAnonymousData: true });

    expect(
      screen.getByRole("dialog", { name: "Welcome to Lyonoyl\u00a0Mail" }),
    ).toBeTruthy();
    // Each box starts from the settings that arrived, not from the empty start.
    press("Next");
    press("Next");
    expect(box(KEEP_TO_MYSELF).checked).toBe(false);
  });

  it("hides the window under it completely", () => {
    // Drawn over the window rather than beside it, the cover takes the window's own background,
    // whole: a see-through one shows the mail before the person has been through this.
    renderOnboarding();

    const classes = cover()?.className.split(/\s+/) ?? [];
    expect(classes).toContain("bg-background");
    expect(classes.some((name) => name.startsWith("bg-background/"))).toBe(
      false,
    );
  });

  it("fades away once finished, and hands the window over when it has gone", async () => {
    const { done } = renderOnboarding();

    toTheLastStep();
    press("Finish");

    await vi.waitFor(() =>
      expect(cover()?.getAttribute("data-onboarding")).toBe("leaving"),
    );
    expect(cover()?.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["animate-out", "fade-out-0", "fill-mode-forwards"]),
    );
    expect(done).not.toHaveBeenCalled();

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
  });

  it("goes at once for somebody who asked for less motion", async () => {
    const asked = window.matchMedia.bind(window);
    vi.spyOn(window, "matchMedia").mockImplementation((query: string) =>
      query === "(prefers-reduced-motion: reduce)"
        ? ({ matches: true } as MediaQueryList)
        : asked(query),
    );
    const { done } = renderOnboarding();

    toTheLastStep();
    press("Finish");

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(cover()?.getAttribute("data-onboarding")).toBe("shown");
  });

  it("stays open and says so when the answers could not be written", async () => {
    looking.mockRejectedValue(new Error("refused"));
    const { done } = renderOnboarding();

    toTheLastStep();
    press("Finish");

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Your choices could not be saved. Try again, or continue and choose later in the settings",
    );
    expect(done).not.toHaveBeenCalled();
    // And it can be tried again.
    expect(
      (screen.getByRole("button", { name: "Finish" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("lets a person go on without the answers when they could not be written", async () => {
    // Settings that will not be written down are no reason to keep somebody out of their mail.
    looking.mockRejectedValue(new Error("refused"));
    const { done } = renderOnboarding();

    toTheLastStep();
    press("Finish");
    await screen.findByRole("alert");
    press("Continue without saving");

    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(looking).toHaveBeenCalledTimes(1);
    // Gone on from without the answers, so the next start greets again.
    expect(welcoming).not.toHaveBeenCalled();
  });

  it("offers going on without saving only once saving has failed", () => {
    renderOnboarding();
    toTheLastStep();

    expect(
      screen.queryByRole("button", { name: "Continue without saving" }),
    ).toBeNull();
  });

  it("wears the answer that was written when the other one was not", async () => {
    looking.mockRejectedValue(new Error("refused"));
    const { changed, done } = renderOnboarding();

    press("Next");
    press("Next");
    fireEvent.click(box(KEEP_TO_MYSELF));
    press("Next");
    press("Finish");

    await screen.findByRole("alert");
    expect(changed).toHaveBeenCalledWith({ ...FRESH, shareAnonymousData: true });
    expect(done).not.toHaveBeenCalled();
    // Not finished, so a later start asks again.
    expect(welcoming).not.toHaveBeenCalled();
  });

  it("is not finished twice while the answers are on their way", async () => {
    let answer: (stored: PreferencesDto) => void = () => {};
    looking.mockImplementation(
      () => new Promise<PreferencesDto>((resolve) => (answer = resolve)),
    );
    const { done } = renderOnboarding();

    toTheLastStep();
    press("Finish");
    await vi.waitFor(() => expect(looking).toHaveBeenCalled());

    const finishing = screen.getByRole("button", {
      name: "Finish",
    }) as HTMLButtonElement;
    expect(finishing.disabled).toBe(true);
    fireEvent.click(finishing);
    expect(looking).toHaveBeenCalledTimes(1);
    expect(done).not.toHaveBeenCalled();

    answer({ ...FRESH, lookForUpdates: "never" });
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
  });

  it("keeps every step in one place, and only the one shown within reach", () => {
    // A step per place would let the card grow and shrink as it is gone through, and move the
    // buttons under the pointer that is pressing them.
    renderOnboarding();

    const steps = [...document.querySelectorAll("section[data-step]")];
    expect(steps).toHaveLength(4);
    expect(new Set(steps.map((step) => step.parentElement)).size).toBe(1);

    const waiting = steps.filter(
      (step) => step.getAttribute("data-step") === "waiting",
    );
    expect(waiting).toHaveLength(3);
    for (const step of waiting) {
      expect(step.hasAttribute("inert")).toBe(true);
      expect(step.getAttribute("aria-hidden")).toBe("true");
    }

    // The picture and the heading first, at the same place on every step, and what is under
    // them in the room that is left, in its middle.
    for (const step of steps) {
      const [picture, heading, rest] = [...step.children];
      expect(picture.tagName.toLowerCase()).toBe("svg");
      expect(heading.tagName).toBe("H2");
      expect(rest.hasAttribute("data-under-the-heading")).toBe(true);
      expect(rest.className.split(/\s+/)).toEqual(
        expect.arrayContaining(["flex-1", "justify-center"]),
      );
    }

    // The step shown comes in softly, and without a movement for somebody who asked for less.
    const shown = steps.find(
      (step) => step.getAttribute("data-step") === "shown",
    );
    expect(shown?.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["animate-in", "fade-in-0", "motion-reduce:animate-none"]),
    );
  });

  it("offers the picture last where the system can change it, with the stored one chosen", async () => {
    await renderWearable();

    toThePicture();

    expect(screen.getByRole("dialog", { name: "Pick an icon" })).toBeTruthy();
    expect(screen.getByText("Step 5 of 5")).toBeTruthy();
    expect(tiles().map((one) => one.getAttribute("data-app-icon"))).toEqual([
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
    expect(pressed()).toEqual(["ember"]);
    // The pictures and nothing else: no word on any of them.
    for (const one of tiles()) expect(one.textContent).toBe("");
    // The last step finishes rather than going on.
    expect(screen.queryByRole("button", { name: "Next" })).toBeNull();
    press("Finish");
    await vi.waitFor(() => expect(welcoming).toHaveBeenCalledWith(true));
  });

  it("puts a picture on the moment it is chosen, without finishing anything", async () => {
    const { changed, arrive } = await renderWearable();
    toThePicture();

    fireEvent.click(tile("rust"));

    // Shown at once, over the heading as well as among the pictures, while the core is still
    // writing it down.
    expect(pressed()).toEqual(["rust"]);
    expect(overTheHeading().getAttribute("src")).toMatch(/\/rust\.png$/);
    expect(wearing).toHaveBeenCalledTimes(1);
    expect(wearing).toHaveBeenCalledWith("rust");
    await vi.waitFor(() =>
      expect(changed).toHaveBeenCalledWith({
        ...FRESH,
        theme: "light",
        appIcon: "rust",
      }),
    );
    // The window hands back what the core stored, as it does.
    arrive(held);
    expect(pressed()).toEqual(["rust"]);
    expect(overTheHeading().getAttribute("src")).toMatch(/\/rust\.png$/);
    // Choosing a picture is not finishing: nothing else was written, and the card stays.
    expect(sharing).not.toHaveBeenCalled();
    expect(looking).not.toHaveBeenCalled();
    expect(welcoming).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "Pick an icon" })).toBeTruthy();
  });

  it("goes back to the picture already worn when the core cannot keep the one chosen", async () => {
    wearing.mockRejectedValue(new Error("refused"));
    const { changed } = await renderWearable();
    toThePicture();

    fireEvent.click(tile("rust"));
    expect(pressed()).toEqual(["rust"]);
    await settle();

    expect(pressed()).toEqual(["ember"]);
    expect(overTheHeading().getAttribute("src")).toMatch(/\/ember\.png$/);
    expect(changed).not.toHaveBeenCalled();
  });

  it("keeps the latest choice when an earlier one is refused after it was made", async () => {
    // Two pictures chosen quickly, and the first refused once the second is on its way: going
    // back to what was shown before the first would take the second off the screen while the
    // core goes on to put it in the Dock.
    const answers: Array<{
      keep: (stored: PreferencesDto) => void;
      refuse: (reason: unknown) => void;
    }> = [];
    wearing.mockImplementation(
      () =>
        new Promise<PreferencesDto>((keep, refuse) =>
          answers.push({ keep, refuse }),
        ),
    );
    const { changed, arrive } = await renderWearable();
    toThePicture();

    fireEvent.click(tile("rust"));
    fireEvent.click(tile("ink"));
    expect(pressed()).toEqual(["ink"]);

    answers[0].refuse(new Error("refused"));
    await settle();
    expect(pressed()).toEqual(["ink"]);

    const stored: PreferencesDto = { ...FRESH, theme: "light", appIcon: "ink" };
    answers[1].keep(stored);
    await settle();
    expect(changed).toHaveBeenCalledWith(stored);
    arrive(stored);
    expect(pressed()).toEqual(["ink"]);
  });

  it("draws the same pictures whether the window is dark or light", async () => {
    const drawnIn = async (theme: "dark" | "light") => {
      await renderWearable({ ...FRESH, theme });
      toThePicture();
      const drawn = [overTheHeading(), ...tiles().map((one) => one.querySelector("img")!)]
        .map((picture) => picture.getAttribute("src"));
      cleanup();
      return drawn;
    };

    const dark = await drawnIn("dark");
    const light = await drawnIn("light");
    expect(dark).toHaveLength(10);
    expect(dark).toEqual(light);
    expect(dark[0]).toMatch(/\/ember\.png$/);
  });

  it.each([
    ["cannot change the picture", () => wearable.mockResolvedValue(false)],
    ["cannot say whether it can", () => wearable.mockRejectedValue(new Error("no answer"))],
  ])("keeps to four steps on a system that %s", async (_, answer) => {
    answer();
    renderOnboarding();
    await vi.waitFor(() => expect(wearable).toHaveBeenCalled());
    await settle();

    expect(screen.getByText("Step 1 of 4")).toBeTruthy();
    toTheLastStep();
    expect(screen.getByRole("dialog", { name: "Stay up to date" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Finish" })).toBeTruthy();
    expect(document.querySelectorAll("section[data-step]")).toHaveLength(4);
    expect(document.querySelector("[data-app-icon]")).toBeNull();
  });

  it("says on the picture's step, and only there, that the answers could not be written", async () => {
    looking.mockRejectedValue(new Error("refused"));
    const { done } = await renderWearable();
    toThePicture();

    press("Finish");

    const said = await screen.findByRole("alert");
    expect(said.textContent).toBe(
      "Your choices could not be saved. Try again, or continue and choose later in the settings",
    );
    expect(said.closest("section")?.getAttribute("data-step")).toBe("shown");
    expect(document.querySelectorAll('[role="alert"]')).toHaveLength(1);
    press("Continue without saving");
    await vi.waitFor(() => expect(done).toHaveBeenCalledTimes(1));
  });

  it("keeps the picture's step in the place every step stands, with the picture where theirs is", async () => {
    await renderWearable();

    const steps = [...document.querySelectorAll("section[data-step]")];
    expect(steps).toHaveLength(5);
    expect(new Set(steps.map((step) => step.parentElement)).size).toBe(1);
    const [picture, heading, rest] = [...steps[4].children];
    expect(picture.tagName).toBe("IMG");
    expect(picture.getAttribute("src")).toMatch(/\/ember\.png$/);
    expect(heading.tagName).toBe("H2");
    expect(heading.textContent).toBe("Pick an icon");
    expect(rest.hasAttribute("data-under-the-heading")).toBe(true);
  });

  it("measures how far a list's lines reach from its start edge, wrapped as they stand", () => {
    // From the lines of text and not from the items: an item is as wide as the room it was given,
    // whether a line fills it or not.
    const list = document.createElement("ul");
    list.innerHTML = "<li><span>first</span></li><li><span>second</span></li>";
    document.body.append(list);
    vi.spyOn(list, "getBoundingClientRect").mockReturnValue(box_(100, 700));
    linesOf((text) =>
      text === "first"
        ? [box_(126, 400)]
        : text === "second"
          ? [box_(126, 520), box_(126, 410)]
          : [],
    );

    expect(widestLine(list)).toBe(420);
    list.remove();
  });

  it("measures from the right edge in a language written from the right", () => {
    const list = document.createElement("ul");
    list.style.direction = "rtl";
    list.innerHTML = "<li><span>first</span></li><li><span>second</span></li>";
    document.body.append(list);
    vi.spyOn(list, "getBoundingClientRect").mockReturnValue(box_(100, 700));
    linesOf((text) =>
      text === "first" ? [box_(400, 674)] : text === "second" ? [box_(280, 674)] : [],
    );

    expect(widestLine(list)).toBe(420);
    list.remove();
  });

  it("measures nothing where nothing was laid out", () => {
    const list = document.createElement("ul");
    list.innerHTML = "<li><span>first</span></li>";
    document.body.append(list);

    expect(widestLine(list)).toBe(0);
    list.remove();
  });

  it("narrows the promises to their widest line once laid out, so the list stands in the middle", () => {
    // One promise too long for one line made the list as wide as the card, and every line stood
    // against its start, with the room on the other side empty.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(box_(0, 606));
    linesOf((text) =>
      text.startsWith("Pictures from the web") ? [box_(26, 496.2)] : [box_(26, 426)],
    );
    renderOnboarding();
    press("Next");

    const promises = within(
      screen.getByRole("region", { name: "How Lyonoyl\u00a0Mail protects you" }),
    ).getByRole("list");
    expect(promises.style.width).toBe("497px");
  });

  it("leaves the promises as the stylesheet makes them where nothing was laid out", () => {
    renderOnboarding();
    press("Next");

    const promises = within(
      screen.getByRole("region", { name: "How Lyonoyl\u00a0Mail protects you" }),
    ).getByRole("list");
    expect(promises.style.width).toBe("");
  });
});
