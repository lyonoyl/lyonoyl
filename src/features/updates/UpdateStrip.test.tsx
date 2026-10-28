import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { OfferedDto } from "@/shared/ipc/generated/OfferedDto";
import { UpdateStrip } from "./UpdateStrip";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    lookForAnUpdate: vi.fn(),
    installTheUpdate: vi.fn(),
    onUpdateComing: vi.fn(),
    onUpdatePutting: vi.fn(),
    restartIntoTheNewVersion: vi.fn(),
  };
});

const {
  lookForAnUpdate,
  installTheUpdate,
  onUpdateComing,
  onUpdatePutting,
  restartIntoTheNewVersion,
} = await import("@/shared/ipc/client");
const looking = vi.mocked(lookForAnUpdate);
const installing = vi.mocked(installTheUpdate);
const coming = vi.mocked(onUpdateComing);
const putting = vi.mocked(onUpdatePutting);
const restarting = vi.mocked(restartIntoTheNewVersion);

const OFFERED: OfferedDto = {
  version: "1.1.0",
  bytes: 42_000_000,
  notes: { en: "what changed" },
};

/** The handler the component gave the download event, so a test can feed it. */
let tellItHowFar:
  ((how: { soFar: number; whole: number | null }) => void) | null;
let tellItPutting: (() => void) | null;

function show(className?: string) {
  render(
    <I18nProvider initial="en">
      <UpdateStrip className={className} />
    </I18nProvider>,
  );
}

describe("the update strip", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tellItHowFar = null;
    tellItPutting = null;

    looking.mockResolvedValue(null);
    installing.mockResolvedValue("swapped");
    coming.mockImplementation((handle) => {
      tellItHowFar = handle;
      return Promise.resolve(() => {});
    });
    restarting.mockResolvedValue(undefined);
    putting.mockImplementation((handle) => {
      tellItPutting = handle;
      return Promise.resolve(() => {});
    });
  });

  // Put back whatever the test took, and after a failure too: a test that dies holding the clock
  // leaves every test after it waiting on one that does not move.
  afterEach(() => {
    vi.useRealTimers();
  });

  it("asks nothing about looking for updates, which the onboarding and the settings ask", async () => {
    // A third place asking the same question would ask it of somebody who already answered it, or
    // who will be asked again by the onboarding at the next start.
    looking.mockResolvedValue(OFFERED);
    show();

    const said = await screen.findByRole("status");
    expect(screen.queryByRole("group")).toBeNull();
    expect(within(said).getAllByRole("button").map((one) => one.textContent)).toEqual([
      "Install",
      "Hide this",
    ]);
  });

  /// A corner that says "you are up to date" all day is a corner people stop reading.
  it("draws nothing at all when there is nothing to say", async () => {
    show();

    await waitFor(() => expect(looking).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
  });

  /// And nothing when the check itself failed, because nobody asked for it.
  it("says nothing when the check could not be made", async () => {
    looking.mockRejectedValue({ code: "offline", message: "", path: null });
    show();

    await waitFor(() => expect(looking).toHaveBeenCalled());
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("stands where the foot puts it", async () => {
    // The foot gives it a line of its own above the activity line and the gear. Dropped on the
    // way, the strip would squeeze in beside them.
    looking.mockResolvedValue(OFFERED);
    show("basis-full");

    const said = await screen.findByRole("status");
    expect(said.className.split(/\s+/)).toContain("basis-full");
  });

  it("stands for an offer in the folded sidebar with a mark whose press opens it", async () => {
    // Hidden there instead, an offer would wait unseen for as long as somebody keeps it folded.
    looking.mockResolvedValue(OFFERED);
    const onUnfold = vi.fn();
    render(
      <I18nProvider initial="en">
        <UpdateStrip folded onUnfold={onUnfold} />
      </I18nProvider>,
    );

    const said = await screen.findByRole("status");
    const mark = within(said).getByRole("button");
    expect(mark.textContent).toContain("1.1.0");
    expect(mark.querySelector("svg.lucide-circle-arrow-up")).toBeTruthy();
    // Its buttons are the open sidebar's.
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();

    fireEvent.click(mark);
    expect(onUnfold).toHaveBeenCalledTimes(1);
  });

  it("says what is out, how large it is, and offers to install it", async () => {
    looking.mockResolvedValue(OFFERED);
    show();

    const said = await screen.findByRole("status");
    expect(said.textContent).toContain("1.1.0");
    // The size goes through the formatter every other screen uses rather than being spelled here.
    expect(said.textContent).toContain("42");
    expect(screen.getByRole("button", { name: "Install" })).toBeTruthy();
  });

  /// The whole reason the progress exists: a long wait with nothing moving reads as a dead control.
  it("shows how much has arrived while it comes down", async () => {
    looking.mockResolvedValue(OFFERED);
    // Never settles, so the strip stays in the state a person would be watching.
    installing.mockImplementation(() => new Promise(() => {}));
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() => expect(tellItHowFar).not.toBeNull());
    act(() => tellItHowFar?.({ soFar: 21_000_000, whole: 42_000_000 }));

    const bar = await screen.findByRole("progressbar");
    expect(bar.getAttribute("value")).toBe("21000000");
    expect(bar.getAttribute("max")).toBe("42000000");
  });

  /// A host that never said how large it is cannot have a bar drawn against nothing.
  it("counts what has arrived when nobody said how much is coming", async () => {
    looking.mockResolvedValue(OFFERED);
    installing.mockImplementation(() => new Promise(() => {}));
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() => expect(tellItHowFar).not.toBeNull());
    act(() => tellItHowFar?.({ soFar: 7_000_000, whole: null }));

    await waitFor(() => expect(screen.queryByRole("progressbar")).toBeNull());
    expect((await screen.findByRole("status")).textContent).toContain("7");
  });

  /// The gap between the last byte and the restart is its own wait and gets its own word.
  it("says it is putting it in place once the download is over", async () => {
    looking.mockResolvedValue(OFFERED);
    installing.mockImplementation(() => new Promise(() => {}));
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));
    await waitFor(() => expect(tellItPutting).not.toBeNull());
    act(() => tellItHowFar?.({ soFar: 42_000_000, whole: 42_000_000 }));
    act(() => tellItPutting?.());

    const said = await screen.findByRole("status");
    expect(said.textContent).toContain("Putting it in place");
  });

  it("asks for a restart once it is installed", async () => {
    looking.mockResolvedValue(OFFERED);
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Restart now" })).toBeTruthy(),
    );
  });

  /// The button starts the application again rather than reloading what it draws.
  ///
  /// The swap replaced the bundle on disk and threw the old one away; the process asking is still
  /// the old binary, so a reload would leave somebody running the version they were just told they
  /// had left — and the screen saying what changed, which compares the note against the version
  /// running, could never appear.
  it("restarts the application rather than the window", async () => {
    looking.mockResolvedValue(OFFERED);
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));
    fireEvent.click(await screen.findByRole("button", { name: "Restart now" }));

    await waitFor(() => expect(restarting).toHaveBeenCalled());
  });

  /// Pressing Install when the project no longer offers anything is not an install.
  ///
  /// The command asks again rather than trusting what this window was shown minutes ago, so this is
  /// an ordinary answer. Saying "installed" over it told somebody their application had been
  /// replaced when nothing had been fetched at all.
  it("does not claim an install when there was nothing newer after all", async () => {
    looking.mockResolvedValue(OFFERED);
    installing.mockResolvedValue("nothingNewer");
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
    expect(screen.queryByRole("button", { name: "Restart now" })).toBeNull();
  });

  /// A download carrying somebody else's signature is not a bad afternoon, and is not described as
  /// one.
  it("tells a wrong signature apart from a download that did not arrive", async () => {
    looking.mockResolvedValue(OFFERED);
    installing.mockRejectedValue({
      code: "providerProtocol",
      message: "",
      path: null,
    });
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() =>
      expect((screen.getByRole("status") as HTMLElement).textContent).toContain(
        "different signature",
      ),
    );
  });

  it("says plainly when the download did not arrive, and offers to try again", async () => {
    looking.mockResolvedValue(OFFERED);
    installing.mockRejectedValue({ code: "offline", message: "", path: null });
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy(),
    );
    expect(screen.getByRole("status").textContent).toContain(
      "could not be downloaded",
    );
  });

  /// An offer arriving mid-download does not replace what somebody is watching.
  ///
  /// The strip asks again while the window stays open, and a window left up for a fortnight is
  /// exactly where a second answer lands on top of a first — with a person watching a bar that
  /// turns back into the button they already pressed.
  it("does not put an offer over a download in progress", async () => {
    // `shouldAdvanceTime` because the waiting below is real: without it every `waitFor` in this
    // test sits on a clock that never moves and the test dies of its own timeout.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    looking.mockResolvedValue(OFFERED);
    installing.mockImplementation(() => new Promise(() => {}));
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Install" }));
    await waitFor(() => expect(tellItHowFar).not.toBeNull());
    act(() => tellItHowFar?.({ soFar: 21_000_000, whole: 42_000_000 }));

    // The hour really passes, so the interval really fires and really asks again.
    const askedBefore = looking.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60 * 60 * 1000);
    });

    expect(looking.mock.calls.length).toBeGreaterThan(askedBefore);
    expect(screen.queryByRole("progressbar")).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Install" })).toBeNull();
  });

  /// Somebody who is not going to update now can put it away, and it does not come back at them.
  it("can be put away", async () => {
    looking.mockResolvedValue(OFFERED);
    show();

    fireEvent.click(await screen.findByRole("button", { name: "Hide this" }));

    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  });

  it("names the reason an install did not happen, and names each one only once", async () => {
    // These were crossed: the only failure a copy on Windows or Linux can reach — the system does
    // not replace an application from inside itself — arrived as "that download carries a different
    // signature", which accuses whoever served the file. The sentence written for it was wired to a
    // disk error instead.
    const said = async (code: string) => {
      looking.mockResolvedValue(OFFERED);
      installing.mockRejectedValue({ code, message: "", path: null });
      show();
      fireEvent.click(await screen.findByRole("button", { name: "Install" }));
      // Waited until the strip has stopped showing the download rather than until it says the
      // expected words: waiting for the answer would turn a wrong sentence into a timeout, and a
      // timeout does not say what was shown instead.
      const line = await waitFor(() => {
        const found = screen.getByRole("status").textContent ?? "";
        expect(found).not.toMatch(/of \d+ MB/);
        expect(found).not.toContain("Install");
        return found;
      });
      // Each reason gets its own strip: `show` renders and this file's `afterEach` clears up, so
      // three renders inside one test would leave three strips for the query to choose from.
      cleanup();
      return line;
    };

    expect(await said("notOursToReplace")).toContain(
      "cannot replace this copy",
    );
    expect(await said("providerProtocol")).toContain("different signature");
    expect(await said("storageUnavailable")).toContain("nowhere to put");
  });
});
