import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider, useI18n } from "@/shared/i18n";

import { EmptyTrash } from "./EmptyTrash";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, emptyFolder: vi.fn() };
});

const { emptyFolder } = await import("@/shared/ipc/client");
const emptying = vi.mocked(emptyFolder);

function Harness({
  holds,
  onEmptied,
}: {
  holds: number;
  onEmptied: () => void;
}) {
  const { t } = useI18n();
  return <EmptyTrash folderId="7" holds={holds} onEmptied={onEmptied} t={t} />;
}

function show(holds: number, onEmptied = vi.fn()) {
  render(
    <I18nProvider initial="en">
      <Harness holds={holds} onEmptied={onEmptied} />
    </I18nProvider>,
  );
  return { onEmptied };
}

beforeEach(() => {
  emptying.mockReset();
  emptying.mockResolvedValue(3);
});

describe("emptying the trash", () => {
  it("says nothing at all about a trash with nothing in it", () => {
    show(0);

    expect(screen.queryByRole("button", { name: /empty/i })).toBeNull();
  });

  it("asks before it does it, and says how many", () => {
    // There is no way back from this: what is queued is one permanent deletion per message, and
    // the trash is exactly where the way back used to be.
    show(3);

    expect(screen.getByText("The trash holds 3 messages")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Empty the trash" }));

    expect(
      screen.getByText("Delete 3 messages for good? There is no way back"),
    ).toBeTruthy();
    expect(
      emptying,
      "asked for before anybody answered",
    ).not.toHaveBeenCalled();
  });

  it("leaves them alone when that is the answer", () => {
    show(3);
    fireEvent.click(screen.getByRole("button", { name: "Empty the trash" }));

    fireEvent.click(screen.getByRole("button", { name: "Keep them" }));

    expect(emptying).not.toHaveBeenCalled();
    expect(screen.getByText("The trash holds 3 messages")).toBeTruthy();
  });

  it("empties it once, and tells the list to read itself again", async () => {
    const { onEmptied } = show(3);
    fireEvent.click(screen.getByRole("button", { name: "Empty the trash" }));

    fireEvent.click(
      screen.getByRole("button", { name: "Delete them for good" }),
    );

    await waitFor(() => expect(emptying).toHaveBeenCalledWith("7"));
    await waitFor(() => expect(onEmptied).toHaveBeenCalled());
  });

  it("stays where it was when nothing could be queued", async () => {
    // Nothing was asked for and nothing changes on screen, which is the honest answer to "it did
    // not happen" -- and the offer is still there to press again.
    emptying.mockRejectedValue({ code: "internal", message: "", path: null });
    const { onEmptied } = show(3);
    fireEvent.click(screen.getByRole("button", { name: "Empty the trash" }));

    fireEvent.click(
      screen.getByRole("button", { name: "Delete them for good" }),
    );

    await waitFor(() => expect(emptying).toHaveBeenCalled());
    expect(onEmptied).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Delete them for good" }),
    ).toBeTruthy();
  });
});
