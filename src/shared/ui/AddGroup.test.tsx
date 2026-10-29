import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { AddGroup } from "./AddGroup";

/** Long enough for an answer that has already arrived to be acted on. */
async function aMoment() {
  await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
}

/** A button that opens the panel, as the plus or a row's menu does, so the focus has a way back. */
function Host({
  panel,
}: {
  panel: (close: () => void) => React.ReactNode;
}) {
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

function openFrom(panel: (close: () => void) => React.ReactNode) {
  render(
    <I18nProvider initial="en">
      <Host panel={panel} />
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

describe("adding a group", () => {
  function openAdding(onAdd: (name: string) => Promise<boolean>) {
    return openFrom((close) => <AddGroup onAdd={onAdd} onClose={close} />);
  }

  it("starts the typing in its field, and hands the focus back when it goes", async () => {
    const opener = openAdding(vi.fn());

    expect(screen.getByRole("dialog", { name: "Add group" })).toBeTruthy();
    expect(document.activeElement).toBe(field());

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it("asks for the group as typed and closes once it exists", async () => {
    const onAdd = vi.fn().mockResolvedValue(true);
    openAdding(onAdd);

    fireEvent.change(field(), { target: { value: "Work" } });
    send();

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onAdd).toHaveBeenCalledTimes(1);
    expect(onAdd).toHaveBeenCalledWith("Work");
  });

  it("asks nothing for a name that is blank", async () => {
    const onAdd = vi.fn().mockResolvedValue(true);
    openAdding(onAdd);
    const add = screen.getByRole("button", { name: "Add group" }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);

    fireEvent.change(field(), { target: { value: "   " } });
    send();
    await aMoment();

    expect(add.disabled).toBe(true);
    expect(onAdd).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("asks once for two presses that land before the answer", async () => {
    let answer: (made: boolean) => void = () => {};
    const onAdd = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          answer = resolve;
        }),
    );
    openAdding(onAdd);
    fireEvent.change(field(), { target: { value: "Work" } });
    const form = field().closest("form")!;

    // Both inside one turn, before anything is drawn again: only what the press itself sets can
    // stop the second.
    act(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await act(async () => answer(true));

    expect(onAdd).toHaveBeenCalledTimes(1);
  });

  it("keeps the name and says so when the group was not made, and asks again on the next press", async () => {
    const onAdd = vi.fn().mockResolvedValueOnce(false).mockResolvedValue(true);
    openAdding(onAdd);
    fireEvent.change(field(), { target: { value: "Work" } });

    send();

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The group could not be added",
    );
    expect(field().value).toBe("Work");
    expect(screen.getByRole("dialog")).toBeTruthy();

    send();

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onAdd).toHaveBeenCalledTimes(2);
  });

  it("says so as well when the asking itself failed", async () => {
    openAdding(vi.fn().mockRejectedValue(new Error("the bridge is down")));
    fireEvent.change(field(), { target: { value: "Work" } });

    send();

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The group could not be added",
    );
  });

  it("leaves on Escape without asking, and keeps the key from the window", async () => {
    const onAdd = vi.fn();
    const opener = openAdding(onAdd);
    fireEvent.change(field(), { target: { value: "Work" } });

    // The window listens on `window`, where Escape closes the screen in front.
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    fireEvent.keyDown(field(), { key: "Escape", bubbles: true });
    window.removeEventListener("keydown", behind);

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(behind).not.toHaveBeenCalled();
    expect(onAdd).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(opener);
  });

  it("keeps every key from what it was opened over", async () => {
    // A row or a list underneath moves on an arrow or a letter. Typed into the panel, a name
    // would also be walking the sidebar.
    const underneath = vi.fn();
    render(
      <I18nProvider initial="en">
        <div onKeyDown={underneath}>
          <AddGroup onAdd={vi.fn()} onClose={vi.fn()} />
        </div>
      </I18nProvider>,
    );

    fireEvent.keyDown(field(), { key: "ArrowDown", bubbles: true });
    fireEvent.keyDown(field(), { key: "w", bubbles: true });

    expect(underneath).not.toHaveBeenCalled();
  });

  it("is drawn over the whole window, not inside what asked for it", () => {
    render(
      <I18nProvider initial="en">
        <div data-testid="asked-from">
          <AddGroup onAdd={vi.fn()} onClose={vi.fn()} />
        </div>
      </I18nProvider>,
    );

    expect(screen.getByRole("dialog").closest("[data-testid='asked-from']")).toBeNull();
  });

  it("types into the search box's kind of field, so the cross is the one the search box has", () => {
    render(
      <I18nProvider initial="en">
        <AddGroup onAdd={vi.fn()} onClose={vi.fn()} />
      </I18nProvider>,
    );

    // The cross itself is the engine's, drawn for a search field and not drawn by a test document;
    // what can be held here is that the field is one it draws the cross for.
    expect(field().getAttribute("type")).toBe("search");
  });
});
