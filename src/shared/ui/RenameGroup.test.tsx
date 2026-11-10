import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { RenameGroup } from "./RenameGroup";

/** Long enough for an answer that has already arrived to be acted on. */
async function aMoment() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
}

/** A button that opens the panel, as a group's menu does, so the focus has a way back. */
function Host({ panel }: { panel: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Opener
      </button>
      {open && panel(() => setOpen(false))}
    </>
  );
}

function openRenaming(name: string, onRename: (name: string) => Promise<boolean>) {
  render(
    <I18nProvider initial="en">
      <Host
        panel={(close) => (
          <RenameGroup name={name} onRename={onRename} onClose={close} />
        )}
      />
    </I18nProvider>,
  );
  const opener = screen.getByRole("button", { name: "Opener" });
  opener.focus();
  fireEvent.click(opener);
  return opener;
}

function field() {
  return screen.getByRole("textbox", { name: "Group name" }) as HTMLInputElement;
}

function send() {
  fireEvent.submit(field().closest("form")!);
}

describe("renaming a group", () => {
  it("starts from the name the group has, with the typing in its field", () => {
    openRenaming("Work", vi.fn());

    expect(screen.getByRole("dialog", { name: "Rename group" })).toBeTruthy();
    expect(field().value).toBe("Work");
    expect(document.activeElement).toBe(field());
    // Selected whole, so the new name is typed over the old one rather than after it.
    expect([field().selectionStart, field().selectionEnd]).toEqual([0, 4]);
  });

  it("asks for the new name and closes once the group carries it", async () => {
    const onRename = vi.fn().mockResolvedValue(true);
    openRenaming("Work", onRename);

    fireEvent.change(field(), { target: { value: "Office" } });
    send();

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onRename).toHaveBeenCalledTimes(1);
    expect(onRename).toHaveBeenCalledWith("Office");
  });

  it("closes without asking for a name left as it was", async () => {
    // Nothing to keep: asking the core to write the name it already holds is a round trip that
    // can only fail.
    const onRename = vi.fn().mockResolvedValue(true);
    openRenaming("Work", onRename);

    fireEvent.change(field(), { target: { value: "  Work  " } });
    send();

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onRename).not.toHaveBeenCalled();
  });

  it("asks nothing for a name that is blank", async () => {
    const onRename = vi.fn().mockResolvedValue(true);
    openRenaming("Work", onRename);

    fireEvent.change(field(), { target: { value: "   " } });
    send();
    await aMoment();

    const rename = screen.getByRole("button", { name: "Rename" }) as HTMLButtonElement;
    expect(rename.disabled).toBe(true);
    expect(onRename).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("asks once for two presses that land before the answer", async () => {
    let answer: (renamed: boolean) => void = () => {};
    const onRename = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          answer = resolve;
        }),
    );
    openRenaming("Work", onRename);
    fireEvent.change(field(), { target: { value: "Office" } });
    const form = field().closest("form")!;

    act(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await act(async () => answer(true));

    expect(onRename).toHaveBeenCalledTimes(1);
  });

  it("keeps the new name and says so when the group was not renamed", async () => {
    const onRename = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true);
    openRenaming("Work", onRename);
    fireEvent.change(field(), { target: { value: "Office" } });

    send();

    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "The group could not be renamed",
    );
    expect(field().value).toBe("Office");

    send();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onRename).toHaveBeenCalledTimes(2);
  });

  it("leaves on Escape without asking, and hands the focus back", async () => {
    const onRename = vi.fn().mockResolvedValue(true);
    const opener = openRenaming("Work", onRename);

    fireEvent.change(field(), { target: { value: "Office" } });
    fireEvent.keyDown(field(), { key: "Escape" });

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onRename).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(opener);
  });
});
