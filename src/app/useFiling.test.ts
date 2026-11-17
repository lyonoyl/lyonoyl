import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Filed } from "@/features/message-list/FilingNotice";

import { useFiling } from "./useFiling";

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
});
