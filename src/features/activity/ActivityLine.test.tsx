import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { HowThingsAreDto } from "@/shared/ipc/generated/HowThingsAreDto";
import { ActivityLine } from "./ActivityLine";
import { WHEN_IDLE } from "./steady";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    howThingsAre: vi.fn(),
    onMailChanged: vi.fn(),
    whatIsBeingRead: vi.fn(),
  };
});

const { howThingsAre, onMailChanged, whatIsBeingRead } = await import(
  "@/shared/ipc/client"
);
const asking = vi.mocked(howThingsAre);
const changed = vi.mocked(onMailChanged);
const beingRead = vi.mocked(whatIsBeingRead);

const FINE: HowThingsAreDto = {
  lastSuccessAt: Math.round(Date.now() / 1000) - 120,
  stoppedBy: null,
  accountsFailing: 0,
  waiting: 0,
  stopped: 0,
  cameBack: 0,
  sending: 0,
};

function show(how: Partial<HowThingsAreDto> = {}, className?: string) {
  asking.mockResolvedValue({ ...FINE, ...how });
  render(
    <I18nProvider initial="en">
      <ActivityLine className={className} />
    </I18nProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  changed.mockResolvedValue(() => {});
  beingRead.mockReset();
  beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });
});

describe("ActivityLine", () => {
  it("says nothing when there is nothing to say", async () => {
    // An application that announces "everything is fine" all day teaches people to stop reading
    // the one line that would tell them when it is not.
    show();

    await waitFor(() => expect(asking).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("says the whole of what it has to say beside the gear, wrapped rather than cut short", async () => {
    // The foot is narrow, and cut to one line the end of a long line is lost to everybody who
    // does not point at it -- and the end is where a message that came back to the drafts is said.
    show({ stoppedBy: "offline", waiting: 2, cameBack: 1 }, "min-w-0 flex-1");

    const line = await screen.findByRole("status");
    const said = line.querySelector("p")!;

    // Every part of it is on the screen, the last one included.
    expect(said.textContent).toContain("No connection");
    expect(said.textContent).toMatch(/drafts/i);
    // Nothing cuts it: no ellipsis, no single line, no clipping, and no hint standing in for
    // words that are not shown.
    const classes = said.className.split(/\s+/);
    for (const cut of [
      "truncate",
      "whitespace-nowrap",
      "overflow-hidden",
      "text-ellipsis",
      "line-clamp-1",
    ]) {
      expect(classes, cut).not.toContain(cut);
    }
    expect(said.hasAttribute("title")).toBe(false);
    // Where the foot put it: taking the width the gear leaves, and allowed to be narrower than
    // its longest word would make it, which is what lets it wrap inside that width.
    expect(line.className.split(/\s+/)).toEqual(
      expect.arrayContaining(["min-w-0", "flex-1"]),
    );
  });

  it("says there is no connection, and how stale that leaves things", async () => {
    // Two facts, not one: "it is not working" and "you are looking at two minutes ago" lead to
    // different decisions about whether to trust what is on screen.
    show({ stoppedBy: "offline" });

    const line = await screen.findByRole("status");
    expect(line.textContent).toContain("No connection");
    expect(line.textContent).toContain("minutes ago");
  });

  it("asks a person to sign in again when that is what stopped it", async () => {
    show({ stoppedBy: "signIn" });

    expect((await screen.findByRole("status")).textContent).toContain(
      "Sign in again",
    );
  });

  it("says when it has never managed a pass at all", async () => {
    show({ stoppedBy: "offline", lastSuccessAt: null });

    expect((await screen.findByRole("status")).textContent).toContain(
      "nothing has been checked yet",
    );
  });

  it("says a sign-in is needed without a time when nothing was ever checked", async () => {
    show({ stoppedBy: "signIn", lastSuccessAt: null });

    const line = (await screen.findByRole("status")).textContent ?? "";
    expect(line).toContain("Sign in again");
    expect(line).toContain("nothing has been checked yet");
    expect(line).not.toContain("last updated");
  });

  it("says something went wrong without a time when nothing was ever checked", async () => {
    show({ stoppedBy: "other", lastSuccessAt: null });

    const line = (await screen.findByRole("status")).textContent ?? "";
    expect(line).toContain("Something went wrong");
    expect(line).toContain("nothing has been checked yet");
  });

  it("names one account that is not updating as one", async () => {
    show({ accountsFailing: 1 });

    expect((await screen.findByRole("status")).textContent).toContain(
      "One account is not updating",
    );
  });

  it("does not say the same thing twice when everything has stopped", async () => {
    // When every account is failing, the reason is the sentence. Counting them as well is the same
    // news in two forms, and a line that repeats itself is a line people skim past.
    show({ stoppedBy: "offline", accountsFailing: 2 });

    const line = await screen.findByRole("status");
    expect(line.textContent).toContain("No connection");
    expect(line.textContent).not.toContain("not updating");
  });

  it("counts the accounts that are not updating without calling everything stopped", async () => {
    // One signed-out account among four does not make the mail on screen stale.
    show({ accountsFailing: 2 });

    const line = await screen.findByRole("status");
    expect(line.textContent).toContain("2 accounts are not updating");
    expect(line.textContent).not.toContain("No connection");
  });

  it("counts what has not reached the server apart from what needs a person", async () => {
    // One is a matter of waiting; the other is a matter of deciding.
    show({ waiting: 3, stopped: 1 });

    const line = await screen.findByRole("status");
    expect(line.textContent).toContain("3 changes are not on the server yet");
    expect(line.textContent).toContain("One change needs you");
  });

  it("says one thing in the singular", async () => {
    show({ waiting: 1 });

    expect((await screen.findByRole("status")).textContent).toContain(
      "One change is not on the server yet",
    );
  });

  it("asks again when a pass changed something", async () => {
    // Otherwise the line would say "four minutes ago" while mail arrives behind it.
    let announce: null | (() => void) = null;
    changed.mockImplementation((listener) => {
      announce = () =>
        listener({ accountId: "acc-1", arrived: 1, updated: 0, removed: 0 });
      return Promise.resolve(() => {});
    });
    show({ waiting: 1 });

    await waitFor(() => expect(asking).toHaveBeenCalledTimes(1));
    (announce as null | (() => void))?.();

    await waitFor(() => expect(asking).toHaveBeenCalledTimes(2));
  });

  it("asks again the moment a stretch of reading ends", async () => {
    // A refresh that reads an account takes it off the count of accounts not updating. Asked only
    // on the slow tick, the line went on counting it for up to half a minute after the button had
    // finished, and a person pressing it again saw nothing change.
    beingRead.mockResolvedValue({ done: 0, asked: 1, through: 0, of: 1000 });
    show({ accountsFailing: 1 });
    expect(await screen.findByText("One account is not updating")).toBeTruthy();
    await waitFor(() => expect(beingRead).toHaveBeenCalled());

    asking.mockResolvedValue(FINE);
    beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });

    await waitFor(
      () => expect(screen.queryByText("One account is not updating")).toBeNull(),
      { timeout: 3000 },
    );
  });

  it("shows how far a round has got while it is running", async () => {
    // A round takes seconds and nothing else on the screen moves during it. Without this the
    // window looks like it is doing nothing at all, which is what a person reports.
    beingRead.mockResolvedValue({ done: 7, asked: 9, through: 7000, of: 9000 });
    show();

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(said.textContent).toContain("Checking mail");
    // Said in mailboxes and worked out from how far through the burst the reading is: seven
    // mailboxes' worth of nine. Counted by mailboxes finished it would say nought, because every
    // mailbox is read at the same moment as every other and they finish together at the end.
    expect(said.textContent).toContain("7 of 9");

    const bar = screen.getByRole("progressbar", {
      name: "Mail is being checked",
    });
    expect(bar.getAttribute("aria-valuenow")).toBe("7000");
    expect(bar.getAttribute("aria-valuemax")).toBe("9000");
  });

  it("shows the round even when the database will not answer", async () => {
    // The question behind everything else on this line reads the profile, and the profile is busy
    // for exactly as long as a round lasts — every account being read is using it. Drawn behind
    // that question, the one thing that is about a round in progress disappeared for the whole of
    // every round, which is what it did.
    asking.mockRejectedValue(new Error("no connection to be had"));
    beingRead.mockResolvedValue({ done: 4, asked: 9, through: 4000, of: 9000 });

    render(
      <I18nProvider initial="en">
        <ActivityLine />
      </I18nProvider>,
    );

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(said.textContent).toContain("Checking mail");
    expect(said.textContent).toContain("4 of 9");
  });

  it("says nothing at all about work that is over in a blink", async () => {
    // A round over nine quiet mailboxes can be shorter than the eye takes to move. Announced and
    // withdrawn in that time it reads as a fault rather than as progress, and the budget in
    // docs/performance/response-budgets.md says so: nothing appears before a quarter of a second.
    //
    // Busy on the first question and over by the second, which is the shape of a round nobody
    // should be told about.
    beingRead.mockResolvedValueOnce({ done: 0, asked: 9, through: 0, of: 9000 });
    beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });
    show();

    // Watched all the way through rather than looked at once at the end: a bar that appeared and
    // went is exactly the fault, and it is invisible to a single look afterwards.
    let everShown = false;
    for (let look = 0; look < 18; look += 1) {
      await new Promise((settle) => setTimeout(settle, 50));
      if (screen.queryByRole("progressbar") !== null) everShown = true;
    }

    expect(everShown, "a bar appeared and went again").toBe(false);
  });

  it("keeps the line up for long enough to read once the work is done", async () => {
    // The other half. Taken away the instant the last mailbox finishes, the line blinks out a
    // moment after it appeared — which is the same flicker from the other end.
    beingRead.mockResolvedValue({ done: 0, asked: 9, through: 0, of: 9000 });
    show();

    const bar = await screen.findByRole("progressbar", {}, { timeout: 3000 });
    expect(bar).toBeTruthy();

    // Every mailbox finished at once, which is what a round does.
    beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });
    await new Promise((settle) => setTimeout(settle, 400));

    expect(
      screen.queryByRole("progressbar"),
      "the line went the moment the work did",
    ).not.toBeNull();
  });

  it("slides between one folder and the next rather than jumping", async () => {
    // A folder finishes every few hundred milliseconds, so without this the bar is a series of
    // jumps. Asserted as the rule rather than the motion: jsdom lays nothing out, and what can be
    // checked is that the fill is told to move and told to stop for somebody who asked it to.
    beingRead.mockResolvedValue({
      done: 0,
      asked: 9,
      through: 3000,
      of: 9000,
    });
    show();

    const bar = await screen.findByRole("progressbar", {}, { timeout: 3000 });
    const fill = bar.firstElementChild;

    expect(fill?.className).toContain("transition-[width]");
    // The length named rather than the prefix: `duration-0` contains "duration-" and is a bar that
    // jumps, which is the thing this test exists to refuse.
    expect(fill?.className).toContain("duration-500");
    expect(fill?.className).toContain("motion-reduce:transition-none");
  });

  it("moves between one mailbox finishing and the next", async () => {
    // The reason folders are counted at all: with nothing
    // finished, the number is already past nought because the work is.
    beingRead.mockResolvedValue({
      done: 0,
      asked: 9,
      through: 3000,
      of: 9000,
    });
    show();

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(said.textContent).toContain("3 of 9");
  });

  it("says every mailbox is done once every mailbox is", async () => {
    // The end of the work is the one moment worth seeing. The count of what is being read goes
    // back to nought the instant the last mailbox stops, so without this the line's last word is
    // eight of nine about work that is finished, and nine of nine is never seen.
    beingRead.mockResolvedValue({
      done: 0,
      asked: 9,
      through: 5000,
      of: 9000,
    });
    show();

    await screen.findByRole("progressbar", {}, { timeout: 3000 });

    // Every mailbox finished at once, which is what a round does.
    beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });

    await waitFor(
      () => {
        const said = screen.getByRole("status");
        expect(said.textContent).toContain("9 of 9");
      },
      { timeout: 2000 },
    );
  });

  it("does not say every mailbox is done while one is still being read", async () => {
    // Nine of nine beside a mailbox still being read is the one reading a person would act on.
    beingRead.mockResolvedValue({
      done: 8,
      asked: 9,
      through: 9000,
      of: 9000,
    });
    show();

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(said.textContent).toContain("8 of 9");
    expect(said.textContent).not.toContain("9 of 9");
  });

  it("does not count one mailbox out of one", async () => {
    // "Nought of one" about the only mailbox there is reads as something being wrong rather than
    // as something being read.
    beingRead.mockResolvedValue({ done: 0, asked: 1, through: 0, of: 1000 });
    show();

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(said.textContent).toContain("Checking your mail");
    expect(said.textContent).not.toContain("of 1");
  });

  it("says nothing about a round that is not running", async () => {
    // Most of the time. A line that announced a quiet minute is a line nobody reads by the second
    // minute.
    beingRead.mockResolvedValue({ done: 0, asked: 0, through: 0, of: 0 });
    show({ cameBack: 1 });

    const said = await screen.findByRole("status");
    expect(said.textContent).not.toContain("Checking mail");
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("says a message is on its way while it is on its way", async () => {
    // The stretch between pressing send and the copy turning up in the folder for sent mail, which
    // is looked for a minute after a server takes the message. Nothing counted the outbox until a
    // message had been given up on, so for that whole stretch the product had somebody's mail and
    // said so nowhere.
    show({ sending: 1 });

    const said = await screen.findByRole("status");
    expect(said.textContent).toContain("on its way");
  });

  it("does not call a message on its way a change to a mailbox", async () => {
    // Two counts and two sentences. "Changes are not on the server yet" is about a mailbox --
    // archiving, flagging -- and a person reading it about their own unsent mail learns nothing.
    show({ sending: 2 });

    const said = await screen.findByRole("status");
    expect(said.textContent).toContain("2 messages are on their way");
    expect(said.textContent).not.toContain("not on the server yet");
  });

  it("says when a message came back, because nothing else does", () => {
    // The composer closes saying the message was sent. A server that then refuses it leaves the
    // person with no sign at all, which is why this line exists and why it is worth a test: it is
    // the only thing in the product that mentions a message that did not go.
    show({ cameBack: 1 });

    return screen
      .findByRole("status")
      .then((said) => expect(said.textContent).toContain("came back"));
  });

  it("does not ask a window nobody can see what is being read", async () => {
    // A cheap question asked four times a second all day is a laptop kept awake for it, and the
    // answer to nineteen in twenty is that nothing is being read at all.
    show();
    await waitFor(() => expect(beingRead).toHaveBeenCalled());

    hidden("hidden");
    beingRead.mockClear();
    // Longer than the idle pace, or a window that went on asking would simply not have got round
    // to it yet and this would pass on a product that never stopped.
    await new Promise((settle) => setTimeout(settle, WHEN_IDLE + 500));

    expect(
      beingRead,
      "a window nobody can see went on asking",
    ).not.toHaveBeenCalled();

    // And starts again the moment it can be seen, rather than waiting for the next tick.
    hidden("visible");
    await waitFor(() => expect(beingRead).toHaveBeenCalled());
  });

  it("asks faster while something is being read than while nothing is", async () => {
    // The pace has to carry a bar that moves inside a round of a few seconds. Held at that pace
    // when there is nothing to draw, it is the same question asked for nothing all day.
    show();
    await waitFor(() => expect(beingRead).toHaveBeenCalled());

    beingRead.mockClear();
    await new Promise((settle) => setTimeout(settle, 1200));
    const idle = beingRead.mock.calls.length;

    beingRead.mockResolvedValue({ done: 0, asked: 9, through: 0, of: 9000 });
    await waitFor(() => expect(screen.queryByRole("progressbar")).not.toBeNull(), {
      timeout: 3000,
    });

    beingRead.mockClear();
    await new Promise((settle) => setTimeout(settle, 1200));
    const reading = beingRead.mock.calls.length;

    expect(
      reading,
      `asked ${reading} times while reading and ${idle} times while idle`,
    ).toBeGreaterThan(idle * 2);
  });
});

/** What the window sees of whether anybody is looking at it. */
function hidden(state: "hidden" | "visible") {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("ActivityLine, in the folded sidebar", () => {
  function showFolded(how: Partial<HowThingsAreDto> = {}) {
    asking.mockResolvedValue({ ...FINE, ...how });
    const onUnfold = vi.fn();
    render(
      <I18nProvider initial="en">
        <ActivityLine folded onUnfold={onUnfold} />
      </I18nProvider>,
    );
    return { onUnfold };
  }

  it("says what needs saying with a mark, each part on a line of the hint, and opens the sidebar", async () => {
    const { onUnfold } = showFolded({ stoppedBy: "offline", cameBack: 1 });

    const said = await screen.findByRole("status");
    const mark = screen.getByRole("button");
    // Every part, the last one included, and no sentence drawn where there is no room for one.
    expect(mark.textContent).toContain("No connection");
    expect(mark.textContent).toMatch(/drafts/i);
    expect(said.querySelector("p")).toBeNull();

    fireEvent.focus(mark);
    const hint = await screen.findByRole("tooltip");
    expect(hint.textContent).toContain("No connection");
    expect(hint.textContent).toMatch(/drafts/i);

    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });

  it("draws a round as its bar, and keeps the words for a screen reader", async () => {
    beingRead.mockResolvedValue({ done: 7, asked: 9, through: 7000, of: 9000 });
    showFolded();

    const said = await screen.findByRole("status", {}, { timeout: 3000 });
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
      "7000",
    );
    const words = said.querySelector(".sr-only")!;
    expect(words.textContent).toContain("Checking mail");
    expect(words.textContent).toContain("7 of 9");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("says nothing when there is nothing to say", async () => {
    showFolded();

    await waitFor(() => expect(asking).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
