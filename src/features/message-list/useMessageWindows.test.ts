import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ViewWindowDto } from "@/shared/ipc/generated/ViewWindowDto";
import { useMessageWindows } from "./useMessageWindows";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, getViewWindow: vi.fn() };
});

const { getViewWindow } = await import("@/shared/ipc/client");
const windowFor = vi.mocked(getViewWindow);

function windowOf(label: string, total: number): ViewWindowDto {
  return {
    rows: [
      {
        inConversation: 1,
        conversationId: null,
        messageId: "1",
        folderId: "1",
        stopped: null,
        subject: label,
        sender: null,
        preview: null,
        receivedAt: 1_700_000_000,
        unread: false,
        leaving: false,
        arriving: false,
      },
    ],
    total,
    viewEpoch: "1",
    notAnswering: [],
  };
}

beforeEach(() => {
  windowFor.mockReset();
});

describe("useMessageWindows", () => {
  it("shows the rows of the folder that was asked for", async () => {
    windowFor.mockResolvedValue(windowOf("inbox", 1));
    const { result } = renderHook(() =>
      useMessageWindows({
        scope: { kind: "folder", folderId: "1" },
        unreadOnly: false,
      }),
    );

    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.rowAt(0)?.subject).toBe("inbox");
    expect(result.current.total).toBe(1);
  });

  it("asks again when the words being looked for change", async () => {
    // Every search shares one key, so that typing does not remount the list and take the scroll
    // position with it. The words still have to reach the question: read off that key, the list
    // answered whatever settled first and then held it until the box was emptied.
    windowFor.mockImplementation((request) =>
      Promise.resolve(
        windowOf(
          request.scope.kind === "looking"
            ? request.scope.typed
            : "not a search",
          1,
        ),
      ),
    );

    const { result, rerender } = renderHook(
      ({ typed }: { typed: string }) =>
        useMessageWindows({
          scope: { kind: "looking", typed },
          unreadOnly: false,
        }),
      { initialProps: { typed: "quarter" } },
    );

    await waitFor(() =>
      expect(result.current.rowAt(0)?.subject).toBe("quarter"),
    );

    rerender({ typed: "quarterly" });

    await waitFor(() =>
      expect(result.current.rowAt(0)?.subject).toBe("quarterly"),
    );
    expect(windowFor).toHaveBeenLastCalledWith(
      expect.objectContaining({
        scope: { kind: "looking", typed: "quarterly" },
      }),
    );
  });

  it("discards an answer that arrives after the folder changed", async () => {
    // The first folder answers slowly, the second immediately.
    let releaseSlow: (value: ViewWindowDto) => void = () => {};
    windowFor.mockImplementationOnce(
      () =>
        new Promise<ViewWindowDto>((resolve) => {
          releaseSlow = resolve;
        }),
    );
    windowFor.mockResolvedValue(windowOf("archive", 2));

    const { result, rerender } = renderHook(
      ({ folderId }) =>
        useMessageWindows({
          scope: { kind: "folder", folderId },
          unreadOnly: false,
        }),
      { initialProps: { folderId: "1" } },
    );

    rerender({ folderId: "2" });
    await waitFor(() => expect(result.current.total).toBe(2));

    await act(async () => {
      releaseSlow(windowOf("inbox", 999));
    });

    expect(result.current.rowAt(0)?.subject).toBe("archive");
    expect(result.current.total).toBe(2);
  });

  it("asks once for a page even when the same range is requested repeatedly", async () => {
    windowFor.mockResolvedValue(windowOf("inbox", 500));
    const { result } = renderHook(() =>
      useMessageWindows({
        scope: { kind: "folder", folderId: "1" },
        unreadOnly: false,
      }),
    );
    await waitFor(() => expect(result.current.status).toBe("ready"));
    windowFor.mockClear();

    act(() => {
      result.current.requestRange(0, 40);
      result.current.requestRange(10, 50);
    });

    expect(windowFor).not.toHaveBeenCalled();
  });

  it("reports a failure instead of pretending the folder is empty", async () => {
    windowFor.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { result } = renderHook(() =>
      useMessageWindows({
        scope: { kind: "folder", folderId: "1" },
        unreadOnly: false,
      }),
    );

    await waitFor(() => expect(result.current.status).toBe("failed"));
    expect(result.current.error?.code).toBe("storageUnavailable");
  });
});
