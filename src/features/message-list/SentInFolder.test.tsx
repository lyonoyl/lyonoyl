import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { SentHereDto } from "@/shared/ipc/generated/SentHereDto";
import type { ViewWindowDto } from "@/shared/ipc/generated/ViewWindowDto";

import { ROW_HEIGHT } from "./rows";
import {
  READ_BACK_MS,
  SentInFolder,
  WHILE_ON_ITS_WAY_MS,
} from "./SentInFolder";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    sentHere: vi.fn(),
    getViewWindow: vi.fn(),
    markMessageRead: vi.fn(),
  };
});

const { sentHere, getViewWindow, markMessageRead } = await import(
  "@/shared/ipc/client"
);
const sent = vi.mocked(sentHere);
const windowFor = vi.mocked(getViewWindow);
const marking = vi.mocked(markMessageRead);

/** Half past ten in the morning, on a day every format writes differently from its neighbours. */
const WRITTEN_AT = 1_700_044_200;

/** A message sent from here under its own number and subject, still on its way or taken. */
function sentFromHere(
  id: number,
  subject: string,
  onItsWay: boolean,
): SentHereDto {
  return {
    id,
    sender: "zeta@example.com",
    to: ["kappa@example.net"],
    cc: [],
    bcc: [],
    subject,
    body: "Shall we meet at noon?",
    preview: "Shall we meet at noon?",
    files: [],
    writtenAt: WRITTEN_AT,
    onItsWay,
  };
}

/** What the server's folder for sent mail holds, as the list is handed it. */
function onTheServer(
  rows: { subject: string; receivedAt?: number }[],
): ViewWindowDto {
  return {
    rows: rows.map(({ subject, receivedAt }, index) => ({
      inConversation: 1,
      conversationId: null,
      messageId: `${index + 1}`,
      folderId: "9",
      stopped: null,
      subject,
      sender: "zeta@example.com",
      preview: null,
      receivedAt: receivedAt ?? 1_700_000_000 - index * 60,
      unread: false,
      leaving: false,
      arriving: false,
    })),
    total: rows.length,
    viewEpoch: "1",
    notAnswering: [],
  };
}

/** The rows drawn from here, as the subjects of the messages they show, out of those given. */
function drawnHere(...subjects: string[]): string[] {
  const rows = [...document.querySelectorAll("[data-sent-here]")];
  return subjects.filter((subject) =>
    rows.some((row) => row.textContent?.includes(subject)),
  );
}

/** How many times the server's part draws a message. */
function onServerPart(subject: string): number {
  return screen
    .queryAllByRole("option")
    .filter((row) => row.textContent?.includes(subject)).length;
}

/** Lets every answer already given arrive, so what is asserted next is after them and not before. */
async function settleDown() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
}

/** A promise held open, and the hand that settles it. */
function held<T>() {
  let settle: (value: T) => void = () => {};
  const promise = new Promise<T>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

beforeAll(() => {
  // The server's part is the ordinary list, which draws only the rows that fit its viewport; this
  // environment measures every element as nothing, which would draw none of them.
  class MeasuringObserver {
    constructor(private readonly callback: ResizeObserverCallback) {}
    observe() {
      this.callback([], this as unknown as ResizeObserver);
    }
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver =
    MeasuringObserver as unknown as typeof ResizeObserver;
  for (const property of ["clientHeight", "offsetHeight"] as const) {
    Object.defineProperty(HTMLElement.prototype, property, {
      configurable: true,
      get: () => 600,
    });
  }
  HTMLElement.prototype.getBoundingClientRect = () =>
    ({
      width: 900,
      height: 600,
      top: 0,
      left: 0,
      right: 900,
      bottom: 600,
      x: 0,
      y: 0,
    }) as DOMRect;
  HTMLElement.prototype.scrollTo = () => {};
});

function show({
  density,
  opened = null,
}: { density?: DensityDto; opened?: number | null } = {}) {
  const onOpen = vi.fn();
  const onOpenHere = vi.fn();
  const onHolds = vi.fn();
  const folder = (generation: number, chosen: number | null) => (
    <I18nProvider initial="en">
      {/* Where the focus goes when nothing in the folder is left to take it. */}
      <input data-search aria-label="Search" />
      <SentInFolder
        accountId="acc-1"
        list={{
          scope: { kind: "folder", folderId: "9" },
          generation,
          density,
          onOpen,
          onHolds,
        }}
        opened={chosen}
        onOpen={onOpenHere}
      />
    </I18nProvider>
  );
  const { rerender } = render(folder(0, opened));
  return {
    onOpen,
    onOpenHere,
    onHolds,
    /** The window saying the mail changed, and what the reader shows from here. */
    again: (generation: number, chosen: number | null = opened) =>
      rerender(folder(generation, chosen)),
  };
}

/** Whether the window can be seen, as a browser says it. */
function seen(visibility: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => visibility,
  });
}

beforeEach(() => {
  sent.mockReset();
  sent.mockResolvedValue([sentFromHere(4, "About Tuesday", true)]);
  windowFor.mockReset();
  windowFor.mockResolvedValue(onTheServer([]));
  marking.mockReset();
  marking.mockResolvedValue(1);
  seen("visible");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("the folder for sent mail", () => {
  it("shows a message the moment send is pressed, saying it is being sent where its date goes", async () => {
    show();

    const subject = await screen.findByText("About Tuesday");
    const row = subject.closest("button")!;
    expect(within(row).getByText("zeta@example.com")).toBeTruthy();
    expect(within(row).getByText("Shall we meet at noon?")).toBeTruthy();
    expect(within(row).getByText("Sending…")).toBeTruthy();
    expect(row.querySelector("time")).toBeNull();
    // Faint, as a row on its way anywhere is in the list below.
    expect(row.className).toContain("opacity-40");
    expect(screen.queryByText("This folder is empty")).toBeNull();
  });

  it("shows a message a server took with its date, as its copy will be shown", async () => {
    sent.mockResolvedValue([sentFromHere(4, "About Tuesday", false)]);
    show();

    const row = (await screen.findByText("About Tuesday")).closest("button")!;
    expect(within(row).queryByText("Sending…")).toBeNull();
    expect(row.querySelector("time")?.getAttribute("dateTime")).toBe(
      new Date(WRITTEN_AT * 1000).toISOString(),
    );
    expect(row.className).not.toContain("opacity-40");
  });

  it("writes the date the way the list below writes it", async () => {
    // The row from here is replaced by the server's copy of it, dated alike: a date written two
    // ways is the row changing under the eye for no reason.
    sent.mockResolvedValue([sentFromHere(4, "Sent from here", false)]);
    windowFor.mockResolvedValue(
      onTheServer([{ subject: "On the server", receivedAt: WRITTEN_AT }]),
    );
    show();

    const here = (await screen.findByText("Sent from here")).closest(
      "button",
    )!;
    const there = (await screen.findByText("On the server")).closest(
      "[role='option']",
    )!;
    const written = here.querySelector("time")?.textContent;
    expect(written).toBeTruthy();
    expect(written).toBe(there.querySelector("time")?.textContent);
  });

  it("shows what was sent from here above what the server holds, under no heading", async () => {
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));
    show();

    const here = await screen.findByText("About Tuesday");
    const there = await screen.findByText("Last week");
    expect(
      here.compareDocumentPosition(there) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.queryByRole("heading")).toBeNull();
  });

  it("says it is empty once, when neither place has anything", async () => {
    sent.mockResolvedValue([]);
    show();

    expect(await screen.findByText("This folder is empty")).toBeTruthy();
    expect(screen.getAllByText("This folder is empty")).toHaveLength(1);
  });

  it("says nothing about being empty while what was sent from here has not been read yet", async () => {
    sent.mockReturnValue(new Promise(() => {}));
    const { onHolds } = show();

    await waitFor(() => expect(windowFor).toHaveBeenCalled());
    await settleDown();
    expect(screen.queryByText("This folder is empty")).toBeNull();
    expect(onHolds).not.toHaveBeenCalledWith(0);
  });

  it("says it could not read what was sent from here, rather than that there is nothing", async () => {
    sent.mockRejectedValue({ code: "internal", message: "", path: null });
    const { onHolds } = show();

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("This folder is empty")).toBeNull();
    expect(onHolds).not.toHaveBeenCalledWith(0);
  });

  it("still shows the server's part when what was sent from here could not be read", async () => {
    sent.mockRejectedValue({ code: "internal", message: "", path: null });
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));
    show();

    expect(await screen.findByText("Last week")).toBeTruthy();
    expect(screen.getByRole("alert")).toBeTruthy();
  });

  it("draws nothing from here until the server's part has answered", async () => {
    // Drawn first and alone, the rows would be pushed about a moment later, under the pointer.
    const server = held<ViewWindowDto>();
    windowFor.mockReturnValue(server.promise);
    show();

    await waitFor(() => expect(sent).toHaveBeenCalled());
    await settleDown();
    expect(screen.queryByText("About Tuesday")).toBeNull();

    await act(async () => server.settle(onTheServer([{ subject: "Last week" }])));

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
  });

  it("tells the window how many the folder shows, from both places", async () => {
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));
    const { onHolds } = show();

    await waitFor(() => expect(onHolds).toHaveBeenLastCalledWith(2));
  });

  it("reads again when the window says the mail changed", async () => {
    const { again } = show();
    await screen.findByText("About Tuesday");
    sent.mockResolvedValue([]);

    again(1);

    await waitFor(() => expect(screen.queryByText("About Tuesday")).toBeNull());
    expect(sent).toHaveBeenCalledTimes(2);
  });

  it("reads again every second while a message is on its way, and stops once none is", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    show();
    await screen.findByText("Sending…");
    expect(sent).toHaveBeenCalledTimes(1);
    sent.mockResolvedValue([sentFromHere(4, "About Tuesday", false)]);

    await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS));

    await waitFor(() => expect(screen.queryByText("Sending…")).toBeNull());
    expect(sent).toHaveBeenCalledTimes(2);

    await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS * 5));
    expect(sent).toHaveBeenCalledTimes(2);
  });

  it("asks nothing while nobody can see the window", async () => {
    // A message waiting all night for a connection, on a laptop with the lid shut.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    show();
    await screen.findByText("Sending…");
    seen("hidden");

    await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS * 5));
    expect(sent).toHaveBeenCalledTimes(1);

    seen("visible");
    await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS));
    expect(sent).toHaveBeenCalledTimes(2);
  });

  it("does not let an answer to an older question stand over a newer one", async () => {
    // Asked again while the first answer was still on its way, the first arriving last would put
    // back a message the second had already seen read back.
    const first = held<SentHereDto[]>();
    sent.mockReturnValueOnce(first.promise);
    const { again } = show();
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
    sent.mockResolvedValue([]);
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));

    again(1);
    await screen.findByText("Last week");
    await act(async () =>
      first.settle([sentFromHere(4, "About Tuesday", true)]),
    );
    await settleDown();

    expect(screen.queryByText("About Tuesday")).toBeNull();
  });

  it("opens one sent from here in the reader, and the server's list lets go of what it had chosen", async () => {
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));
    const message = sentFromHere(4, "About Tuesday", true);
    sent.mockResolvedValue([message]);
    const { onOpen, onOpenHere } = show();

    fireEvent.click(await screen.findByText("Last week"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({ messageId: "1", folderId: "9" }),
    );
    expect(
      screen.getByRole("option", { selected: true }).textContent,
    ).toContain("Last week");

    fireEvent.click(screen.getByText("About Tuesday"));

    expect(onOpenHere).toHaveBeenCalledWith(message);
    await waitFor(() => expect(onOpen).toHaveBeenLastCalledWith(null));
    expect(screen.queryByRole("option", { selected: true })).toBeNull();
  });

  it("keeps a row read back until the server's part draws its copy, and then draws it once", async () => {
    // The question about this device answers first: taken away then, every row below would jump
    // up, and back down when the copy arrived a moment later.
    sent.mockResolvedValue([sentFromHere(4, "About Tuesday", false)]);
    windowFor.mockResolvedValue(onTheServer([{ subject: "Last week" }]));
    const { again } = show();
    await screen.findByText("Last week");
    expect(drawnHere("About Tuesday")).toEqual(["About Tuesday"]);
    const server = held<ViewWindowDto>();
    sent.mockResolvedValue([]);
    windowFor.mockReturnValue(server.promise);

    again(1);
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(2));
    await settleDown();

    expect(drawnHere("About Tuesday")).toEqual(["About Tuesday"]);

    await act(async () =>
      server.settle(
        onTheServer([{ subject: "About Tuesday" }, { subject: "Last week" }]),
      ),
    );

    await waitFor(() => expect(onServerPart("About Tuesday")).toBe(1));
    expect(drawnHere("About Tuesday")).toEqual([]);
  });

  it("does not keep a row read back once the server's part has already drawn its copy", async () => {
    // The other order: the server's part answered while the question about this device was on its
    // way, so the copy is there already and the row from here is a second one.
    sent.mockResolvedValue([sentFromHere(4, "About Tuesday", false)]);
    const { again } = show();
    await screen.findByText("About Tuesday");
    const here = held<SentHereDto[]>();
    sent.mockReturnValue(here.promise);
    windowFor.mockResolvedValue(onTheServer([{ subject: "About Tuesday" }]));

    again(1);
    await waitFor(() => expect(onServerPart("About Tuesday")).toBe(1));
    await act(async () => here.settle([]));

    expect(drawnHere("About Tuesday")).toEqual([]);
  });

  it("does not keep a row read back for longer than a moment when the copy is never drawn", async () => {
    // Its copy filed somewhere this folder does not show.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    sent.mockResolvedValue([sentFromHere(4, "About Tuesday", false)]);
    const { again } = show();
    await screen.findByText("About Tuesday");
    sent.mockResolvedValue([]);

    again(1);
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(2));
    await act(() => vi.advanceTimersByTimeAsync(READ_BACK_MS - 100));
    expect(drawnHere("About Tuesday")).toEqual(["About Tuesday"]);

    await act(() => vi.advanceTimersByTimeAsync(200));
    expect(drawnHere("About Tuesday")).toEqual([]);
  });

  it("holds a row read back no longer than a moment however often the folder is read again", async () => {
    // Read every second while another message waits for a connection.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const waiting = sentFromHere(5, "Still waiting", true);
    sent.mockResolvedValue([
      waiting,
      sentFromHere(4, "About Tuesday", false),
    ]);
    const { again } = show();
    await screen.findByText("About Tuesday");
    sent.mockResolvedValue([waiting]);

    again(1);
    await waitFor(() => expect(sent).toHaveBeenCalledTimes(2));
    // A second at a time, the window drawn after each, as it is between two readings.
    for (let gone = 0; gone < READ_BACK_MS; gone += WHILE_ON_ITS_WAY_MS) {
      await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS));
    }
    await act(() => vi.advanceTimersByTimeAsync(WHILE_ON_ITS_WAY_MS / 2));

    expect(sent.mock.calls.length).toBeGreaterThan(2);
    expect(drawnHere("About Tuesday", "Still waiting")).toEqual([
      "Still waiting",
    ]);
  });

  it("takes away at once a message no server took that has gone back to the drafts", async () => {
    const { again } = show();
    await screen.findByText("Sending…");
    sent.mockResolvedValue([]);

    again(1);

    await waitFor(() => expect(drawnHere("About Tuesday")).toEqual([]));
    expect(screen.queryByText("Sending…")).toBeNull();
  });

  it("marks the one the reader shows", async () => {
    sent.mockResolvedValue([
      sentFromHere(4, "About Tuesday", true),
      sentFromHere(5, "About Wednesday", true),
    ]);
    show({ opened: 5 });

    const shown = (await screen.findByText("About Wednesday")).closest(
      "button",
    )!;
    const other = screen.getByText("About Tuesday").closest("button")!;
    expect(shown.getAttribute("aria-current")).toBe("true");
    expect(shown.className).toContain("bg-muted");
    expect(other.hasAttribute("aria-current")).toBe(false);
  });

  it("draws its rows the way the list below draws its own", async () => {
    sent.mockResolvedValue([
      sentFromHere(4, "First", false),
      sentFromHere(5, "Second", false),
    ]);
    show({ density: "compact" });

    await screen.findByText("Second");
    const list = screen.getByRole("list");
    // No bar standing in the way of the rows where a system draws one.
    expect(list.className).toContain("no-gutter");
    const rows = within(list).getAllByRole("listitem");
    // A line between two rows, and none under the last one.
    expect(rows.map((row) => row.hasAttribute("data-separated"))).toEqual([
      true,
      false,
    ]);
    expect(rows[0].style.height).toBe(`${ROW_HEIGHT.compact}px`);
    // What was written keeps its own direction, as the mail in the list below does.
    expect(screen.getByText("Second").getAttribute("dir")).toBe("auto");
    expect(within(rows[1]).getByText("zeta@example.com").getAttribute("dir")).toBe(
      "auto",
    );
  });

  it("is one stop for the keyboard however many there are, walked by the arrows", async () => {
    sent.mockResolvedValue([
      sentFromHere(1, "First", true),
      sentFromHere(2, "Second", true),
      sentFromHere(3, "Third", true),
    ]);
    show();

    await screen.findByText("Third");
    const rows = ["First", "Second", "Third"].map(
      (subject) => screen.getByText(subject).closest("button")!,
    );
    expect(rows.map((row) => row.tabIndex)).toEqual([0, -1, -1]);

    rows[0].focus();
    fireEvent.keyDown(rows[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(rows[1]);
    expect(rows.map((row) => row.tabIndex)).toEqual([-1, 0, -1]);

    fireEvent.keyDown(rows[1], { key: "End" });
    expect(document.activeElement).toBe(rows[2]);
    fireEvent.keyDown(rows[2], { key: "ArrowDown" });
    expect(document.activeElement).toBe(rows[2]);

    fireEvent.keyDown(rows[2], { key: "Home" });
    expect(document.activeElement).toBe(rows[0]);
    fireEvent.keyDown(rows[0], { key: "ArrowUp" });
    expect(document.activeElement).toBe(rows[0]);
  });

  it("puts the focus on the row in its place when the one it was on is read back", async () => {
    sent.mockResolvedValue([
      sentFromHere(1, "First", false),
      sentFromHere(2, "Second", false),
    ]);
    const { again } = show();
    (await screen.findByText("First")).closest("button")!.focus();
    sent.mockResolvedValue([sentFromHere(2, "Second", false)]);
    windowFor.mockResolvedValue(onTheServer([{ subject: "First" }]));

    again(1);

    await waitFor(() => expect(drawnHere("First", "Second")).toEqual(["Second"]));
    expect(document.activeElement).toBe(
      screen.getByText("Second").closest("button"),
    );
  });

  it("puts the focus on the server's list when the last one from here is read back", async () => {
    windowFor.mockResolvedValue(onTheServer([{ subject: "About Tuesday" }]));
    sent.mockResolvedValue([sentFromHere(4, "Sent a moment ago", false)]);
    const { again } = show();
    (await screen.findByText("Sent a moment ago")).closest("button")!.focus();
    sent.mockResolvedValue([]);
    windowFor.mockResolvedValue(
      onTheServer([{ subject: "Sent a moment ago" }, { subject: "About Tuesday" }]),
    );

    again(1);

    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("listbox")),
    );
  });

  it("puts the focus on the search when nothing is left in the folder at all", async () => {
    const { again } = show();
    (await screen.findByText("About Tuesday")).closest("button")!.focus();
    sent.mockResolvedValue([]);

    again(1);

    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByLabelText("Search")),
    );
  });

  it("leaves the focus where it is when it was somewhere else", async () => {
    sent.mockResolvedValue([
      sentFromHere(1, "First", false),
      sentFromHere(2, "Second", false),
    ]);
    const { again } = show();
    (await screen.findByText("First")).closest("button")!.focus();
    const search = screen.getByLabelText("Search");
    search.focus();
    sent.mockResolvedValue([sentFromHere(2, "Second", false)]);
    windowFor.mockResolvedValue(onTheServer([{ subject: "First" }]));

    again(1);

    await waitFor(() => expect(drawnHere("First", "Second")).toEqual(["Second"]));
    expect(document.activeElement).toBe(search);
  });

  it("leaves the focus with nothing when the person put it on nothing", async () => {
    // A click on the text of the reader takes the focus off the row and gives it to nothing: that
    // is the person going somewhere, and a row read back later must not pull them back.
    sent.mockResolvedValue([
      sentFromHere(1, "First", false),
      sentFromHere(2, "Second", false),
    ]);
    const { again } = show();
    const first = (await screen.findByText("First")).closest("button")!;
    first.focus();
    first.blur();
    await settleDown();
    sent.mockResolvedValue([sentFromHere(2, "Second", false)]);
    windowFor.mockResolvedValue(onTheServer([{ subject: "First" }]));

    again(1);

    await waitFor(() => expect(drawnHere("First", "Second")).toEqual(["Second"]));
    expect(document.activeElement).toBe(document.body);
  });
});
