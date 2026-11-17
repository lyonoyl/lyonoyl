import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Filed } from "@/features/message-list/FilingNotice";
import type { PutAwayFromANotice } from "@/shared/ipc/generated/PutAwayFromANotice";

import { useFiling } from "./useFiling";

vi.mock("@/shared/ipc/client", () => ({ onPutAwayFromANotice: vi.fn() }));

const client = await import("@/shared/ipc/client");
const notices = vi.mocked(client.onPutAwayFromANotice);

/** What the core says of a message put away with a notification's button, once it is listened for. */
let heard: (put: PutAwayFromANotice) => void = () => {};
const stopped = vi.fn();

beforeEach(() => {
  stopped.mockReset();
  notices.mockReset();
  notices.mockImplementation(async (handle) => {
    heard = handle;
    return stopped;
  });
});

const DELETED: Filed = { messageId: "7", folderId: "1", what: "deleted" };

describe("useFiling", () => {
  it("gives the corner back once a message is back, unless its line goes on to say where", () => {
    // Back by the notice's button, the notice has nothing left to say, and the corner is free for
    // the offer to empty the trash. Back by the key, the line says where the message went and then
    // goes by itself, as any line does.
    const { result } = renderHook(() => useFiling(vi.fn()));

    act(() => result.current.filedAway(DELETED));
    expect(result.current.notice).toBe(DELETED);
    act(() => result.current.putBack("7"));
    expect(result.current.notice).toBeNull();

    act(() => result.current.filedAway(DELETED));
    act(() => result.current.putBack("7", true));
    expect(result.current.notice).toBe(DELETED);
    act(() => result.current.forgetWhatWasFiled());
    expect(result.current.notice).toBeNull();
  });

  it("leaves the key nothing to take back once a message is back, whichever way it came", () => {
    const { result } = renderHook(() => useFiling(vi.fn()));

    for (const sayingWhere of [false, true]) {
      act(() => result.current.filedAway(DELETED));
      act(() => result.current.putBack("7", sayingWhere));
      const asked = result.current.takeBackAsked;

      act(() => result.current.takeBackByKey());

      expect(result.current.takeBackAsked).toBe(asked);
    }
  });

  it("raises nothing by the key while an answer about a message is on its way", () => {
    // The notice that asked may have left the tree with a screen opened over the list; asked
    // again, the core would be asked a second time about the same message.
    const { result } = renderHook(() => useFiling(vi.fn()));
    act(() => result.current.filedAway(DELETED));
    result.current.inFlight.current = DELETED;
    const asked = result.current.takeBackAsked;

    act(() => result.current.takeBackByKey());
    expect(result.current.takeBackAsked).toBe(asked);

    result.current.inFlight.current = null;
    act(() => result.current.takeBackByKey());
    expect(result.current.takeBackAsked).toBe(asked + 1);
  });

  it("says there is something to take back only while the key would take a message back", () => {
    // What greys the menu's Undo out: a message marked read by being opened is not one, one put
    // back is not one any more, and nothing brings back what emptying the trash deleted.
    const { result } = renderHook(() => useFiling(vi.fn()));
    expect(result.current.canTakeBack).toBe(false);

    act(() =>
      result.current.filedAway({ messageId: "9", folderId: "1", what: "read" }),
    );
    expect(result.current.canTakeBack).toBe(false);

    act(() => result.current.filedAway(DELETED));
    expect(result.current.canTakeBack).toBe(true);
    act(() => result.current.putBack("7"));
    expect(result.current.canTakeBack).toBe(false);

    act(() => result.current.filedAway(DELETED));
    act(() => result.current.trashEmptied());
    expect(result.current.canTakeBack).toBe(false);
  });

  it("still has the newer message to take back when an earlier one comes back late", () => {
    // The notice answers about the message it asked about, which may be one filed before the
    // last: the key still takes the last one back.
    const { result } = renderHook(() => useFiling(vi.fn()));
    act(() => result.current.filedAway(DELETED));
    act(() =>
      result.current.filedAway({
        messageId: "8",
        folderId: "1",
        what: "deleted",
      }),
    );

    act(() => result.current.putBack("7"));

    expect(result.current.canTakeBack).toBe(true);
  });

  it("reads the list again once a message is back", () => {
    const generation = vi.fn();
    const { result } = renderHook(() => useFiling(generation));

    act(() => result.current.filedAway(DELETED));
    act(() => result.current.putBack("7", true));

    expect(generation).toHaveBeenCalledTimes(1);
    expect(result.current.told).toEqual({ messageId: "7", state: "back" });
  });

  it("holds a message put away with a notification's button as one filed here, for the key", async () => {
    const { result } = renderHook(() => useFiling(vi.fn()));
    await act(async () => {});

    act(() => heard({ messageId: "9", folderId: "1", what: "deleted" }));

    const filed: Filed = { messageId: "9", folderId: "1", what: "deleted" };
    expect(result.current.filed).toEqual(filed);
    expect(result.current.notice).toEqual(filed);
    // With its folder: the window may be showing the same message in another, where it has not moved.
    expect(result.current.told).toEqual({
      messageId: "9",
      state: "gone",
      folderId: "1",
    });
    expect(result.current.canTakeBack).toBe(true);

    const asked = result.current.takeBackAsked;
    act(() => result.current.takeBackByKey());
    expect(result.current.takeBackAsked).toBe(asked + 1);
  });

  it("stops listening once the window has gone, and when it went while the listening was set up", async () => {
    const gone = renderHook(() => useFiling(vi.fn()));
    await act(async () => {});
    gone.unmount();
    expect(stopped).toHaveBeenCalledTimes(1);

    // The answer arriving after the window went: let go of at once rather than kept for ever.
    let answer: (stop: () => void) => void = () => {};
    notices.mockImplementation(
      () => new Promise<() => void>((resolve) => (answer = resolve)),
    );
    const early = renderHook(() => useFiling(vi.fn()));
    early.unmount();
    const late = vi.fn();
    await act(async () => answer(late));
    expect(late).toHaveBeenCalledTimes(1);
  });
});
