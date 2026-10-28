import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { DensityDto } from "@/shared/ipc/generated/DensityDto";
import type { KeptDraftDto } from "@/shared/ipc/generated/KeptDraftDto";
import type { ViewWindowDto } from "@/shared/ipc/generated/ViewWindowDto";

import { DraftsInFolder } from "./DraftsInFolder";
import { ROW_HEIGHT } from "./rows";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    keptDrafts: vi.fn(),
    discardDraft: vi.fn(),
    getViewWindow: vi.fn(),
    markMessageRead: vi.fn(),
  };
});

const { keptDrafts, discardDraft, getViewWindow, markMessageRead } =
  await import("@/shared/ipc/client");
const kept = vi.mocked(keptDrafts);
const throwingAway = vi.mocked(discardDraft);
const windowFor = vi.mocked(getViewWindow);
const marking = vi.mocked(markMessageRead);

const DRAFT = {
  id: 4,
  recipients: "kappa@example.net",
  copies: "",
  blind: "sigma@example.org",
  subject: "About Tuesday",
  body: "half a thought",
  answering: "<tuesday@example.com>",
  chain: ["<tuesday@example.com>"],
};

/** A draft kept here under its own number and subject, and nothing else worth telling apart. */
function keptHere(id: number, subject: string): KeptDraftDto {
  return {
    id,
    recipients: "kappa@example.net",
    copies: "",
    blind: "",
    subject,
    body: "",
    answering: null,
    chain: [],
  };
}

/** What the server's drafts folder holds, as the list is handed it. */
function onTheServer(subjects: string[]): ViewWindowDto {
  return {
    rows: subjects.map((subject, index) => ({
      inConversation: 1,
      conversationId: null,
      messageId: `${index + 1}`,
      folderId: "7",
      stopped: null,
      subject,
      sender: "zeta@example.com",
      preview: null,
      receivedAt: 1_700_000_000 - index * 60,
      unread: false,
      leaving: false,
      arriving: false,
    })),
    total: subjects.length,
    viewEpoch: "1",
    notAnswering: [],
  };
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

function show({ density }: { density?: DensityDto } = {}) {
  const onOpenDraft = vi.fn();
  const onChanged = vi.fn();
  const onOpen = vi.fn();
  const onHolds = vi.fn();
  render(
    <I18nProvider initial="en">
      {/* Where the focus goes when nothing in the folder is left to take it. */}
      <input data-search aria-label="Search" />
      <DraftsInFolder
        accountId="acc-1"
        list={{
          scope: { kind: "folder", folderId: "7" },
          generation: 0,
          density,
          onOpen,
          onHolds,
        }}
        onOpenDraft={onOpenDraft}
        onChanged={onChanged}
      />
    </I18nProvider>,
  );
  return { onOpenDraft, onChanged, onOpen, onHolds };
}

beforeEach(() => {
  kept.mockReset();
  kept.mockResolvedValue([DRAFT]);
  throwingAway.mockReset();
  throwingAway.mockResolvedValue(undefined);
  windowFor.mockReset();
  windowFor.mockResolvedValue(onTheServer([]));
  marking.mockReset();
  marking.mockResolvedValue(1);
});

describe("the drafts folder", () => {
  it("shows a draft the server holds, which was counted and never shown", async () => {
    // Saved by another client or by webmail and brought in by a sync. The sidebar counts it, and
    // getting new mail keeps it, because it is there -- so the folder has to show it.
    kept.mockResolvedValue([]);
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    show();

    expect(await screen.findByText("From the phone")).toBeTruthy();
    expect(screen.queryByText("This folder is empty")).toBeNull();
    // One part, so nothing to tell apart.
    expect(screen.queryByRole("heading")).toBeNull();
  });

  it("shows what was written down here, and no empty line from the server's part", async () => {
    show();

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
    expect(screen.getByText("kappa@example.net")).toBeTruthy();
    // What was written keeps its own direction, as the mail in the list below does.
    expect(screen.getByText("About Tuesday").getAttribute("dir")).toBe("auto");
    expect(screen.getByText("kappa@example.net").getAttribute("dir")).toBe(
      "auto",
    );
    await waitFor(() => expect(windowFor).toHaveBeenCalled());
    expect(screen.queryByText("This folder is empty")).toBeNull();
    expect(screen.queryByRole("heading")).toBeNull();
  });

  it("shows both, each under a heading of its own", async () => {
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    show();

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
    expect(await screen.findByText("From the phone")).toBeTruthy();
    expect(
      screen.getAllByRole("heading").map((heading) => heading.textContent),
    ).toEqual(["On this device", "On the server"]);
    expect(screen.queryByText("This folder is empty")).toBeNull();
  });

  it("says it is empty once, when neither place has anything", async () => {
    kept.mockResolvedValue([]);
    show();

    expect(await screen.findByText("This folder is empty")).toBeTruthy();
    expect(screen.getAllByText("This folder is empty")).toHaveLength(1);
    expect(screen.queryByRole("heading")).toBeNull();
  });

  it("says nothing about being empty while what is kept here has not been read yet", async () => {
    // The server's part answering first with nothing would otherwise say "empty" over drafts that
    // are a moment from arriving -- and so would the pane beside it.
    kept.mockReturnValue(new Promise(() => {}));
    const { onHolds } = show();

    await waitFor(() => expect(windowFor).toHaveBeenCalled());
    await settleDown();
    expect(screen.queryByText("This folder is empty")).toBeNull();
    expect(onHolds).not.toHaveBeenCalledWith(0);
  });

  it("says it could not read what is kept here, rather than that there is nothing", async () => {
    // The count beside the folder counts these, so "empty" under it would be the very number over
    // nothing this folder was rebuilt to stop saying.
    kept.mockRejectedValue({ code: "internal", message: "", path: null });
    const { onHolds } = show();

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByText("This folder is empty")).toBeNull();
    expect(onHolds).not.toHaveBeenCalledWith(0);
  });

  it("still shows the server's part when what is kept here could not be read", async () => {
    kept.mockRejectedValue({ code: "internal", message: "", path: null });
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    show();

    expect(await screen.findByText("From the phone")).toBeTruthy();
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(
      screen.getAllByRole("heading").map((heading) => heading.textContent),
    ).toEqual(["On this device", "On the server"]);
  });

  it("still shows what is kept here when the server's part could not be read", async () => {
    windowFor.mockRejectedValue({ code: "internal", message: "", path: null });
    show();

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
    expect(screen.getByRole("alert")).toBeTruthy();
  });

  it("draws nothing of what is kept here until the server's part has answered", async () => {
    // Drawn first and alone, the rows would be pushed down by a heading a moment later, under the
    // pointer of somebody about to press one.
    const server = held<ViewWindowDto>();
    windowFor.mockReturnValue(server.promise);
    show();

    await waitFor(() => expect(kept).toHaveBeenCalled());
    await settleDown();
    expect(screen.queryByText("About Tuesday")).toBeNull();

    await act(async () => server.settle(onTheServer(["From the phone"])));

    expect(await screen.findByText("About Tuesday")).toBeTruthy();
    expect(screen.getAllByRole("heading")).toHaveLength(2);
  });

  it("keeps the server's part out of sight until what is kept here is known", async () => {
    // The other order: the server's rows at the top of the column, and then three drafts and two
    // headings arriving above them.
    const here = held<KeptDraftDto[]>();
    kept.mockReturnValue(here.promise);
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    show();

    await waitFor(() => expect(windowFor).toHaveBeenCalled());
    await settleDown();
    expect(screen.queryByRole("listbox")).toBeNull();

    await act(async () => here.settle([DRAFT]));

    expect(await screen.findByRole("listbox")).toBeTruthy();
    expect(screen.getByText("About Tuesday")).toBeTruthy();
  });

  it("opens a draft on the server in the reader, as the message it is", async () => {
    kept.mockResolvedValue([]);
    windowFor.mockResolvedValue(
      onTheServer(["From the phone", "From the web"]),
    );
    const { onOpen, onOpenDraft } = show();

    await screen.findByText("From the web");
    fireEvent.click(screen.getByText("From the web"));

    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "2",
        folderId: "7",
      }),
    );
    expect(onOpenDraft).not.toHaveBeenCalled();
  });

  it("walks the server's part by key, as any list of mail", async () => {
    windowFor.mockResolvedValue(
      onTheServer(["From the phone", "From the web"]),
    );
    const { onOpen } = show();

    await screen.findByText("From the web");
    const list = await screen.findByRole("listbox");
    // The first press chooses the first message, the second moves on from it.
    fireEvent.keyDown(list, { key: "ArrowDown" });
    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "1",
        folderId: "7",
      }),
    );
    fireEvent.keyDown(list, { key: "ArrowDown" });

    await waitFor(() =>
      expect(onOpen).toHaveBeenLastCalledWith({
        messageId: "2",
        folderId: "7",
      }),
    );
  });

  it("tells the window how many the folder shows, from both places", async () => {
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    const { onHolds } = show();

    await waitFor(() => expect(onHolds).toHaveBeenLastCalledWith(2));
  });

  it("tells the window there is something here when only drafts kept here are", async () => {
    // Told the server's count alone, the pane beside two drafts said there was nothing to read.
    kept.mockResolvedValue([keptHere(1, "First"), keptHere(2, "Second")]);
    const { onHolds } = show();

    await screen.findByText("Second");
    await waitFor(() => expect(onHolds).toHaveBeenLastCalledWith(2));
    expect(onHolds).not.toHaveBeenCalledWith(0);
  });

  it("opens one written here as the draft it is, conversation and blind copies and all", async () => {
    const { onOpenDraft } = show();
    fireEvent.click(await screen.findByText("About Tuesday"));

    expect(onOpenDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        // Under the number it already has: opened without it, saving would leave a second copy in
        // the folder beside the first every time.
        draftId: 4,
        to: "kappa@example.net",
        bcc: "sigma@example.org",
        subject: "About Tuesday",
        // Still inside its conversation: sent without this it starts a new one for whoever
        // receives it, and that cannot be undone.
        answering: "<tuesday@example.com>",
        chain: ["<tuesday@example.com>"],
        fromAccount: "acc-1",
      }),
    );
  });

  it("draws its rows the way the list below draws its own", async () => {
    kept.mockResolvedValue([keptHere(1, "First"), keptHere(2, "Second")]);
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
    // One line at the list's own height, as the compact list below draws.
    expect(rows[0].style.height).toBe(`${ROW_HEIGHT.compact}px`);
    // And what was written keeps its own direction on one line as it does on two.
    expect(screen.getByText("Second").getAttribute("dir")).toBe("auto");
    expect(
      within(rows[1])
        .getByText(
          (_, element) =>
            element?.tagName === "SPAN" && element.className.includes("w-48"),
        )
        .getAttribute("dir"),
    ).toBe("auto");
  });

  it("is two stops for the keyboard however many are kept, and walked by the arrows", async () => {
    kept.mockResolvedValue([
      keptHere(1, "First"),
      keptHere(2, "Second"),
      keptHere(3, "Third"),
    ]);
    show();

    await screen.findByText("Third");
    const opening = ["First", "Second", "Third"].map((subject) =>
      screen.getByRole("button", { name: new RegExp(`^${subject}`) }),
    );
    const discarding = screen.getAllByRole("button", {
      name: /discard the draft/i,
    });
    // The draft being stood on and its discard, and nothing else: the list of mail below is
    // reached with the next press of Tab, not after two for every draft.
    expect(opening.map((button) => button.tabIndex)).toEqual([0, -1, -1]);
    expect(discarding.map((button) => button.tabIndex)).toEqual([0, -1, -1]);

    opening[0].focus();
    fireEvent.keyDown(opening[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(opening[1]);
    expect(opening.map((button) => button.tabIndex)).toEqual([-1, 0, -1]);

    fireEvent.keyDown(opening[1], { key: "End" });
    expect(document.activeElement).toBe(opening[2]);
    fireEvent.keyDown(opening[2], { key: "ArrowDown" });
    expect(document.activeElement).toBe(opening[2]);

    fireEvent.keyDown(opening[2], { key: "Home" });
    expect(document.activeElement).toBe(opening[0]);
    fireEvent.keyDown(opening[0], { key: "ArrowUp" });
    expect(document.activeElement).toBe(opening[0]);
  });

  it("throws one away, reads the folder again and has the count read again", async () => {
    const { onChanged } = show();
    await screen.findByText("About Tuesday");
    kept.mockResolvedValue([]);

    fireEvent.click(screen.getByRole("button", { name: /discard the draft/i }));

    await waitFor(() => expect(throwingAway).toHaveBeenCalledWith(4));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    // The last one gone, and the server holding nothing: the folder says so, once.
    expect(await screen.findByText("This folder is empty")).toBeTruthy();
    expect(screen.queryByText("About Tuesday")).toBeNull();
  });

  it("puts the focus on the next draft when one is thrown away", async () => {
    // Left alone it fell to the document, and the next Tab started again at the top of the window.
    kept.mockResolvedValue([
      keptHere(1, "First"),
      keptHere(2, "Second"),
      keptHere(3, "Third"),
    ]);
    show();
    await screen.findByText("Third");
    kept.mockResolvedValue([keptHere(1, "First"), keptHere(3, "Third")]);

    const discard = screen.getByRole("button", {
      name: "Discard the draft Second",
    });
    discard.focus();
    fireEvent.click(discard);

    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: /^Third/ }),
      ),
    );
  });

  it("puts the focus on the server's part when the last one kept here is thrown away", async () => {
    windowFor.mockResolvedValue(onTheServer(["From the phone"]));
    show();
    await screen.findByText("About Tuesday");
    kept.mockResolvedValue([]);

    const discard = screen.getByRole("button", { name: /discard the draft/i });
    discard.focus();
    fireEvent.click(discard);

    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByRole("listbox")),
    );
  });

  it("puts the focus on the search when nothing is left in the folder at all", async () => {
    show();
    await screen.findByText("About Tuesday");
    kept.mockResolvedValue([]);

    const discard = screen.getByRole("button", { name: /discard the draft/i });
    discard.focus();
    fireEvent.click(discard);

    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByLabelText("Search")),
    );
  });

  it("leaves the focus where it is when it was never on the draft thrown away", async () => {
    // A click that does not take the focus, which is what a button does on macOS: the person is
    // somewhere else, and pulling the focus to a row would be the window moving them.
    kept.mockResolvedValue([keptHere(1, "First"), keptHere(2, "Second")]);
    show();
    await screen.findByText("Second");
    kept.mockResolvedValue([keptHere(2, "Second")]);
    const search = screen.getByLabelText("Search");
    search.focus();

    fireEvent.click(
      screen.getByRole("button", { name: "Discard the draft First" }),
    );

    await waitFor(() => expect(screen.queryByText("First")).toBeNull());
    expect(document.activeElement).toBe(search);
  });

  it("leaves it where it is when it could not be thrown away", async () => {
    // A row that vanishes on a write that failed is a draft a person believes they have lost.
    throwingAway.mockRejectedValue({
      code: "internal",
      message: "",
      path: null,
    });
    const { onChanged } = show();
    await screen.findByText("About Tuesday");

    fireEvent.click(screen.getByRole("button", { name: /discard the draft/i }));

    await waitFor(() => expect(throwingAway).toHaveBeenCalled());
    expect(onChanged).not.toHaveBeenCalled();
    expect(screen.getByText("About Tuesday")).toBeTruthy();
  });
});
