import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { AddMenu } from "./AddMenu";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, createAccountGroup: vi.fn() };
});

const client = await import("@/shared/ipc/client");
const makeGroup = vi.mocked(client.createAccountGroup);

const PLUS = { name: "Add an account or a group" };

function renderPlus() {
  const onAddAccount = vi.fn();
  const onGroupAdded = vi.fn();
  render(
    <I18nProvider initial="en">
      <AddMenu onAddAccount={onAddAccount} onGroupAdded={onGroupAdded} />
    </I18nProvider>,
  );
  return { onAddAccount, onGroupAdded };
}

/** Opens the plus's menu the way a pointer does. */
function openMenu() {
  fireEvent.pointerDown(screen.getByRole("button", PLUS), {
    button: 0,
    ctrlKey: false,
  });
}

/** Chooses Add group, and hands back the field the panel puts the typing in. */
async function startAGroup() {
  openMenu();
  fireEvent.click(await screen.findByRole("menuitem", { name: "Add group" }));
  return screen.findByRole("textbox", { name: "Group name" });
}

/** Long enough for the menu's own wait before it hands the focus back. */
async function aMoment() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
}

beforeEach(() => {
  makeGroup.mockReset();
  makeGroup.mockResolvedValue("g1");
});

describe("the plus in the sidebar's heading", () => {
  it("offers a mailbox or a group, and does nothing until one is chosen", async () => {
    const { onAddAccount } = renderPlus();

    openMenu();

    const offered = await screen.findAllByRole("menuitem");
    expect(offered.map((item) => item.textContent)).toEqual([
      "Add account",
      "Add group",
    ]);
    expect(onAddAccount).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("connects a mailbox once the menu has gone", async () => {
    const { onAddAccount } = renderPlus();

    openMenu();
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Add account" }),
    );

    // Once the menu has gone, so the screen it opens keeps the focus it takes.
    await waitFor(() => expect(onAddAccount).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(makeGroup).not.toHaveBeenCalled();
  });

  it("names a group in a panel whose field holds the typing, and keeps holding it", async () => {
    renderPlus();

    const field = await startAGroup();

    expect(screen.getByRole("dialog", { name: "Add group" })).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(field));
    // The menu hands the focus back to the plus a moment after it closes. Let through, the first
    // letter typed goes to the plus, and the panel waits with its field empty.
    await aMoment();
    expect(document.activeElement).toBe(field);
  });

  it("makes the group as typed, closes the panel, tells the sidebar, and is back on the plus", async () => {
    const { onGroupAdded } = renderPlus();
    const field = await startAGroup();

    fireEvent.change(field, { target: { value: "Work" } });
    fireEvent.submit(field.closest("form")!);

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onGroupAdded).toHaveBeenCalledTimes(1);
    expect(makeGroup).toHaveBeenCalledWith("Work");
    expect(document.activeElement).toBe(screen.getByRole("button", PLUS));
  });

  it("tells the sidebar nothing when the core refuses the group", async () => {
    makeGroup.mockRejectedValue({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { onGroupAdded } = renderPlus();
    const field = await startAGroup();

    fireEvent.change(field, { target: { value: "Work" } });
    fireEvent.submit(field.closest("form")!);

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(onGroupAdded).not.toHaveBeenCalled();
  });

  it("is back on the plus when the panel is left with Escape", async () => {
    renderPlus();
    const field = await startAGroup();

    fireEvent.keyDown(field, { key: "Escape", bubbles: true });

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(makeGroup).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(screen.getByRole("button", PLUS));
  });
});
