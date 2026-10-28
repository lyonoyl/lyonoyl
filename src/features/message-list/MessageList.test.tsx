import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import {
  afterAll,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import { useState } from "react";

import { I18nProvider } from "@/shared/i18n";
import type { ViewWindowDto } from "@/shared/ipc/generated/ViewWindowDto";
import { MessageList } from "./MessageList";
import { LIST_GUTTER, ROW_HEIGHT } from "./rows";
import { PAGE_SIZE } from "./useMessageWindows";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    getViewWindow: vi.fn(),
    getConversation: vi.fn(),
    markMessageRead: vi.fn(),
    tryActionAgain: vi.fn(),
    deleteMessage: vi.fn(),
    archiveMessage: vi.fn(),
  };
});

const {
  getViewWindow,
  getConversation,
  markMessageRead,
  tryActionAgain,
  archiveMessage,
  deleteMessage,
} = await import("@/shared/ipc/client");
const windowFor = vi.mocked(getViewWindow);
const conversationFor = vi.mocked(getConversation);
const marking = vi.mocked(markMessageRead);
const asking = vi.mocked(tryActionAgain);
const deleting = vi.mocked(deleteMessage);
const archiving = vi.mocked(archiveMessage);

/** The virtualizer measures the viewport, and a test environment reports zero for everything. */
const VIEWPORT_HEIGHT = 600;

beforeAll(() => {
  // The virtualizer asks a ResizeObserver for the viewport size. The one this environment ships
  // reports zero, which would render an empty list; reporting nothing makes the virtualizer fall
  // back to getBoundingClientRect, stubbed below.
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

  // The virtualizer measures the viewport through offsetHeight, which this environment leaves at
  // zero for every element.
  for (const [property, value] of [
    ["clientHeight", VIEWPORT_HEIGHT],
    ["offsetHeight", VIEWPORT_HEIGHT],
    ["clientWidth", 900],
    ["offsetWidth", 900],
  ] as const) {
    Object.defineProperty(HTMLElement.prototype, property, {
      configurable: true,
      get() {
        return value;
      },
    });
  }
  HTMLElement.prototype.getBoundingClientRect =
    function getBoundingClientRect() {
      return {
        width: 900,
        height: VIEWPORT_HEIGHT,
        top: 0,
        left: 0,
        right: 900,
        bottom: VIEWPORT_HEIGHT,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      } as DOMRect;
    };
  HTMLElement.prototype.scrollTo = () => {};
});

function page(offset: number, total: number, count = PAGE_SIZE): ViewWindowDto {
  return {
    rows: Array.from(
      { length: Math.min(count, Math.max(0, total - offset)) },
      (_, position) => {
        const index = offset + position;
        return {
          inConversation: 1,
          conversationId: null,
          messageId: `${index + 1}`,
          folderId: "1",
          stopped: null,
          subject: `Subject ${index}`,
          sender: `sender${index}@example.com`,
          preview: null,
          receivedAt: 1_700_000_000 - index * 60,
          unread: index % 5 === 0,
          leaving: false,
          arriving: false,
        };
      },
    ),
    total,
    viewEpoch: "1",
    notAnswering: [],
  };
}

function renderList(emptyState?: React.ReactNode, language = "en") {
  return render(
    <I18nProvider initial={language}>
      <MessageList
        scope={{ kind: "folder", folderId: "1" }}
        emptyState={emptyState}
      />
    </I18nProvider>,
  );
}

/**
 * The list, once it has stopped asking for windows.
 *
 * `findByText("Subject 0")` proves the first window arrived, and that is not the same as the list
 * having settled: the virtualizer reports what is visible, the list asks for that range, and those
 * answers land afterwards. A test that acted on the first row and then read the rendered rows was
 * racing one of those arrivals — which re-renders, shifts what the virtualizer returns, and moves
 * the row the assertion was about. It showed as one test in twenty failing, a different one each
 * time, in whichever test the arrival happened to land.
 */
async function settled() {
  await screen.findByText("Subject 0");

  let asked = windowFor.mock.calls.length;
  await waitFor(() => {
    const now = windowFor.mock.calls.length;
    const quiet = now === asked;
    asked = now;
    expect(quiet, "the list is still asking for windows").toBe(true);
  });
}

/**
 * Chooses the first row, the way a first arrow press does in a list where nothing is chosen yet.
 *
 * Every list starts with nothing chosen, so a test about moving from a row has to choose one.
 */
async function chooseTheFirstRow() {
  const list = screen.getByRole("listbox");
  fireEvent.keyDown(list, { key: "ArrowDown" });
  await waitFor(() =>
    expect(list.getAttribute("aria-activedescendant")).toBe(
      "message-list-row-0",
    ),
  );
  return list;
}

beforeEach(() => {
  windowFor.mockReset();
  // Reset rather than cleared: a test that leaves an answer behind makes the next one pass for a
  // reason that is not its own.
  //
  // Answering again straight away, though, because a reset mock returns `undefined` and the list
  // calls `.catch` on what it gets back. Whether it makes the call at all depends on what the
  // person did in that test — opening an unread row is what marks it read — so a test that never
  // meant to exercise this would fail on a timing it did not choose, in a different test each
  // run. A default answer here is the difference between a suite that is red about a real thing
  // and one that is red about the order its own effects happened to fire in.
  marking.mockReset();
  marking.mockResolvedValue(1);
  asking.mockReset();
  asking.mockResolvedValue(undefined);
  deleting.mockReset();
  deleting.mockResolvedValue(undefined);
  // Answering with nothing rather than being cleared, for the same reason: a conversation that is
  // asked for and never answered leaves a row saying it is open with nothing under it.
  conversationFor.mockReset();
  conversationFor.mockResolvedValue([]);
});

describe("MessageList", () => {
  /** A page whose first row carries an instruction that stopped. */
  function pageWithAStop(reason: string) {
    return async (request: { offset: number }) => {
      const window = page(request.offset, 3);
      const rows = [...window.rows];
      if (rows[0]) rows[0] = { ...rows[0], stopped: reason };
      return { ...window, rows };
    };
  }

  /** A page whose rows carry the names given, in that order. */
  function pageOf(names: string[]): ViewWindowDto {
    const built = page(0, names.length);
    return {
      ...built,
      rows: built.rows.map((row, at) => ({
        ...row,
        messageId: names[at] ?? row.messageId,
        subject: `Subject ${at}`,
        unread: true,
      })),
    };
  }

  it("draws no frame round the list, however the focus arrived and wherever the keys moved", async () => {
    // The selected row says where a person is. The list takes the focus by itself once a message
    // is filed away, and a frame round the whole column then lit up at the press of a button that
    // had nothing to do with it.
    windowFor.mockImplementation((request) =>
      Promise.resolve(page(request.offset, 3)),
    );
    renderList();
    await settled();
    const list = screen.getByRole("listbox");
    const framed = () =>
      list.className
        .split(/\s+/)
        .some((name) => /(^|:)(ring|outline)(-|$)/.test(name) && name !== "outline-none");

    act(() => list.focus());
    expect(framed(), "a frame where the focus arrived").toBe(false);
    expect(list.className.split(/\s+/)).toContain("outline-none");

    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0",
      ),
    );
    expect(framed(), "a frame while the keys move").toBe(false);
  });

  it("marks nothing read when an arrow moved nothing and the rows then changed", async () => {
    // The flag saying somebody moved is armed by the keystroke and spent by the effect that
    // follows it. An arrow at the top of the list moves nothing, React bails out of a state set
    // to what it already holds, and that effect never runs — so the flag stayed armed until the
    // next thing that moved the rows, and a message nobody chose was read on the server.
    let arrived = false;
    windowFor.mockImplementation(() =>
      Promise.resolve(
        pageOf(arrived ? ["new", "a", "b", "c"] : ["a", "b", "c"]),
      ),
    );

    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={0} />
      </I18nProvider>,
    );
    await settled();

    await chooseTheFirstRow();
    fireEvent.keyDown(screen.getByRole("listbox"), { key: "ArrowUp" });
    marking.mockClear();

    arrived = true;
    rerender(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={1} />
      </I18nProvider>,
    );
    // The message that was selected is now one row down, and the selection followed it by name
    // rather than staying on the position — which is the list working as designed, and the proof
    // that nothing read this as somebody opening the newcomer.
    await screen.findByText("Subject 3");
    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-1"),
    );

    expect(marking).not.toHaveBeenCalled();
  });

  it("marks nothing read when the words being looked for change", async () => {
    // The list is deliberately not remounted while somebody types — a key that moved with the
    // words would take the scroll position and the loaded rows on every letter — so nothing about
    // it is thrown away between one question and the next. A selection is a position, and a
    // position in the answer to the old question names a different message in the new one.
    windowFor.mockImplementation((request) =>
      Promise.resolve(
        pageOf(
          request.scope.kind === "looking" && request.scope.typed === "quarter"
            ? ["a", "b", "c"]
            : ["x", "y", "z"],
        ),
      ),
    );

    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "looking", typed: "quarter" }} />
      </I18nProvider>,
    );
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-1",
      ),
    );
    marking.mockClear();

    rerender(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "looking", typed: "quarterly" }} />
      </I18nProvider>,
    );
    await screen.findByText("Subject 0");
    // The message being read is not in the new answer, so nothing in it is chosen — and nothing
    // in it was read by being landed on either.
    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBeNull(),
    );

    expect(marking).not.toHaveBeenCalled();
  });

  it("says how many messages a conversation holds, and says nothing for one", async () => {
    // A "1" beside every subject would be noise on every row and information on none.
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 2);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        inConversation: at === 0 ? 7 : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return page_;
    });
    renderList();
    await settled();
    const said = await screen.findByLabelText(
      "7 messages in this conversation",
    );
    expect(said.textContent).toBe("7");
    expect(
      screen.queryByLabelText(/1 messages in this conversation/),
    ).toBeNull();
  });

  it("opens a conversation under its row, and closes it again", async () => {
    // The number is the way in and the way out: the messages appear under the row that stands for
    // them, and everything below moves down by exactly as much as appeared.
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 3);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        inConversation: at === 0 ? 3 : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return page_;
    });
    conversationFor.mockResolvedValue([
      { ...page(0, 1).rows[0], messageId: "a", subject: "Oldest reply" },
      { ...page(0, 1).rows[0], messageId: "b", subject: "Middle reply" },
      { ...page(0, 1).rows[0], messageId: "1", subject: "Subject 0" },
    ]);

    renderList();
    await settled();

    const below = () =>
      screen.getByText("Subject 1").closest("[role='option']") as HTMLElement;
    const wasAt = below().style.transform;
    expect(wasAt).toBe(`translateY(${LIST_GUTTER + ROW_HEIGHT.comfortable}px)`);

    fireEvent.click(screen.getByLabelText("3 messages in this conversation"));

    expect(await screen.findByText("Oldest reply")).toBeTruthy();
    expect(screen.getByText("Middle reply")).toBeTruthy();
    // Three messages appeared above it, so it stands three rows lower — not two, not four.
    await waitFor(() =>
      expect(below().style.transform).toBe(
        `translateY(${LIST_GUTTER + ROW_HEIGHT.comfortable * 4}px)`,
      ),
    );

    fireEvent.click(screen.getByLabelText("3 messages in this conversation"));

    await waitFor(() => expect(screen.queryByText("Oldest reply")).toBeNull());
    expect(below().style.transform).toBe(wasAt);

    // Closing asks nobody anything, and opening again shows what was already read rather than
    // waiting for it a second time.
    expect(conversationFor).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByLabelText("3 messages in this conversation"));
    expect(screen.getByText("Oldest reply")).toBeTruthy();
  });

  /** A window whose first row stands for the conversation named "a". */
  function windowWhereTheFirstRowIsAConversation(
    of: number,
    shownBy: string,
    epoch = "1",
  ) {
    return async (request: { offset: number }) => {
      const page_ = page(request.offset, 3);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        messageId: at === 0 ? shownBy : row.messageId,
        inConversation: at === 0 ? of : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return { ...page_, viewEpoch: epoch };
    };
  }

  it("redraws an open conversation a reply lands in, instead of closing it", async () => {
    // A reply arriving moves every row and makes a different message the newest, which is the row
    // the conversation is now shown by. Held by where it sat, or by the message it showed, the
    // conversation somebody was reading closed itself exactly when something happened in it.
    windowFor.mockImplementation(windowWhereTheFirstRowIsAConversation(2, "1"));
    conversationFor.mockResolvedValue([
      { ...page(0, 1).rows[0], messageId: "a", subject: "Oldest reply" },
      { ...page(0, 1).rows[0], messageId: "1", subject: "Subject 0" },
    ]);

    const view = render(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={0} />
      </I18nProvider>,
    );
    await settled();
    fireEvent.click(screen.getByLabelText("2 messages in this conversation"));
    expect(await screen.findByText("Oldest reply")).toBeTruthy();

    // The reply lands. The conversation holds three now and is shown by the reply itself.
    windowFor.mockImplementation(
      windowWhereTheFirstRowIsAConversation(3, "the-reply", "2"),
    );
    conversationFor.mockResolvedValue([
      { ...page(0, 1).rows[0], messageId: "a", subject: "Oldest reply" },
      { ...page(0, 1).rows[0], messageId: "1", subject: "Subject 0" },
      {
        ...page(0, 1).rows[0],
        messageId: "the-reply",
        subject: "What just arrived",
      },
    ]);
    view.rerender(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={1} />
      </I18nProvider>,
    );

    expect(await screen.findByText("What just arrived")).toBeTruthy();
    expect(screen.getByText("Oldest reply")).toBeTruthy();
  });

  it("shows nothing under a row that is not the conversation that was opened", async () => {
    // The other half of holding a conversation by its name: a row standing for something else is
    // not the place to draw it, and neither is the position it used to have.
    windowFor.mockImplementation(windowWhereTheFirstRowIsAConversation(2, "1"));
    conversationFor.mockResolvedValue([
      { ...page(0, 1).rows[0], messageId: "a", subject: "Oldest reply" },
      { ...page(0, 1).rows[0], messageId: "1", subject: "Subject 0" },
    ]);

    const view = render(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={0} />
      </I18nProvider>,
    );
    await settled();
    fireEvent.click(screen.getByLabelText("2 messages in this conversation"));
    expect(await screen.findByText("Oldest reply")).toBeTruthy();

    // Every row now stands for a message and none for that conversation.
    windowFor.mockImplementation(async (request) => ({
      ...page(request.offset, 3),
      viewEpoch: "2",
    }));
    view.rerender(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: "1" }} generation={1} />
      </I18nProvider>,
    );

    await waitFor(() => expect(screen.queryByText("Oldest reply")).toBeNull());

    // And the list keeps no room for it either: a conversation drawing nothing while the places
    // under it stay counted pushes every row below it down past two gaps that hold no message.
    await waitFor(() =>
      expect(
        (
          screen
            .getByText("Subject 1")
            .closest("[role='option']") as HTMLElement
        ).style.transform,
      ).toBe(`translateY(${LIST_GUTTER + ROW_HEIGHT.comfortable}px)`),
    );
  });

  it("leaves the row as it was when the conversation cannot be read", async () => {
    // A row that says it is open with nothing under it is worse than one that never opened.
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 2);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        inConversation: at === 0 ? 4 : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return page_;
    });
    conversationFor.mockRejectedValue(new Error("no"));

    renderList();
    await settled();

    const badge = screen.getByLabelText("4 messages in this conversation");
    fireEvent.click(badge);

    await waitFor(() =>
      expect(badge.getAttribute("aria-expanded")).toBe("false"),
    );
    expect(
      screen.getByText("Subject 1").closest("[role='option']"),
    ).toBeTruthy();
  });

  /** Three rows, the first of them standing for a conversation of three messages. */
  function aListWithAConversation() {
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 3);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        inConversation: at === 0 ? 3 : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return page_;
    });
    conversationFor.mockResolvedValue([
      {
        ...page(0, 1).rows[0],
        messageId: "a",
        subject: "Oldest reply",
        unread: false,
      },
      {
        ...page(0, 1).rows[0],
        messageId: "b",
        subject: "Middle reply",
        unread: false,
      },
      {
        ...page(0, 1).rows[0],
        messageId: "1",
        subject: "Subject 0",
        unread: false,
      },
    ]);
  }

  /** Opens the conversation on the first row and waits for its messages to arrive. */
  async function openTheConversation() {
    fireEvent.click(screen.getByLabelText("3 messages in this conversation"));
    await screen.findByText("Oldest reply");
  }

  it("walks into an open conversation and out of its far end", async () => {
    // Without this the messages under a row are reachable by pointer only: the arrows step over
    // the whole conversation and land on the row below it.
    aListWithAConversation();
    renderList();
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-0",
      ),
    );
    expect(
      screen
        .getByText("Oldest reply")
        .closest("[role=option]")
        ?.getAttribute("aria-selected"),
    ).toBe("true");

    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-1",
      ),
    );

    // Three messages, then the row under the conversation — not the row three places down.
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-1",
      ),
    );
  });

  it("comes back up out of a conversation the way it went in", async () => {
    aListWithAConversation();
    renderList();
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-1",
      ),
    );

    fireEvent.keyDown(list, { key: "ArrowUp" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-0",
      ),
    );

    fireEvent.keyDown(list, { key: "ArrowUp" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0",
      ),
    );
  });

  it("lands on the last message of an open conversation when coming up onto its row", async () => {
    // Coming up onto a row whose conversation is open and stopping there would step over every
    // message in it, which is the same hole in the other direction.
    aListWithAConversation();
    renderList();
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    for (let press = 0; press < 4; press += 1)
      fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-1",
      ),
    );

    fireEvent.keyDown(list, { key: "ArrowUp" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-2",
      ),
    );
  });

  it("opens and closes a conversation without reaching for the number beside it", async () => {
    aListWithAConversation();
    renderList();
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowRight" });
    expect(await screen.findByText("Oldest reply")).toBeTruthy();

    fireEvent.keyDown(list, { key: "ArrowRight" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-0",
      ),
    );

    fireEvent.keyDown(list, { key: "ArrowLeft" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0",
      ),
    );
    // Coming out of a conversation is not closing it: its messages are still under the row, and
    // it takes the second press to fold them away.
    expect(screen.getByText("Oldest reply")).toBeTruthy();

    fireEvent.keyDown(list, { key: "ArrowLeft" });
    await waitFor(() => expect(screen.queryByText("Oldest reply")).toBeNull());
  });

  it("opens a conversation with the left arrow in a window that runs right to left", async () => {
    // Mirrored, the conversation opens towards the left, where its messages are drawn from.
    aListWithAConversation();
    renderList(undefined, "ar");
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowRight" });
    expect(screen.queryByText("Oldest reply")).toBeNull();

    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(await screen.findByText("Oldest reply")).toBeTruthy();

    fireEvent.keyDown(list, { key: "ArrowLeft" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-0",
      ),
    );

    fireEvent.keyDown(list, { key: "ArrowRight" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0",
      ),
    );
  });

  it("asks for nothing when the row it is on stands for one message", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    fireEvent.keyDown(screen.getByRole("listbox"), { key: "ArrowRight" });
    expect(conversationFor).not.toHaveBeenCalled();
  });

  it("opens each message of a conversation as the arrows reach it", async () => {
    // Walking through a conversation is reading it. A selection that moved without the reader
    // following would be the list and the reader disagreeing about what is being read.
    aListWithAConversation();
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();
    await openTheConversation();
    const list = await chooseTheFirstRow();
    onOpen.mockClear();

    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "a", folderId: "1" }),
    );

    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "b", folderId: "1" }),
    );
  });

  it("keeps the message the selection is on inside a conversation, on Enter", async () => {
    // Enter is a choice like a click, of what the selection is on: the message inside the
    // conversation, and never the one its row shows.
    aListWithAConversation();
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "a",
        folderId: "1",
      }),
    );

    fireEvent.keyDown(list, { key: "Enter" });
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(onOpen).toHaveBeenLastCalledWith({ messageId: "a", folderId: "1" });
    expect(list.getAttribute("aria-activedescendant")).toBe(
      "message-list-row-0-inside-0",
    );
  });

  it("walks out of a conversation whose newest message is also its row's, however often the window draws it", async () => {
    // The newest message of an open conversation is shown twice: inside it, and as its row. Found
    // on the row whenever anything draws the list again -- the window does, with a new question,
    // on every message opened -- the next arrow would walk back into the conversation instead of
    // out of it, for ever.
    aListWithAConversation();
    function Reading() {
      const [, setOpened] = useState<unknown>(null);
      return (
        <I18nProvider initial="en">
          <MessageList
            scope={{ kind: "folder", folderId: "1" }}
            onOpen={setOpened}
          />
        </I18nProvider>
      );
    }
    render(<Reading />);
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    for (const at of [0, 1, 2]) {
      fireEvent.keyDown(list, { key: "ArrowDown" });
      await waitFor(() =>
        expect(list.getAttribute("aria-activedescendant")).toBe(
          `message-list-row-0-inside-${at}`,
        ),
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(list.getAttribute("aria-activedescendant")).toBe(
      "message-list-row-0-inside-2",
    );

    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-1",
      ),
    );
  });

  it("does not throw a message away from inside a conversation", async () => {
    // Filing plays a row out of the list and takes it out of the count, and a message hanging
    // under a row is neither. The key doing half of that would leave a row saying three where two
    // are drawn.
    aListWithAConversation();
    renderList();
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-0",
      ),
    );

    fireEvent.keyDown(list, { key: "Delete" });
    expect(deleting).not.toHaveBeenCalled();
  });

  it("comes back out onto the row when the conversation it was inside closes", async () => {
    // A conversation closes for reasons of its own, and what was selected inside it is then a
    // message the list does not draw.
    aListWithAConversation();
    renderList();
    await settled();
    await openTheConversation();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0-inside-1",
      ),
    );

    fireEvent.click(screen.getByLabelText("3 messages in this conversation"));
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-0",
      ),
    );
  });

  it("shows what stopped an instruction, on the row it was about", async () => {
    // The application promises that what it could not carry out reaches a person rather than being
    // dropped. This is the only place a person can see that.
    windowFor.mockImplementation(pageWithAStop("messageIsNotThere"));
    renderList();

    expect(
      await screen.findByText("That message is no longer in this folder"),
    ).toBeTruthy();
  });

  it("explains a reason it has no words for rather than saying nothing", async () => {
    // Written by a newer version. Saying nothing hides an instruction a person has to decide
    // about; naming a specific wrong reason sends them somewhere pointless.
    windowFor.mockImplementation(pageWithAStop("some-newer-reason"));
    renderList();

    expect(await screen.findByText("The server would not do it")).toBeTruthy();
  });

  it("offers a way out of the explanation, not only the explanation", async () => {
    // An explanation with nothing to do about it is a dead end rather than a message.
    asking.mockResolvedValue(undefined);
    windowFor.mockImplementation(pageWithAStop("theServerRefused"));
    const opened = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={opened}
        />
      </I18nProvider>,
    );
    await screen.findByText("The server would not do it");
    const announced = opened.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    expect(asking).toHaveBeenCalledWith(expect.any(String), "1");
    expect(opened.mock.calls.length).toBe(announced);
  });

  it("keeps the dot and the retry link when a folder number is a large one", async () => {
    // A folder number carries the file the folder is kept in, so it outgrows what a 32-bit field
    // holds and what a JavaScript number holds exactly. It used to arrive as nothing at all: the
    // row then showed a dot nobody could press and no way to try a stopped action again.
    const huge = "9007199254740993";
    marking.mockResolvedValue(1);
    asking.mockResolvedValue(undefined);
    windowFor.mockImplementation(async (request) => {
      const window = page(request.offset, 3);
      window.rows = window.rows.map((row, position) => ({
        ...row,
        folderId: huge,
        stopped: position === 0 ? "theServerRefused" : null,
      }));
      return window;
    });
    render(
      <I18nProvider initial="en">
        <MessageList scope={{ kind: "folder", folderId: huge }} />
      </I18nProvider>,
    );
    await settled();

    fireEvent.click(
      screen.getAllByRole("button", { name: /Mark as/ })[0] as HTMLElement,
    );
    expect(marking).toHaveBeenCalledWith(
      expect.any(String),
      huge,
      expect.any(Boolean),
    );

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(asking).toHaveBeenCalledWith(expect.any(String), huge);
  });

  it("names an account whose mail could not be read instead of leaving it out", async () => {
    // The mail is in a file per account, so one file that will not open costs one account. A list
    // that came back short without saying so would be trusted and should not be.
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 3);
      return { ...page_, notAnswering: ["work@example.com"] };
    });
    renderList();
    await settled();

    const said = await screen.findByRole("status");
    expect(said.textContent).toContain("work@example.com");
    // And the mail that could be read is still there.
    expect(screen.getAllByRole("option").length).toBeGreaterThan(0);
  });

  it("names each account in its own direction in a window that runs right to left", async () => {
    // Joined around the addresses rather than through them: a list isolated as a whole takes its
    // direction from its first address and puts every conjunction beside the wrong one.
    windowFor.mockImplementation(async (request) => ({
      ...page(request.offset, 3),
      notAnswering: ["zeta@example.com", "kappa@example.net"],
    }));
    renderList(undefined, "ar");
    await settled();

    const said = (await screen.findByRole("status")).textContent ?? "";
    expect(said).toContain("\u2068zeta@example.com\u2069");
    expect(said).toContain("\u2068kappa@example.net\u2069");
  });

  it("says nothing about a row nothing stopped on", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("marks a message read from the dot that shows it, without opening it", async () => {
    // The thing showing the state is the thing that sets it. And the click must not reach the row:
    // opening a message is what marks it read by accident.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    const opened = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={opened}
        />
      </I18nProvider>,
    );
    await settled();

    // The list announces the row it opens on, so what matters is that the click adds nothing to
    // that — not that nothing was ever opened.
    const announced = opened.mock.calls.length;

    fireEvent.click(
      screen.getAllByRole("button", { name: "Mark as read" })[0] as HTMLElement,
    );

    expect(marking).toHaveBeenCalledWith(expect.any(String), "1", true);
    expect(opened.mock.calls.length).toBe(announced);
  });

  it("says so when opening a message is what marked it read", async () => {
    // Nobody asked for it, so nobody is expected to guess that the dot beside a read row — drawn
    // as nothing at all — is the way back.
    marking.mockResolvedValue(1);
    // Every row unread, so the one that is clicked is one that opening changes.
    windowFor.mockImplementation(async (request) => {
      const window = page(request.offset, 3);
      return {
        ...window,
        rows: window.rows.map((row) => ({ ...row, unread: true })),
      };
    });
    const filed = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={vi.fn()}
          onFiled={filed}
        />
      </I18nProvider>,
    );
    await settled();

    fireEvent.click(screen.getAllByRole("option")[1] as HTMLElement);

    await waitFor(() =>
      expect(filed).toHaveBeenCalledWith({
        messageId: expect.any(String),
        folderId: "1",
        what: "read",
      }),
    );
  });

  it("says nothing when the dot is what marked it read", async () => {
    // They pressed the thing that shows the state, and it is still under their finger.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    const filed = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={vi.fn()}
          onFiled={filed}
        />
      </I18nProvider>,
    );
    await settled();
    filed.mockClear();

    fireEvent.click(
      screen.getAllByRole("button", { name: "Mark as read" })[0] as HTMLElement,
    );

    expect(filed).not.toHaveBeenCalled();
  });

  it("shows the message read before the core has answered", async () => {
    // The core would answer the same thing; what this saves is the wait, at the one moment where
    // a delay is felt — right under the finger that just clicked.
    let answer = () => {};
    marking.mockImplementation(
      () =>
        new Promise<number>((resolve) => {
          answer = () => resolve(1);
        }),
    );
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    // Counted rather than merely present: the page holds rows that were already read, and one of
    // their dots would satisfy "is there one" without this one having changed at all.
    const unreadBefore = screen.getAllByRole("button", {
      name: "Mark as read",
    }).length;
    expect(unreadBefore).toBeGreaterThan(0);

    fireEvent.click(
      screen.getAllByRole("button", { name: "Mark as read" })[0] as HTMLElement,
    );

    // Still waiting on the core, and the dot already says read.
    expect(
      screen.queryAllByRole("button", { name: "Mark as read" }).length,
    ).toBe(unreadBefore - 1);
    answer();
  });

  it("changes the row that was clicked, and only that one", async () => {
    // Every test above clicks the first unread row, so a change that always landed on the first
    // row would look exactly the same. This one clicks another.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 30));
    renderList();
    await settled();

    const dots = screen.getAllByRole("button", { name: "Mark as read" });
    expect(dots.length).toBeGreaterThan(1);

    const first = dots[0] as HTMLElement;
    fireEvent.click(dots[1] as HTMLElement);

    expect(first.getAttribute("aria-label")).toBe("Mark as read");
    expect(screen.getAllByRole("button", { name: "Mark as read" }).length).toBe(
      dots.length - 1,
    );
  });

  it("puts the dot back when the write did not happen", async () => {
    // Nothing reached the disk, so nothing may be left looking as though it had.
    marking.mockRejectedValue(new Error("no"));
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    const unreadBefore = screen.getAllByRole("button", {
      name: "Mark as read",
    }).length;

    fireEvent.click(
      screen.getAllByRole("button", { name: "Mark as read" })[0] as HTMLElement,
    );
    expect(
      screen.queryAllByRole("button", { name: "Mark as read" }).length,
    ).toBe(unreadBefore - 1);

    await waitFor(() =>
      expect(
        screen.getAllByRole("button", { name: "Mark as read" }).length,
      ).toBe(unreadBefore),
    );
  });

  it("shows the rows the core returned", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    renderList();

    expect(await screen.findByText("Subject 0")).toBeTruthy();
    expect(screen.getByText("sender0@example.com")).toBeTruthy();
  });

  it("makes room for the whole set, not just for the window", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    renderList();

    await settled();
    const spacer = screen.getByRole("listbox").firstElementChild as HTMLElement;

    // 250 comfortable rows: the scrollbar describes the set, not the hundred loaded rows. Said as
    // the rule rather than as a number, so changing what a row looks like does not fail a test
    // about how much room the set takes.
    expect(spacer.style.height).toBe(
      `${250 * ROW_HEIGHT.comfortable + 2 * LIST_GUTTER}px`,
    );
  });

  it("shows a message as sender, subject and its opening lines", async () => {
    windowFor.mockImplementation(async () => ({
      rows: [
        {
          inConversation: 1,
          conversationId: null,
          messageId: "m-1",
          folderId: "1",
          stopped: null,
          subject: "Weekly report",
          sender: "Zeta",
          preview: "Numbers are up. Details inside.",
          receivedAt: 1_700_000_000,
          unread: true,
          leaving: false,
          arriving: false,
        },
      ],
      total: 1,
      viewEpoch: "1",
      notAnswering: [],
    }));
    renderList();

    expect(await screen.findByText("Zeta")).toBeTruthy();
    expect(screen.getByText("Weekly report")).toBeTruthy();
    expect(screen.getByText("Numbers are up. Details inside.")).toBeTruthy();
  });

  it("leaves out the opening lines of a message whose body has not been read", async () => {
    windowFor.mockImplementation(async () => ({
      rows: [
        {
          inConversation: 1,
          conversationId: null,
          messageId: "m-1",
          folderId: "1",
          stopped: null,
          subject: "Envelope only",
          sender: "Zeta",
          preview: null,
          receivedAt: 1_700_000_000,
          unread: false,
          leaving: false,
          arriving: false,
        },
      ],
      total: 1,
      viewEpoch: "1",
      notAnswering: [],
    }));
    renderList();

    expect(await screen.findByText("Envelope only")).toBeTruthy();
    // Nothing stands in for text that is not there yet.
    expect(screen.getByRole("option").textContent).toContain("Zeta");
  });

  it("packs a message into one line when the reader asked for compact", async () => {
    windowFor.mockImplementation(async () => ({
      rows: [
        {
          inConversation: 1,
          conversationId: null,
          messageId: "m-1",
          folderId: "1",
          stopped: null,
          subject: "Weekly report",
          sender: "Zeta",
          preview: "Numbers are up.",
          receivedAt: 1_700_000_000,
          unread: false,
          leaving: false,
          arriving: false,
        },
      ],
      total: 1,
      viewEpoch: "1",
      notAnswering: [],
    }));
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          density="compact"
        />
      </I18nProvider>,
    );

    expect(await screen.findByText("Weekly report")).toBeTruthy();
    expect(
      screen.queryByText("Numbers are up."),
      "a compact row is a line per message, not four",
    ).toBeNull();
  });

  it("gives a compact list more rows in the same space", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          density="compact"
        />
      </I18nProvider>,
    );

    await settled();
    const spacer = screen.getByRole("listbox").firstElementChild as HTMLElement;

    expect(spacer.style.height).toBe(
      `${250 * ROW_HEIGHT.compact + 2 * LIST_GUTTER}px`,
    );
    expect(ROW_HEIGHT.compact).toBeLessThan(ROW_HEIGHT.comfortable);
  });

  it("draws no line into the row that is chosen, or into the one above it", async () => {
    // The highlight is a rounded shape now, and a rule running into a corner is what makes a
    // selection look like a mistake rather than a choice. A line belongs between two rows, so it
    // belongs to neither of them when one is the chosen one.
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    renderList();
    await settled();

    // Moved off the first row on purpose: the list starts on it, and a row with nothing above it
    // cannot show whether the line above a selection is left out.
    fireEvent.click(screen.getAllByRole("option")[3] as HTMLElement);

    const rows = screen.getAllByRole("option");
    const chosen = rows.findIndex(
      (row) => row.getAttribute("aria-selected") === "true",
    );

    expect(
      chosen,
      "the selection has to sit where it has neighbours on both sides",
    ).toBe(3);
    expect(
      rows[chosen]?.hasAttribute("data-separated"),
      "under the chosen row",
    ).toBe(false);
    expect(
      rows[chosen - 1]?.hasAttribute("data-separated"),
      "above the chosen row",
    ).toBe(false);
    expect(
      rows[chosen + 1]?.hasAttribute("data-separated"),
      "under the row below",
    ).toBe(true);
    // And every ordinary row still has one, or the list would have no lines at all.
    expect(
      rows.filter((row) => row.hasAttribute("data-separated")).length,
    ).toBeGreaterThan(0);
  });

  it("puts no line under the last message, because there is nothing under it", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    const rows = screen.getAllByRole("option");

    expect(rows).toHaveLength(3);
    expect(rows[2]?.hasAttribute("data-separated")).toBe(false);
    expect(rows[1]?.hasAttribute("data-separated")).toBe(true);
  });

  it("leaves the same gap above the first message as below the last", async () => {
    // A chosen first row flush against the top edge reads as cut off rather than as chosen.
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    const spacer = screen.getByRole("listbox").firstElementChild as HTMLElement;
    const first = screen.getAllByRole("option")[0] as HTMLElement;

    expect(first.style.transform).toBe(`translateY(${LIST_GUTTER}px)`);
    expect(spacer.style.height).toBe(
      `${3 * ROW_HEIGHT.comfortable + 2 * LIST_GUTTER}px`,
    );
  });

  it("keeps a hundred thousand messages out of the DOM", async () => {
    windowFor.mockImplementation(async (request) =>
      page(request.offset, 100_000),
    );
    renderList();

    await settled();
    const rendered = screen.getAllByRole("option");

    // Only the visible rows exist; the rest of the hundred thousand are height, not elements.
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered.length).toBeLessThan(100);
  });

  it("moves the active row with the keyboard instead of moving focus", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    renderList();
    await settled();

    const list = screen.getByRole("listbox");
    expect(list.getAttribute("aria-activedescendant")).toBeNull();

    await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-1",
      ),
    );

    // Focus stays on the list: the row it points at is unmounted as soon as it scrolls away.
    expect(document.activeElement).not.toBe(
      screen.getByText("Subject 1").closest("[role=option]"),
    );
  });

  it("does not walk past the ends of the list", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 3));
    renderList();
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowUp" });
    expect(list.getAttribute("aria-activedescendant")).toBe(
      "message-list-row-0",
    );

    fireEvent.keyDown(list, { key: "End" });
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-2",
      ),
    );
  });

  it("opens nothing until somebody chooses a message", async () => {
    // Coming into a mailbox is not reading anything in it. A first row highlighted and opened by
    // itself would put a message nobody chose in front of the person, still counted as unread.
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();

    expect(onOpen).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
    expect(
      screen
        .getAllByRole("option")
        .filter((row) => row.getAttribute("aria-selected") === "true"),
    ).toHaveLength(0);
    expect(
      screen.getByRole("listbox").getAttribute("aria-activedescendant"),
    ).toBeNull();
  });

  it("opens a message once, however often the list re-renders", async () => {
    // Opening a message reads it from the server. Announcing the same one again on every render
    // would be a fetch per render.
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    const onOpen = vi.fn();
    const { rerender } = render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    fireEvent.click(await screen.findByText("Subject 2"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "3", folderId: "1" }),
    );

    rerender(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );

    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("leaves the message being read as it is on Enter, the way a click on its row does", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "2", folderId: "1" }),
    );
    onOpen.mockClear();
    marking.mockClear();
    fireEvent.keyDown(list, { key: "Enter" });
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(onOpen).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
    expect(list.getAttribute("aria-activedescendant")).toBe(
      "message-list-row-1",
    );
  });

  it("opens the row that was clicked", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );

    fireEvent.click(await screen.findByText("Subject 3"));

    expect(onOpen).toHaveBeenCalledWith({ messageId: "4", folderId: "1" });
  });

  it("lets go of what it had chosen when something it does not hold is chosen", async () => {
    // A message sent from this device, opened above the list in the folder for sent mail: a row
    // still drawn as chosen would say the reader shows it.
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    const onOpen = vi.fn();
    // A new hand each time it is drawn, as a caller writing it inline gives: the list must not
    // take a new hand for a new choice.
    const list = (chosenElsewhere: number) => (
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={(opened) => onOpen(opened)}
          chosenElsewhere={chosenElsewhere}
        />
      </I18nProvider>
    );
    const { rerender } = render(list(0));
    fireEvent.click(await screen.findByText("Subject 2"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({ messageId: "3", folderId: "1" }),
    );
    expect(screen.getByRole("listbox").getAttribute("aria-activedescendant")).toBe(
      "message-list-row-2",
    );

    rerender(list(1));

    await waitFor(() => expect(onOpen).toHaveBeenLastCalledWith(null));
    expect(screen.queryByRole("option", { selected: true })).toBeNull();
    expect(
      screen.getByRole("listbox").hasAttribute("aria-activedescendant"),
    ).toBe(false);

    // Once for each choice made elsewhere: a row chosen here afterwards stays chosen, however
    // often the list is drawn again.
    fireEvent.click(screen.getByText("Subject 4"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({ messageId: "5", folderId: "1" }),
    );
    rerender(list(1));
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(onOpen).toHaveBeenLastCalledWith({ messageId: "5", folderId: "1" });
    expect(
      screen.getByRole("option", { selected: true }).textContent,
    ).toContain("Subject 4");
  });

  it("shows what the caller provides when the folder is empty", async () => {
    windowFor.mockImplementation(async () => page(0, 0));
    renderList(<p>Connect your first inbox</p>);

    expect(await screen.findByText("Connect your first inbox")).toBeTruthy();
  });

  it("explains a failure in the interface language", async () => {
    windowFor.mockRejectedValue({
      code: "storageUnavailable",
      message: "unavailable",
      path: null,
    });
    renderList();

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      screen.getByText("The local database could not be opened"),
    ).toBeTruthy();
  });
});

describe("filing a message away", () => {
  /** Renders with a way to say a message has just been filed, the way the reader does. */
  function renderFilable(rows: number) {
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, rows));
    const opened = vi.fn();

    let filed: { messageId: string; state: "gone" | "back" } | null = null;
    let generation = 0;
    const view = render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={opened}
          generation={0}
        />
      </I18nProvider>,
    );

    const draw = () =>
      view.rerender(
        <I18nProvider initial="en">
          <MessageList
            scope={{ kind: "folder", folderId: "1" }}
            onOpen={opened}
            filed={filed}
            generation={generation}
          />
        </I18nProvider>,
      );

    const file = (messageId: string) => {
      filed = { messageId, state: "gone" };
      draw();
    };

    const putBack = (messageId: string) => {
      filed = { messageId, state: "back" };
      draw();
    };

    /** A pass reports that the index changed, which is what makes the list read itself again. */
    const passed = () => {
      generation += 1;
      draw();
    };

    return { opened, file, putBack, passed };
  }

  /** Selects a row by clicking its subject, which is what sets the direction of travel. */
  function goTo(index: number) {
    fireEvent.click(screen.getByText(`Subject ${index}`));
  }

  it("lands on the message below when that is the way the reader was going", async () => {
    const { opened, file } = renderFilable(6);
    await settled();

    goTo(1);
    goTo(2);
    file("3");

    // Rows are numbered from one, so "Subject 2" is the message with identifier "3".
    // Read at once rather than eventually: the message being read has to change under the finger,
    // not when the row has finished sliding away.
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("4");
  });

  it("lands on the message above when the reader was going the other way", async () => {
    const { opened, file } = renderFilable(6);
    await settled();

    goTo(4);
    goTo(2);
    file("3");

    // Read at once rather than eventually: the message being read has to change under the finger,
    // not when the row has finished sliding away.
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("2");
  });

  it("goes the other way at the top of the folder, there being nothing above", async () => {
    const { opened, file } = renderFilable(6);
    await settled();

    goTo(3);
    goTo(0);
    file("1");

    // Read at once rather than eventually: the message being read has to change under the finger,
    // not when the row has finished sliding away.
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("2");
  });

  it("goes the other way at the end of the folder, there being nothing below", async () => {
    const { opened, file } = renderFilable(3);
    await settled();

    goTo(1);
    goTo(2);
    file("3");

    // Read at once rather than eventually: the message being read has to change under the finger,
    // not when the row has finished sliding away.
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("2");
  });

  it("closes the list up over the message, without waiting for any server", async () => {
    const { file } = renderFilable(4);
    await settled();

    goTo(1);
    file("2");

    // It plays its way out first: taking it away under the animation is what makes a list flicker.
    expect(screen.queryByText("Subject 1")).toBeTruthy();
    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());
    expect(screen.queryByText("Subject 3")).toBeTruthy();
  });

  it("leaves nothing open when the folder had one message and it has gone", async () => {
    const { opened, file } = renderFilable(1);
    await settled();

    goTo(0);
    file("1");

    expect(opened.mock.calls[opened.mock.calls.length - 1]?.[0]).toBeNull();
  });

  it("sees both out when a second is filed while the first is still sliding", async () => {
    // The wait belongs to the row, not to the last thing asked for. Tied to the effect, filing a
    // second message cancelled the first one's wait and left its row in the list for good —
    // invisible, still counted, still taking its place.
    const { file } = renderFilable(5);
    await settled();

    goTo(1);
    file("2");
    file("3");

    // Both, in one wait: two rows leaving are two timers, and a machine running the whole suite at
    // once can be slow enough that checking the second one the instant the first has gone is a
    // race the test loses rather than a defect it found.
    await waitFor(
      () => {
        expect(screen.queryByText("Subject 1")).toBeNull();
        expect(screen.queryByText("Subject 2")).toBeNull();
      },
      { timeout: 3000 },
    );
    expect(screen.queryByText("Subject 4")).toBeTruthy();
  });

  it("leaves the selection where it was picked when the row above it leaves", async () => {
    const { opened, file } = renderFilable(6);
    await settled();

    goTo(1);
    file("2");
    // Picked while the filed row is still sliding: every position below it moves up when it goes,
    // and a correction that only knew about its own choice left this one message too far down.
    goTo(4);
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("5");

    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());

    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("5");
  });

  it("does not drag somebody off their row when a different message is filed", async () => {
    const { opened, file } = renderFilable(6);
    await settled();

    goTo(4);
    const onIt = opened.mock.calls[opened.mock.calls.length - 1]?.[0];

    file("2");

    expect(opened.mock.calls[opened.mock.calls.length - 1]?.[0]).toBe(onIt);
    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());
    expect(opened.mock.calls[opened.mock.calls.length - 1]?.[0]).toBe(onIt);
  });

  it("shows the row again when the way back is taken while it is still sliding", async () => {
    const { file, putBack } = renderFilable(4);
    await settled();

    goTo(1);
    file("2");
    putBack("2");

    // Both halves: back in the list, and drawn rather than painted invisible where it sits.
    await new Promise((resume) => setTimeout(resume, 260));
    const row = screen.getByText("Subject 1").closest("[role=option]");
    expect(row?.querySelector(".opacity-0")).toBeNull();
    expect(screen.getByText("Subject 1")).toBeTruthy();
  });

  it("keeps a filed row out while an unrelated change moves the epoch", async () => {
    // The core keeps a filed message in its window, marked as leaving, until the server has
    // answered — and the epoch moves for any change in the folder at all. Forgetting what has been
    // filed whenever the epoch moves put the row somebody just archived straight back on screen.
    const { file, passed } = renderFilable(4);
    await settled();

    goTo(1);
    file("2");
    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());

    // The same rows under a new epoch: nothing about the filed message has reached the server yet.
    windowFor.mockImplementation(async (request) => ({
      ...page(request.offset, 4),
      viewEpoch: "2",
      notAnswering: [],
    }));
    const asked = windowFor.mock.calls.length;
    passed();

    await waitFor(() =>
      expect(windowFor.mock.calls.length).toBeGreaterThan(asked),
    );
    expect(screen.queryByText("Subject 1")).toBeNull();
    expect(screen.queryByText("Subject 2")).toBeTruthy();
  });

  it("moves the rows underneath rather than redrawing them in place", async () => {
    // The difference between a list that closes up and one that jerks. Keyed by position, the box
    // at each place just starts drawing the next message and everything below jumps a row's height
    // in one frame; keyed by the message, the same box stays and only its place changes, which is
    // something that can be animated.
    const { file } = renderFilable(5);
    await settled();

    goTo(1);
    const below = screen.getByText("Subject 3").closest("[role=option]");

    file("2");
    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());

    expect(screen.getByText("Subject 3").closest("[role=option]")).toBe(below);
  });

  it("offers what can be done to a message on the row that shows it", async () => {
    // The first button opens the message, so the menu is on the second one.
    renderFilable(5);
    await settled();

    fireEvent.contextMenu(screen.getByText("Subject 0"));

    expect(
      await screen.findByRole("menuitem", { name: "Move to trash" }),
    ).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Archive" })).toBeTruthy();
  });

  it("throws a message away from the menu the same way the key does", async () => {
    // The same call through the same function. Two paths is how a control that looks the same
    // starts leaving the list differently from its twin.
    deleting.mockResolvedValue(undefined);
    renderFilable(5);
    await settled();

    fireEvent.contextMenu(screen.getByText("Subject 2"));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Move to trash" }),
    );

    await waitFor(() => expect(deleting).toHaveBeenCalledWith("3", "1", false));
  });

  it("files a message away from the menu", async () => {
    archiving.mockResolvedValue(undefined);
    renderFilable(5);
    await settled();

    fireEvent.contextMenu(screen.getByText("Subject 1"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Archive" }));

    await waitFor(() => expect(archiving).toHaveBeenCalledWith("2", "1"));
  });

  it("throws the selected message away on the key every system throws things away with", async () => {
    deleting.mockResolvedValue(undefined);
    const { opened, file } = renderFilable(5);
    void file;
    await settled();

    goTo(2);
    fireEvent.keyDown(screen.getByRole("listbox"), { key: "Delete" });

    await waitFor(() => expect(deleting).toHaveBeenCalledWith("3", "1", false));
    void opened;
  });

  it("reads Backspace the same way, because that is the Delete key on a Mac", async () => {
    deleting.mockResolvedValue(undefined);
    renderFilable(5);
    await settled();

    goTo(2);
    fireEvent.keyDown(screen.getByRole("listbox"), { key: "Backspace" });

    await waitFor(() => expect(deleting).toHaveBeenCalledWith("3", "1", false));
  });

  it("throws one message away however long the key is held down", async () => {
    let answer = () => {};
    deleting.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          answer = resolve;
        }),
    );
    renderFilable(5);
    await settled();

    goTo(2);
    const list = screen.getByRole("listbox");
    fireEvent.keyDown(list, { key: "Delete" });
    fireEvent.keyDown(list, { key: "Delete" });
    fireEvent.keyDown(list, { key: "Delete" });

    expect(deleting).toHaveBeenCalledTimes(1);
    answer();
  });

  it("puts it back when the filing was taken back", async () => {
    const { file, putBack } = renderFilable(4);
    await settled();

    goTo(1);
    file("2");
    await waitFor(() => expect(screen.queryByText("Subject 1")).toBeNull());

    putBack("2");

    expect(await screen.findByText("Subject 1")).toBeTruthy();
  });
});

describe("reading a message", () => {
  it("marks a message read when somebody opens it", async () => {
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 6));
    renderList();
    await settled();

    // Row 5 is unread in this page, and clicking it is opening it.
    fireEvent.click(screen.getByText("Subject 5"));

    await waitFor(() => expect(marking).toHaveBeenCalledWith("6", "1", true));
  });

  it("opens nothing and marks nothing by being come into", async () => {
    // Row 0 is unread and is where the list starts. Coming into a folder must neither read its
    // newest message for the person nor put it in front of them.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 6));
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();

    await new Promise((resume) => setTimeout(resume, 50));
    expect(marking).not.toHaveBeenCalled();
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("opens a row clicked while nothing was chosen, and reads it once", async () => {
    // A click on a row is choosing it, and choosing a message opens it and reads it — once. A
    // second click on the same row moves nothing, so it opens and reads nothing either.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 6));
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();

    fireEvent.click(screen.getByText("Subject 0"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "1", folderId: "1" }),
    );
    expect(marking).toHaveBeenCalledTimes(1);
    expect(marking).toHaveBeenCalledWith("1", "1", true);
    expect(
      screen.getByText("Subject 0").closest("[role=option]")?.className,
    ).not.toContain("font-semibold");

    // Clicked again: it is open and read already. One the person set unread with the dot stays
    // unread, because that is what the dot is for.
    fireEvent.click(screen.getByText("Subject 0"));
    await new Promise((resume) => setTimeout(resume, 20));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(marking).toHaveBeenCalledTimes(1);
  });

  it("chooses the first message on screen with the first arrow, and reads it", async () => {
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, 6));
    renderList();
    await settled();

    const list = screen.getByRole("listbox");
    for (const key of ["Enter", "Delete", "u", "ArrowLeft", "ArrowRight"]) {
      fireEvent.keyDown(list, { key });
    }
    expect(marking).not.toHaveBeenCalled();
    expect(deleting).not.toHaveBeenCalled();
    expect(list.getAttribute("aria-activedescendant")).toBeNull();

    await chooseTheFirstRow();
    await waitFor(() => expect(marking).toHaveBeenCalledWith("1", "1", true));
  });

  it("chooses the last message on screen when the first arrow points up", async () => {
    windowFor.mockImplementation(async (request) => page(request.offset, 250));
    renderList();
    await settled();

    const list = screen.getByRole("listbox");
    fireEvent.keyDown(list, { key: "ArrowUp" });

    // Six hundred pixels of a hundred-pixel rows, after the gap above the first: rows 0 to 4 are
    // whole on the screen.
    await waitFor(() =>
      expect(list.getAttribute("aria-activedescendant")).toBe(
        "message-list-row-4",
      ),
    );
  });

  it("takes the dot off a message opened inside a conversation", async () => {
    // A message of a conversation is drawn from what was read of the conversation, not from a
    // page, and opening it has to take the dot off there too.
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => {
      const page_ = page(request.offset, 3);
      page_.rows = page_.rows.map((row, at) => ({
        ...row,
        inConversation: at === 0 ? 2 : 1,
        conversationId: at === 0 ? "a" : null,
      }));
      return page_;
    });
    conversationFor.mockResolvedValue([
      {
        ...page(0, 1).rows[0],
        messageId: "a",
        subject: "Oldest reply",
        unread: true,
      },
      { ...page(0, 1).rows[0], messageId: "1", subject: "Subject 0" },
    ]);
    renderList();
    await settled();

    fireEvent.click(screen.getByLabelText("2 messages in this conversation"));
    fireEvent.click(await screen.findByText("Oldest reply"));

    await waitFor(() => expect(marking).toHaveBeenCalledWith("a", "1", true));
    expect(screen.getByText("Oldest reply").className).not.toContain(
      "font-semibold",
    );
  });
});

describe("coming back to a mailbox", () => {
  function renderReturningTo(
    returning: {
      messageId: string | null;
      folderId: string | null;
      conversationId?: string | null;
      at: number | null;
      top: number;
    },
    rows: string[],
  ) {
    const returnTo = { conversationId: null, ...returning };
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async () => {
      const built = page(0, rows.length);
      return {
        ...built,
        rows: built.rows.map((row, at) => ({
          ...row,
          messageId: rows[at] ?? row.messageId,
          unread: true,
        })),
      };
    });
    const onOpen = vi.fn();
    const onPlace = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
          onPlace={onPlace}
          returnTo={returnTo}
        />
      </I18nProvider>,
    );
    return { onOpen, onPlace };
  }

  it("shows the message left open there as chosen, without opening or reading it again", async () => {
    // It is already the message in the reader. Opened again it would be marked read again —
    // and one the person had marked unread would stop being unread by being returned to.
    const { onOpen } = renderReturningTo(
      { messageId: "b", folderId: "1", at: 1, top: 0 },
      ["a", "b", "c"],
    );
    await settled();

    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-1"),
    );
    expect(onOpen).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
  });

  it("lets go of a message left open there that has since gone", async () => {
    const { onOpen } = renderReturningTo(
      { messageId: "gone", folderId: "1", at: 1, top: 0 },
      ["a", "b", "c"],
    );
    await settled();

    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(null));
    expect(
      screen.getByRole("listbox").getAttribute("aria-activedescendant"),
    ).toBeNull();
    expect(marking).not.toHaveBeenCalled();
  });

  it("says where the person is, so it can be come back to", async () => {
    const { onPlace } = renderReturningTo(
      { messageId: null, folderId: null, at: null, top: 0 },
      ["a", "b", "c"],
    );
    await settled();

    fireEvent.click(screen.getByText("Subject 2"));

    await waitFor(() =>
      expect(onPlace).toHaveBeenLastCalledWith({
        messageId: "c",
        folderId: "1",
        conversationId: null,
        at: 2,
        held: 3,
        top: 0,
      }),
    );
  });
});

describe("when the index changes underneath", () => {
  function renderWithGeneration(rows: number) {
    marking.mockResolvedValue(1);
    windowFor.mockImplementation(async (request) => page(request.offset, rows));
    const opened = vi.fn();
    const view = render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={opened}
          generation={0}
        />
      </I18nProvider>,
    );

    const changed = (generation: number) =>
      view.rerender(
        <I18nProvider initial="en">
          <MessageList
            scope={{ kind: "folder", folderId: "1" }}
            onOpen={opened}
            generation={generation}
          />
        </I18nProvider>,
      );

    /**
     * A message arrives at the top of the folder, which moves every row down by one.
     *
     * Built out rather than derived from `page`: what this test is about is rows changing places,
     * and a fixture that quietly answers with the same rows is how the defect it is named for
     * shipped green. Everything is unread, so a list that mistakes the shift for an opening is
     * caught marking mail read as well as showing the wrong message.
     */
    const arrives = () => {
      windowFor.mockImplementation(async (request) => {
        const total = rows + 1;
        const held = Math.min(PAGE_SIZE, Math.max(0, total - request.offset));
        return {
          rows: Array.from({ length: held }, (_, position) => {
            const index = request.offset + position;
            const before = index - 1;
            return index === 0
              ? {
                  inConversation: 1,
                  conversationId: null,
                  messageId: "999",
                  folderId: "1",
                  stopped: null,
                  subject: "Just arrived",
                  sender: "new@example.com",
                  preview: null,
                  receivedAt: 1_700_000_100,
                  unread: true,
                  leaving: false,
                  arriving: false,
                }
              : {
                  inConversation: 1,
                  conversationId: null,
                  messageId: `${before + 1}`,
                  folderId: "1",
                  stopped: null,
                  subject: `Subject ${before}`,
                  sender: `sender${before}@example.com`,
                  preview: null,
                  receivedAt: 1_700_000_000 - before * 60,
                  unread: true,
                  leaving: false,
                  arriving: false,
                };
          }),
          total,
          viewEpoch: "2",
          notAnswering: [],
        };
      });
    };

    return { opened, changed, arrives };
  }

  it("keeps the row somebody is on when a pass brings something in", async () => {
    const { opened, changed, arrives } = renderWithGeneration(6);
    await settled();

    fireEvent.click(screen.getByText("Subject 3"));
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("4");
    marking.mockClear();

    // The pass has to actually bring something in. A window that answers with the same rows under
    // the same epoch proves nothing: no row moves, so a list that does the wrong thing on a shift
    // passes anyway — which is exactly how the defect this test is named for shipped green.
    arrives();
    changed(1);
    await waitFor(() => expect(windowFor.mock.calls.length).toBeGreaterThan(1));

    // Not merely "still selected": the reader must not be handed a different message either, and
    // nothing may be marked read — nobody opened anything.
    await waitFor(() => expect(screen.queryByText("Subject 5")).toBeTruthy());
    expect(
      opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
    ).toBe("4");
    expect(marking).not.toHaveBeenCalled();
  });

  it("opens nothing when mail arrives in a mailbox that was empty", async () => {
    // The first message into an empty folder is not put in front of the reader: the person may be
    // reading something else, or nothing at all.
    marking.mockResolvedValue(1);
    let arrived = false;
    windowFor.mockImplementation(async (request) =>
      arrived ? page(request.offset, 1) : page(request.offset, 0),
    );
    const onOpen = vi.fn();
    const view = render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
          generation={0}
        />
      </I18nProvider>,
    );
    await waitFor(() => expect(windowFor).toHaveBeenCalled());

    arrived = true;
    view.rerender(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
          generation={1}
        />
      </I18nProvider>,
    );
    await screen.findByText("Subject 0");
    await new Promise((resume) => setTimeout(resume, 50));

    expect(onOpen).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
  });

  it("lets go of the message being read when a pass takes it away, rather than opening its neighbour", async () => {
    const { opened, changed } = renderWithGeneration(6);
    await settled();

    fireEvent.click(screen.getByText("Subject 3"));
    await waitFor(() =>
      expect(
        opened.mock.calls[opened.mock.calls.length - 1]?.[0]?.messageId,
      ).toBe("4"),
    );
    marking.mockClear();

    // Removed on another device: the pass brings the folder back without it.
    windowFor.mockImplementation(async () => {
      const built = page(0, 5);
      return {
        ...built,
        viewEpoch: "2",
        rows: built.rows.map((row, at) =>
          at < 3 ? row : { ...row, messageId: `${at + 2}` },
        ),
      };
    });
    changed(1);

    await waitFor(() =>
      expect(opened.mock.calls[opened.mock.calls.length - 1]?.[0]).toBeNull(),
    );
    expect(
      screen.getByRole("listbox").getAttribute("aria-activedescendant"),
    ).toBeNull();
    expect(marking).not.toHaveBeenCalled();
  });

  it("reads what it holds again rather than emptying it first", async () => {
    const { changed } = renderWithGeneration(6);
    await settled();

    changed(1);

    // The rows stay on screen while the answer is on its way: a list that blanks is a list that
    // flickers on every message that arrives.
    expect(screen.getByText("Subject 0")).toBeTruthy();
    await waitFor(() => expect(windowFor.mock.calls.length).toBeGreaterThan(1));
    expect(screen.getByText("Subject 0")).toBeTruthy();
  });
});

describe("MessageList, keys pressed on the buttons inside a row", () => {
  /** Enough rows that the row a key is pressed on is not the row the selection is on. */
  function aListOfTen() {
    windowFor.mockImplementation((request) =>
      Promise.resolve(page(request.offset, 10)),
    );
  }

  it("does not throw away the selected message when Delete is pressed on another row's button", async () => {
    // The list's key handler sits on the listbox, so a key pressed on a button inside any drawn
    // row bubbles up to it. It used to answer for them: Delete on the seventh row's read dot
    // threw away whatever the selection was on — a different message, and into the bin, with
    // nothing on screen saying which one went.
    aListOfTen();
    renderList();
    await settled();

    const dots = screen.getAllByRole("button", { name: /mark as/i });
    expect(dots.length).toBeGreaterThan(1);

    fireEvent.keyDown(dots[1], { key: "Delete", bubbles: true });

    expect(deleting).not.toHaveBeenCalled();
  });

  it("does not re-open the selected message when Enter is pressed on another row's button", async () => {
    aListOfTen();
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
        />
      </I18nProvider>,
    );
    await settled();
    onOpen.mockClear();

    const dots = screen.getAllByRole("button", { name: /mark as/i });
    fireEvent.keyDown(dots[1], { key: "Enter", bubbles: true });

    // The list answering Enter here cancelled the button's own activation and opened whatever the
    // selection was on instead — so the press did neither of the two things it could have meant.
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("still throws away the selected message when Delete is pressed on the list itself", async () => {
    // The guard must not turn the list's own keys off. Without this, the fix above would read as
    // "Delete no longer deletes", which is the same test passing for the opposite reason.
    aListOfTen();
    renderList();
    await settled();

    const list = await chooseTheFirstRow();
    fireEvent.keyDown(list, { key: "Delete" });

    await waitFor(() => expect(deleting).toHaveBeenCalled());
  });
});

describe("MessageList, reaching a row's actions without a mouse", () => {
  function aListOfTen() {
    windowFor.mockImplementation((request) =>
      Promise.resolve(page(request.offset, 10)),
    );
  }

  it("keeps the buttons inside a row out of the tab order", async () => {
    // The list draws the visible band plus twelve rows either side, so these put twenty to forty
    // stops between the list and everything after it — and each one scrolled the list as it took
    // focus, so the wall regrew while somebody walked it. The Undo offer under the list expires
    // in eight seconds and was never reachable at all.
    aListOfTen();
    renderList();
    await settled();

    for (const dot of screen.getAllByRole("button", { name: /mark as/i })) {
      expect(dot.getAttribute("tabindex")).toBe("-1");
    }
  });

  it("marks the selected message read from the keyboard", async () => {
    // Taking the dot out of the tab order without this would make marking read a thing only a
    // mouse can do — and it is the state a person changes most often after opening a message.
    aListOfTen();
    renderList();
    await settled();

    // Chosen, and read by being chosen: the key then says otherwise.
    const list = await chooseTheFirstRow();
    await waitFor(() => expect(marking).toHaveBeenCalledWith("1", "1", true));
    marking.mockClear();
    fireEvent.keyDown(list, { key: "u" });

    await waitFor(() => expect(marking).toHaveBeenCalledWith("1", "1", false));
  });
});

describe("MessageList, a stopped action in the tighter density", () => {
  /** A page whose first row carries an instruction that stopped. */
  function aStoppedRow() {
    windowFor.mockImplementation(async (request: { offset: number }) => {
      const window = page(request.offset, 3);
      const rows = [...window.rows];
      // A reason the catalogue actually has a sentence for. A made-up one would render as
      // nothing and the test would be asserting about a blank.
      if (rows[0]) rows[0] = { ...rows[0], stopped: "theServerRefused" };
      return { ...window, rows };
    });
  }

  function renderCompact() {
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          density="compact"
        />
      </I18nProvider>,
    );
  }

  it("says what stopped, in the tighter density as well as the roomy one", async () => {
    // The explanation and its button lived only in the roomy arm of the density branch, so
    // somebody reading a compact list watched an action fail with nothing said and no way to ask
    // again. The density a person chose is not a decision about whether they are told things.
    aStoppedRow();
    renderCompact();
    await settled();

    expect(await screen.findByText(/server would not do it/i)).toBeTruthy();
  });

  it("offers the way to ask again in the tighter density", async () => {
    aStoppedRow();
    renderCompact();
    await settled();

    fireEvent.click(await screen.findByRole("button", { name: /try again/i }));

    await waitFor(() => expect(asking).toHaveBeenCalled());
  });
});

describe("MessageList, saying where a row sits", () => {
  it("counts a row against the whole list, not against the page drawn", async () => {
    // The list draws about thirty rows out of however many there are, so a reader left to count
    // the ones in the page says "3 of 30" about a message three thousand rows down — a number
    // that is wrong and sounds authoritative.
    windowFor.mockImplementation((request) =>
      Promise.resolve(page(request.offset, 4000)),
    );

    renderList();
    await settled();

    const first = screen.getAllByRole("option")[0];
    expect(first.getAttribute("aria-posinset")).toBe("1");
    expect(first.getAttribute("aria-setsize")).toBe("4000");
  });
});

describe("following the message being read", () => {
  /** A row of a list of conversations, shown under its newest message. */
  function conversation(at: number, messageId: string, thread: string) {
    return {
      inConversation: 2,
      conversationId: thread,
      messageId,
      folderId: "1",
      stopped: null,
      subject: `Subject ${at}`,
      sender: `sender${at}@example.com`,
      preview: null,
      receivedAt: 1_700_000_000 - at * 60,
      unread: false,
      leaving: false,
      arriving: false,
    };
  }

  function answering(rows: ReturnType<typeof conversation>[], epoch = "1") {
    windowFor.mockImplementation(async () => ({
      rows,
      total: rows.length,
      viewEpoch: epoch,
      notAnswering: [],
    }));
  }

  function renderEverything(returnTo?: {
    messageId: string;
    conversationId: string;
    at: number;
  }) {
    const onOpen = vi.fn();
    const draw = (generation: number) => (
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "everything" }}
          onOpen={onOpen}
          generation={generation}
          returnTo={returnTo && { ...returnTo, folderId: "1", top: 0 }}
        />
      </I18nProvider>
    );
    const view = render(draw(0));
    return {
      onOpen,
      again: (generation: number) => view.rerender(draw(generation)),
    };
  }

  it("keeps a conversation open in the reader when a reply arrives in it", async () => {
    // The row shows the newest message, and the reply is newer: the row now names another
    // message, and the one being read is still in the conversation it stands for.
    answering([conversation(0, "x", "t1"), conversation(1, "y", "t2")]);
    const { onOpen, again } = renderEverything();
    await settled();
    fireEvent.click(screen.getByText("Subject 0"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "x", folderId: "1" }),
    );

    answering(
      [
        { ...conversation(0, "reply", "t1"), subject: "The reply" },
        conversation(1, "y", "t2"),
      ],
      "2",
    );
    again(1);
    await screen.findByText("The reply");
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(onOpen).not.toHaveBeenCalledWith(null);
    expect(
      screen.getByRole("listbox").getAttribute("aria-activedescendant"),
    ).toBe("message-list-row-0");
  });

  /** The reply case: `x` in the reader, and its conversation's row now showing an unread reply. */
  async function aReplyArrivedUnderTheOneBeingRead() {
    answering([conversation(0, "x", "t1"), conversation(1, "y", "t2")]);
    const { onOpen, again } = renderEverything();
    await settled();
    fireEvent.click(screen.getByText("Subject 0"));
    await waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith({ messageId: "x", folderId: "1" }),
    );

    answering(
      [
        {
          ...conversation(0, "reply", "t1"),
          subject: "The reply",
          unread: true,
        },
        conversation(1, "y", "t2"),
      ],
      "2",
    );
    again(1);
    await screen.findByText("The reply");
    marking.mockClear();
    deleting.mockClear();
    return onOpen;
  }

  it("throws away and marks only the message being read, never another its row shows", async () => {
    // The row stands for the message in the reader and shows a reply nobody has seen. Acting on the
    // row, Delete would send that reply to the trash and `u` take its dot away unseen, while the
    // message being read stays where it is.
    await aReplyArrivedUnderTheOneBeingRead();
    const list = screen.getByRole("listbox");

    fireEvent.keyDown(list, { key: "Delete" });
    fireEvent.keyDown(list, { key: "u" });
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(deleting).not.toHaveBeenCalled();
    expect(marking).not.toHaveBeenCalled();
  });

  it("opens what a row shows when the row standing for another message is chosen again", async () => {
    // Chosen already, a click moves nothing, and a click that opened only what moved would leave
    // the reply the row shows impossible to open by clicking it.
    const onOpen = await aReplyArrivedUnderTheOneBeingRead();

    fireEvent.click(screen.getByText("The reply"));

    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "reply",
        folderId: "1",
      }),
    );
    expect(marking).toHaveBeenCalledWith("reply", "1", true);
  });

  it("reads what a row shows when the row standing for another message is chosen by Enter", async () => {
    // Enter is a choice like a click. Opened straight from the key, the reply would stay unread:
    // its dot and the counts beside the folders would not move, nothing would say it had been
    // marked, and the mailbox would remember the message before it as the one left open.
    const onOpen = await aReplyArrivedUnderTheOneBeingRead();

    fireEvent.keyDown(screen.getByRole("listbox"), { key: "Enter" });

    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "reply",
        folderId: "1",
      }),
    );
    expect(marking).toHaveBeenCalledWith("reply", "1", true);
  });

  it("lets go of a message its conversation no longer holds", async () => {
    // Its row stood for it while the conversation was closed. Opened, the conversation says what
    // it holds now, and a message that has left it -- archived, deleted, moved elsewhere -- is
    // not stood for any more. Not before the answer is in, though: a conversation still being
    // read says nothing about what it holds.
    answering([conversation(0, "x", "t1"), conversation(1, "y", "t2")]);
    let answer: (rows: ReturnType<typeof conversation>[]) => void = () => {};
    conversationFor.mockImplementation(
      () => new Promise((resolve) => (answer = resolve)),
    );
    const { onOpen } = renderEverything({
      messageId: "older",
      conversationId: "t1",
      at: 0,
    });
    await settled();
    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-0"),
    );

    fireEvent.keyDown(screen.getByRole("listbox"), { key: "ArrowRight" });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(onOpen).not.toHaveBeenCalledWith(null);

    await act(async () =>
      answer([{ ...conversation(0, "x", "t1"), inConversation: 1 }]),
    );

    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(null));
  });

  it("comes back to a message read inside a conversation, on the conversation's row", async () => {
    answering([conversation(0, "x", "t1"), conversation(1, "y", "t2")]);
    const { onOpen } = renderEverything({
      messageId: "older",
      conversationId: "t1",
      at: 0,
    });
    await settled();

    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-0"),
    );
    expect(onOpen).not.toHaveBeenCalled();
  });

  /** A folder of `rows` messages, with the one being read at `at` when there is one. */
  function folderHolding(rows: number, kept: { at: number } | null) {
    windowFor.mockImplementation(async (request) => {
      const built = page(request.offset, rows);
      return {
        ...built,
        rows: built.rows.map((row, position) =>
          request.offset + position === kept?.at
            ? { ...row, messageId: "kept" }
            : row,
        ),
      };
    });
  }

  function renderReading(at: number, held?: number) {
    const onOpen = vi.fn();
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          onOpen={onOpen}
          returnTo={{
            messageId: "kept",
            folderId: "1",
            conversationId: null,
            at,
            held,
            top: 0,
          }}
        />
      </I18nProvider>,
    );
    return onOpen;
  }

  it("finds a message that new mail pushed onto the next page rather than letting it go", async () => {
    // Left at row 99, the last of the first page; two messages arrived since, and it is at 101.
    folderHolding(250, { at: 101 });
    const onOpen = renderReading(99);
    await settled();

    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-101"),
    );
    expect(onOpen).not.toHaveBeenCalledWith(null);
  });

  it("finds a message more mail arrived above while the mailbox was left than a page holds", async () => {
    // Left at row 30 of fifty; two hundred arrived since, and it is at 230. Counted as though
    // nothing had arrived, only the pages within a page of row 30 would be read, it is on none of
    // them, and the reader would be closed on a message that is still there.
    folderHolding(250, { at: 230 });
    const onOpen = renderReading(30, 50);
    await settled();

    await waitFor(() =>
      expect(
        screen.getByRole("listbox").getAttribute("aria-activedescendant"),
      ).toBe("message-list-row-230"),
    );
    expect(onOpen).not.toHaveBeenCalledWith(null);
  });

  it("lets go of a message that is on none of the pages it could have moved to", async () => {
    folderHolding(250, null);
    const onOpen = renderReading(99);
    await settled();

    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(null));
    const asked = windowFor.mock.calls.map(([request]) => request.offset);
    expect(asked).toContain(PAGE_SIZE);
  });
});

describe("the place a list is scrolled back to", () => {
  // How far the list could scroll, which this environment reports as nothing: every place asked
  // for would come back as the top.
  beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        return 100_000;
      },
    });
  });
  afterAll(() => {
    delete (HTMLElement.prototype as { scrollHeight?: number }).scrollHeight;
  });

  function renderAt(top: number) {
    windowFor.mockImplementation(async (request) => page(request.offset, 300));
    render(
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          returnTo={{
            messageId: null,
            folderId: null,
            conversationId: null,
            at: null,
            top,
          }}
        />
      </I18nProvider>,
    );
  }

  it("leaves a list left at its top where it starts, gutter and all", async () => {
    const scrolled = vi.spyOn(HTMLElement.prototype, "scrollTo");
    renderAt(0);
    await settled();

    const tops = scrolled.mock.calls.map(
      ([to]) => (to as ScrollToOptions | undefined)?.top,
    );
    scrolled.mockRestore();
    expect(tops).not.toContain(LIST_GUTTER);
  });

  it("puts the row left at the top back there with the gutter above it", async () => {
    const scrolled = vi.spyOn(HTMLElement.prototype, "scrollTo");
    renderAt(5);
    await settled();

    const tops = scrolled.mock.calls.map(
      ([to]) => (to as ScrollToOptions | undefined)?.top,
    );
    scrolled.mockRestore();
    expect(tops).toContain(5 * ROW_HEIGHT.comfortable);
  });
});

describe("a dot the person took away", () => {
  it("stays away through an answer the core gave before it wrote the change down", async () => {
    // A pass announcing its own change re-reads the list while the write is still on its way. That
    // answer says the message is unread, and laid over the rows as it came it puts the dot back
    // under a message just opened.
    let written: () => void = () => {};
    marking.mockImplementation(
      () => new Promise<number>((resolve) => (written = () => resolve(1))),
    );
    // Each answer names itself on its second row, so the test knows it has landed.
    const answeringAs = (said: string) =>
      windowFor.mockImplementation(async (request) => {
        const built = page(request.offset, 6);
        return {
          ...built,
          rows: built.rows.map((row, at) =>
            at === 1 ? { ...row, subject: said } : row,
          ),
        };
      });
    answeringAs("First answer");
    const draw = (generation: number) => (
      <I18nProvider initial="en">
        <MessageList
          scope={{ kind: "folder", folderId: "1" }}
          generation={generation}
        />
      </I18nProvider>
    );
    const view = render(draw(0));
    await settled();
    const dots = () =>
      screen.queryAllByRole("button", { name: "Mark as read" }).length;
    expect(dots()).toBe(2);

    fireEvent.click(screen.getByText("Subject 5"));
    await waitFor(() => expect(dots()).toBe(1));

    answeringAs("Asked before the write");
    view.rerender(draw(1));
    await screen.findByText("Asked before the write");
    expect(dots()).toBe(1);

    // Written down: what the core says from now on is its word, taken as it is — an undo, say.
    written();
    await new Promise((resolve) => setTimeout(resolve, 0));
    answeringAs("Asked after the write");
    view.rerender(draw(2));
    await screen.findByText("Asked after the write");
    expect(dots()).toBe(2);
  });
});
