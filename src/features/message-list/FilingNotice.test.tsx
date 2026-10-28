import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, useI18n } from "@/shared/i18n";
import { FilingNotice, NOTICE_MS, type Filed } from "./FilingNotice";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, undoAction: vi.fn(), markMessageRead: vi.fn() };
});

const { undoAction, markMessageRead } = await import("@/shared/ipc/client");
const undoing = vi.mocked(undoAction);
const marking = vi.mocked(markMessageRead);

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

    await waitFor(() => expect(onTakenBack).toHaveBeenCalledWith("42"));
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

    expect(await screen.findByText("That cannot be brought back")).toBeTruthy();
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
