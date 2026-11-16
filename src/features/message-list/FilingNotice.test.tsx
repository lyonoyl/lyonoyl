import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, useI18n } from "@/shared/i18n";
import {
  FilingNotice,
  NOTICE_MS,
  type Filed,
  type PutAway,
} from "./FilingNotice";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    undoAction: vi.fn(),
    markMessageRead: vi.fn(),
    getMailboxTree: vi.fn(),
  };
});

const { undoAction, markMessageRead, getMailboxTree } =
  await import("@/shared/ipc/client");
const undoing = vi.mocked(undoAction);
const marking = vi.mocked(markMessageRead);
const folders = vi.mocked(getMailboxTree);

/** One account's folders, as the sidebar has them: its inbox, and one of the person's own. */
const FOLDERS = {
  askedThrough: 0,
  accounts: [
    {
      accountId: "acc-1",
      email: "user@example.com",
      displayName: "User",
      collapsed: false,
      place: 0,
      answered: true,
      unread: 0,
      folders: [
        {
          folderId: "1",
          name: "INBOX",
          role: "inbox" as const,
          total: 3,
          unread: 0,
          draftsHere: 0,
          canReceive: true,
        },
        {
          folderId: "5",
          name: "Projects",
          role: "custom" as const,
          total: 1,
          unread: 0,
          draftsHere: 0,
          canReceive: true,
        },
      ],
    },
  ],
};

const ARCHIVED: Filed = { messageId: "42", folderId: "1", what: "archived" };
const READ: Filed = { messageId: "42", folderId: "1", what: "read" };

function Harness({
  filed,
  onTakenBack,
  onUnreadAgain,
}: {
  filed: Filed | null;
  onTakenBack: (id: string) => void;
  onUnreadAgain?: (id: string, folderId: string) => void;
}) {
  const { t } = useI18n();
  return (
    <FilingNotice
      filed={filed}
      onTakenBack={onTakenBack}
      onUnreadAgain={onUnreadAgain}
      t={t}
    />
  );
}

function show(filed: Filed | null, onTakenBack = vi.fn()) {
  const view = render(
    <I18nProvider initial="en">
      <Harness filed={filed} onTakenBack={onTakenBack} />
    </I18nProvider>,
  );
  return { view, onTakenBack };
}

/**
 * Lets whole seconds pass, one at a time.
 *
 * The countdown schedules each second from the one before it, and the next is scheduled by an
 * effect -- which runs only when React has re-rendered. Five seconds advanced in one go fires one
 * timer and leaves the rest unscheduled, which reads as a countdown that stopped.
 */
function seconds(many: number) {
  for (let passed = 0; passed < many; passed += 1) {
    act(() => {
      vi.advanceTimersByTime(1000);
    });
  }
}

beforeEach(() => {
  undoing.mockReset();
  folders.mockReset();
  folders.mockResolvedValue(FOLDERS);
  vi.useRealTimers();
});

describe("the line under the list", () => {
  it("says nothing when nothing has been filed", () => {
    show(null);
    expect(screen.queryByText("Moved to the archive")).toBeNull();
  });

  it("says what happened and offers the way back", () => {
    show(ARCHIVED);
    expect(screen.getByText("Moved to the archive")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Undo" })).toBeTruthy();
  });

  it("says which of the two happened", () => {
    show({ ...ARCHIVED, what: "deleted" });
    expect(screen.getByText("Moved to the trash")).toBeTruthy();
  });

  it("takes the message back and tells whoever is showing the list", async () => {
    undoing.mockResolvedValue(true);
    const { onTakenBack } = show(ARCHIVED);

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    await waitFor(() => expect(onTakenBack).toHaveBeenCalledWith("42", false));
    expect(undoing).toHaveBeenCalledWith("42", "1");
    await waitFor(() =>
      expect(screen.queryByText("Moved to the archive")).toBeNull(),
    );
  });

  it("says plainly when a message cannot be brought back", async () => {
    // Taking it back is two things underneath: removing an instruction that has not gone, or
    // asking for the message back once it has. "No" means neither was possible — the message was
    // deleted for good, or it is not where this device left it.
    undoing.mockResolvedValue(false);
    const { onTakenBack } = show(ARCHIVED);

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    expect(
      await screen.findByText("Lyonoyl Mail could not put the message back"),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
    expect(onTakenBack).not.toHaveBeenCalled();
  });

  it("leaves the offer standing when taking it back did not work", async () => {
    undoing.mockRejectedValue(new Error("no"));
    show(ARCHIVED);

    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    await waitFor(() => expect(undoing).toHaveBeenCalled());
    expect(screen.getByText("Moved to the archive")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Undo" })).toBeTruthy();
  });

  it("takes itself away, so there is nothing left to dismiss", () => {
    vi.useFakeTimers();
    show(ARCHIVED);
    expect(screen.getByText("Moved to the archive")).toBeTruthy();

    // The countdown, and then the going. It stays on screen while it goes, so a message is not
    // told about something that blinks out of existence.
    seconds(NOTICE_MS / 1000);
    expect(
      screen.getByText("Moved to the archive"),
      "gone before it had finished going",
    ).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(screen.queryByText("Moved to the archive")).toBeNull();
  });

  it("says when it has gone, so the corner is free again", () => {
    // What the window holds is "a notice is showing". Never told otherwise, it held that for the
    // rest of the session after the first message anybody filed -- and the offer to empty the
    // trash, which wants the same corner, was never seen again.
    vi.useFakeTimers();
    const gone = vi.fn();
    render(
      <I18nProvider initial="en">
        <FilingNotice
          filed={ARCHIVED}
          onTakenBack={vi.fn()}
          onGone={gone}
          t={((key: string) => key) as never}
        />
      </I18nProvider>,
    );

    seconds(NOTICE_MS / 1000);
    expect(gone).not.toHaveBeenCalled();

    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(gone).toHaveBeenCalled();
  });

  it("says how long is left, and stops saying it while somebody is reading", () => {
    // A thing that vanishes on its own without saying when is a thing nobody can plan around:
    // they hurry for no reason, or lose the offer mid-reach.
    vi.useFakeTimers();
    const { view } = show(ARCHIVED);

    const said = () =>
      view.container.querySelector(".tabular-nums")?.textContent;
    expect(said()).toBe(String(NOTICE_MS / 1000));

    seconds(2);
    expect(said()).toBe(String(NOTICE_MS / 1000 - 2));

    // Read by somebody, so the clock waits for them.
    fireEvent.mouseEnter(screen.getByText("Moved to the archive"));
    seconds(5);
    expect(said(), "the clock ran on under the person reading it").toBe(
      String(NOTICE_MS / 1000 - 2),
    );
    expect(screen.getByText("Moved to the archive")).toBeTruthy();
  });
  it("puts a message marked read back to unread by handing it to the list", async () => {
    // The way back here is not a queued instruction that may already have gone through: it is
    // marking it unread, which can be done at any time and is never too late. The list does it the
    // way its dot does, so the dot and the counts move together; marked here as well, it would be
    // marked twice.
    marking.mockReset();
    const onUnreadAgain = vi.fn();
    render(
      <I18nProvider initial="en">
        <Harness
          filed={READ}
          onTakenBack={vi.fn()}
          onUnreadAgain={onUnreadAgain}
        />
      </I18nProvider>,
    );

    expect(screen.getByText("Marked read")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    await waitFor(() => expect(onUnreadAgain).toHaveBeenCalledWith("42", "1"));
    expect(marking).not.toHaveBeenCalled();
    expect(undoing).not.toHaveBeenCalled();
    expect(screen.queryByText("Marked read")).toBeNull();
  });
});

describe("FilingNotice, reaching Undo without a mouse", () => {
  it("stops the clock while somebody is standing on the offer", async () => {
    // Eight seconds is enough to notice a mistake and not enough to reach the offer by keyboard:
    // the list is between them and Tab has to cross it. The offer used to be taken away from
    // under the very person who had got to it.
    vi.useFakeTimers();
    try {
      show({ messageId: "1", folderId: "1", what: "deleted" });

      const undo = screen.getByRole("button");
      fireEvent.focus(undo);

      await act(async () => {
        vi.advanceTimersByTime(30_000);
      });

      expect(screen.queryByRole("button")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("FilingNotice, asked by the key", () => {
  const DELETED: Filed = { messageId: "7", folderId: "1", what: "deleted" };

  function Asked({
    filed,
    asked,
    onTakenBack = () => {},
  }: {
    filed: PutAway | null;
    asked: number;
    onTakenBack?: (id: string) => void;
  }) {
    const { t } = useI18n();
    return (
      <FilingNotice
        filed={filed}
        takeBackAsked={asked}
        onTakenBack={onTakenBack}
        t={t}
      />
    );
  }

  /** The same, told when it has gone. */
  function Gone({
    filed,
    asked,
    onGone,
  }: {
    filed: PutAway | null;
    asked: number;
    onGone: () => void;
  }) {
    const { t } = useI18n();
    return (
      <FilingNotice
        filed={filed}
        takeBackAsked={asked}
        onTakenBack={() => {}}
        onGone={onGone}
        t={t}
      />
    );
  }

  function noticing(filed: PutAway | null, asked = 0, onTakenBack = vi.fn()) {
    const view = render(
      <I18nProvider initial="en">
        <Asked filed={filed} asked={asked} onTakenBack={onTakenBack} />
      </I18nProvider>,
    );
    const ask = (next: PutAway | null, times: number) =>
      view.rerender(
        <I18nProvider initial="en">
          <Asked filed={next} asked={times} onTakenBack={onTakenBack} />
        </I18nProvider>,
      );
    return { view, ask, onTakenBack };
  }

  /** An answer from the core that comes when the test says. */
  function answerLater() {
    let answer: (takenBack: boolean) => void = () => {};
    undoing.mockImplementation(
      () => new Promise<boolean>((resolve) => (answer = resolve)),
    );
    return (takenBack: boolean) => act(() => answer(takenBack));
  }

  it("takes back what it says when the key is pressed, as its button does", async () => {
    undoing.mockResolvedValue(true);
    const { ask, onTakenBack } = noticing(ARCHIVED);

    ask(ARCHIVED, 1);

    await waitFor(() => expect(onTakenBack).toHaveBeenCalledWith("42", false));
    expect(undoing).toHaveBeenCalledWith("42", "1");
    expect(screen.queryByText("Moved to the archive")).toBeNull();
  });

  it("draws nothing of a message the key raised until its answer, and then where it went back to", async () => {
    // Drawn while the answer is on its way, the line would say the message had been put away just
    // as the person asked for it back. Once it is back, the folder it went back to may not be the
    // one on the screen, and the line is all there is to see.
    const answer = answerLater();
    const { ask, onTakenBack } = noticing(null);

    ask(DELETED, 1);

    expect(undoing).toHaveBeenCalledWith("7", "1");
    expect(screen.queryByText("Moved to the trash")).toBeNull();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();

    await answer(true);

    expect(await screen.findByText("Moved back to Inbox")).toBeTruthy();
    expect(onTakenBack).toHaveBeenCalledWith("7", true);
    expect(screen.queryByText("Moved to the trash")).toBeNull();
    // Nothing is left to take back.
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });

  it("names a folder of the person's own as the server names it", async () => {
    undoing.mockResolvedValue(true);
    const { ask } = noticing(null);

    ask({ messageId: "8", folderId: "5", what: "archived" }, 1);

    expect(await screen.findByText("Moved back to Projects")).toBeTruthy();
  });

  it("says nothing once a message is back when its folder cannot be named", async () => {
    // A line naming no folder would say less than nothing: back is all the list already shows.
    undoing.mockResolvedValue(true);
    folders.mockRejectedValue(new Error("unreadable"));
    const { view, ask, onTakenBack } = noticing(null);

    ask(DELETED, 1);

    await waitFor(() => expect(onTakenBack).toHaveBeenCalledWith("7", false));
    expect(view.container.textContent).toBe("");
  });

  it("says nothing of where a message went back once another line has taken its place", async () => {
    const answer = answerLater();
    const { ask, onTakenBack } = noticing(null);
    ask(DELETED, 1);

    ask(ARCHIVED, 1);
    await answer(true);

    expect(onTakenBack).toHaveBeenCalledWith("7", false);
    expect(screen.queryByText(/Moved back to/)).toBeNull();
    expect(screen.getByText("Moved to the archive")).toBeTruthy();
  });

  it("goes by itself once it has said where the message went back to", async () => {
    vi.useFakeTimers();
    const onGone = vi.fn();
    undoing.mockResolvedValue(true);
    const view = render(
      <I18nProvider initial="en">
        <Gone filed={null} asked={0} onGone={onGone} />
      </I18nProvider>,
    );
    view.rerender(
      <I18nProvider initial="en">
        <Gone filed={DELETED} asked={1} onGone={onGone} />
      </I18nProvider>,
    );
    await act(async () => {});
    await act(async () => {});
    expect(screen.getByText("Moved back to Inbox")).toBeTruthy();

    seconds(NOTICE_MS / 1000);
    act(() => {
      vi.advanceTimersByTime(250);
    });

    expect(onGone).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Moved back to Inbox")).toBeNull();
  });

  /** Raises a line by the key and answers it as the test says, told when it has gone. */
  function raised(onGone: () => void) {
    const view = render(
      <I18nProvider initial="en">
        <Gone filed={null} asked={0} onGone={onGone} />
      </I18nProvider>,
    );
    view.rerender(
      <I18nProvider initial="en">
        <Gone filed={DELETED} asked={1} onGone={onGone} />
      </I18nProvider>,
    );
    return view;
  }

  it("is forgotten when it leaves the tree having said where the message went back to", async () => {
    // A screen opened over the list takes the line with it. Held, the screen closed again would
    // draw the offer the line had answered, about a message already back.
    const onGone = vi.fn();
    undoing.mockResolvedValue(true);
    const view = raised(onGone);
    await screen.findByText("Moved back to Inbox");

    view.unmount();

    expect(onGone).toHaveBeenCalledTimes(1);
  });

  it("is forgotten when it leaves the tree having said a message cannot be brought back", async () => {
    const onGone = vi.fn();
    undoing.mockResolvedValue(false);
    const view = raised(onGone);
    await screen.findByText(/could not put the message back/);

    view.unmount();

    expect(onGone).toHaveBeenCalledTimes(1);
  });

  it("is forgotten when it leaves the tree while its answer is on its way", () => {
    const onGone = vi.fn();
    answerLater();
    const view = raised(onGone);

    view.unmount();

    expect(onGone).toHaveBeenCalledTimes(1);
  });

  it("says nothing of where a message went when the answer comes after it left the tree", async () => {
    // The line is not on the screen any more, so the answer is about none: the list shows the
    // message back, and whatever line stands there now is not told where it went.
    const answer = answerLater();
    const { view, ask, onTakenBack } = noticing(null);
    ask(DELETED, 1);

    view.unmount();
    await answer(true);

    await waitFor(() => expect(onTakenBack).toHaveBeenCalledWith("7", false));
    expect(onTakenBack).not.toHaveBeenCalledWith("7", true);
  });

  it("is forgotten when it leaves the tree saying emptying the trash cannot be undone", () => {
    const onGone = vi.fn();
    const view = render(
      <I18nProvider initial="en">
        <Gone filed="forGood" asked={0} onGone={onGone} />
      </I18nProvider>,
    );
    expect(
      screen.getByText("Emptying the trash cannot be undone"),
    ).toBeTruthy();

    view.unmount();

    expect(onGone).toHaveBeenCalledTimes(1);
  });

  it("comes back with the list when it leaves the tree with its offer unanswered", () => {
    // It still means what it says, so it is not forgotten.
    const onGone = vi.fn();
    const view = render(
      <I18nProvider initial="en">
        <Gone filed={ARCHIVED} asked={0} onGone={onGone} />
      </I18nProvider>,
    );
    expect(screen.getByText("Moved to the archive")).toBeTruthy();

    view.unmount();

    expect(onGone).not.toHaveBeenCalled();
  });

  it("says a message the key raised cannot be brought back, when it cannot", async () => {
    undoing.mockResolvedValue(false);
    const { ask, onTakenBack } = noticing(null);

    ask(DELETED, 1);

    expect(
      await screen.findByText("Lyonoyl Mail could not put the message back"),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
    expect(onTakenBack).not.toHaveBeenCalled();
  });

  it("draws the offer that still stands when asking for a message the key raised failed", async () => {
    undoing.mockRejectedValue(new Error("no"));
    const { ask } = noticing(null);

    ask(DELETED, 1);

    expect(await screen.findByText("Moved to the trash")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Undo" })).toBeTruthy();
  });

  it("says at once that emptying the trash cannot be undone, and asks nobody", () => {
    const { ask } = noticing(null);

    ask("forGood", 1);

    expect(
      screen.getByText("Emptying the trash cannot be undone"),
    ).toBeTruthy();
    expect(
      screen.queryByText("Lyonoyl Mail could not put the message back"),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
    expect(undoing).not.toHaveBeenCalled();
  });

  it("asks once while the answer is on its way, however often the key and the button are pressed", async () => {
    // A second way back asked for meanwhile would be a second instruction to move a message the
    // first is already bringing home.
    const answer = answerLater();
    const { ask } = noticing(ARCHIVED);

    ask(ARCHIVED, 1);
    ask(ARCHIVED, 2);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    expect(undoing).toHaveBeenCalledTimes(1);
    await answer(true);
  });

  it("lets another line's button ask while an answer about an earlier one has not come", async () => {
    undoing.mockImplementation(() => new Promise<boolean>(() => {}));
    const { ask } = noticing(null);
    ask(DELETED, 1);

    ask(ARCHIVED, 1);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));

    expect(undoing).toHaveBeenCalledTimes(2);
    expect(undoing).toHaveBeenLastCalledWith("42", "1");
  });

  it("asks nothing again once it has said a message cannot be brought back", async () => {
    undoing.mockResolvedValue(false);
    const { ask } = noticing(ARCHIVED);
    ask(ARCHIVED, 1);
    await screen.findByText("Lyonoyl Mail could not put the message back");

    ask(ARCHIVED, 2);

    expect(undoing).toHaveBeenCalledTimes(1);
    expect(
      screen.getByText("Lyonoyl Mail could not put the message back"),
    ).toBeTruthy();
  });

  it("asks for a message the key raised over a line that had already said its last", async () => {
    undoing.mockResolvedValue(false);
    const { ask } = noticing(ARCHIVED);
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await screen.findByText("Lyonoyl Mail could not put the message back");

    ask(DELETED, 1);

    await waitFor(() => expect(undoing).toHaveBeenCalledWith("7", "1"));
  });

  it("asks nothing for a press made before it came", () => {
    // Drawn again by a list that came back from behind a screen, it is not a press.
    noticing(ARCHIVED, 3);

    expect(undoing).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Undo" })).toBeTruthy();
  });

  it("does not draw a late answer over the line that has taken its place", async () => {
    const answer = answerLater();
    const { ask } = noticing(null);
    ask(DELETED, 1);

    ask(ARCHIVED, 1);
    await answer(false);

    expect(screen.getByText("Moved to the archive")).toBeTruthy();
    expect(
      screen.queryByText("Lyonoyl Mail could not put the message back"),
    ).toBeNull();
  });

  it("keeps the whole of its time for the answer, however long the core took", async () => {
    vi.useFakeTimers();
    const answer = answerLater();
    const { view, ask } = noticing(null);
    ask(DELETED, 1);

    seconds(3);
    await answer(false);

    expect(
      screen.getByText("Lyonoyl Mail could not put the message back"),
    ).toBeTruthy();
    expect(view.container.querySelector(".tabular-nums")?.textContent).toBe(
      String(NOTICE_MS / 1000),
    );
  });
});
