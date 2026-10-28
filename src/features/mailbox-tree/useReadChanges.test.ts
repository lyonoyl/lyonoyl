import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useReadChanges } from "./useReadChanges";

describe("the read changes laid over the counts", () => {
  it("holds a change from the moment it is asked for, with when that was", () => {
    const { result } = renderHook(() => useReadChanges());
    const before = performance.now();

    act(() => result.current[1]("1", true, new Promise(() => {})));

    expect(result.current[0]).toMatchObject([
      { folderId: "1", unread: -1, asked: null },
    ]);
    expect(result.current[0][0].askedAt).toBeGreaterThanOrEqual(before);
  });

  it("takes where the write stands from the core's answer", async () => {
    // What tells a read of the counts that holds the change from one that does not.
    const write = Promise.resolve(7);
    const { result } = renderHook(() => useReadChanges());

    await act(async () => {
      result.current[1]("1", true, write);
      await write;
    });

    expect(result.current[0]).toMatchObject([{ folderId: "1", asked: 7 }]);
  });

  it("takes a change away when the write that says so fails", async () => {
    // The dot goes back on the row, and the number beside the folder has to go back with it.
    const write = Promise.reject(new Error("the database would not take it"));
    const { result } = renderHook(() => useReadChanges());

    await act(async () => {
      result.current[1]("1", false, write);
      await write.catch(() => {});
    });

    expect(result.current[0]).toEqual([]);
  });
});
