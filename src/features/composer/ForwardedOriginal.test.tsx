import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { LinkAsked } from "@/shared/ipc/generated/LinkAsked";
import type { MessageBodyDto } from "@/shared/ipc/generated/MessageBodyDto";
import { ForwardedOriginal } from "./ForwardedOriginal";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    openOriginalBody: vi.fn(),
    closeMessageBody: vi.fn(),
    onMessageLink: vi.fn(),
  };
});

const { openOriginalBody, closeMessageBody, onMessageLink } =
  await import("@/shared/ipc/client");
const drawing = vi.mocked(openOriginalBody);
const closing = vi.mocked(closeMessageBody);
const listening = vi.mocked(onMessageLink);

/** The handler the frame registered, so a test can play a link being followed. */
let followed: ((link: LinkAsked) => void) | null = null;

function body(over: Partial<MessageBodyDto> = {}): MessageBodyDto {
  return {
    name: "b-1",
    url: "lyomsg://localhost/b-1",
    kind: "html",
    remoteBlocked: 0,
    remoteNotShown: 0,
    remoteNotHad: 0,
    truncated: false,
    attachments: [],
    ...over,
  };
}

function aLink(over: Partial<LinkAsked> = {}): LinkAsked {
  return {
    name: "l-1",
    host: "example.com",
    hostAsWritten: null,
    url: "https://example.com/a",
    ...over,
  };
}

function show(token = "t-original") {
  return render(
    <I18nProvider initial="en">
      <ForwardedOriginal token={token} />
    </I18nProvider>,
  );
}

beforeEach(() => {
  drawing.mockReset();
  drawing.mockResolvedValue(body());
  closing.mockReset();
  closing.mockResolvedValue(undefined);
  listening.mockReset();
  followed = null;
  listening.mockImplementation(async (handler) => {
    followed = handler;
    return () => {
      followed = null;
    };
  });
});

describe("the message passed on in its own form", () => {
  it("is drawn from the token it is held under, in a frame that runs nothing and sends no referrer", async () => {
    show();

    const frame = (await screen.findByTitle(
      "Forwarded message",
    )) as HTMLIFrameElement;
    await waitFor(() =>
      expect(frame.getAttribute("src")).toBe("lyomsg://localhost/b-1"),
    );
    expect(drawing.mock.lastCall?.[0]).toBe("t-original");
    expect(frame.getAttribute("sandbox")).toBe("");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("draws nothing when it cannot be drawn", async () => {
    // Refused, or drawn as nothing: the message still goes whole beneath what is written, and a
    // frame showing nothing would only say something is wrong with it.
    for (const answer of [
      () =>
        drawing.mockRejectedValue({
          code: "invalidRequest",
          message: "",
          path: null,
        }),
      () => drawing.mockResolvedValue(body({ kind: "nothing" })),
    ]) {
      drawing.mockReset();
      answer();
      const { unmount } = show();
      await waitFor(() => expect(drawing).toHaveBeenCalled());
      await act(async () => {});

      expect(screen.queryByTitle("Forwarded message")).toBeNull();
      expect(screen.queryByRole("alert")).toBeNull();
      unmount();
    }
  });

  it("asks about a link followed in it", async () => {
    show();
    await waitFor(() =>
      expect(screen.getByTitle("Forwarded message").getAttribute("src")).toBe(
        "lyomsg://localhost/b-1",
      ),
    );
    await waitFor(() => expect(followed).not.toBeNull());

    await act(async () => followed?.(aLink()));
    expect((await screen.findByRole("dialog")).textContent).toMatch(
      /example\.com/,
    );
  });

  it("lets go of the frame's body once it is gone", async () => {
    const { unmount } = show();
    await waitFor(() =>
      expect(screen.getByTitle("Forwarded message").getAttribute("src")).toBe(
        "lyomsg://localhost/b-1",
      ),
    );

    unmount();

    await waitFor(() => expect(closing).toHaveBeenCalledWith("b-1"));
  });

  it("lets go of a body that arrives after it has gone", async () => {
    let answer: (opened: MessageBodyDto) => void = () => {};
    drawing.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const { unmount } = show();
    await waitFor(() => expect(drawing).toHaveBeenCalled());

    unmount();
    await act(async () => answer(body({ name: "b-late" })));

    await waitFor(() => expect(closing).toHaveBeenCalledWith("b-late"));
  });
});
