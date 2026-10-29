import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { RemoveGroup } from "./RemoveGroup";

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

describe("removing a group", () => {
  function openRemoving(
    name: string,
    onRemove: () => Promise<boolean>,
    mailboxes = 0,
  ) {
    return openFrom((close) => (
      <RemoveGroup
        name={name}
        mailboxes={mailboxes}
        onRemove={onRemove}
        onClose={close}
      />
    ));
  }

  function removeButton() {
    return screen.getByRole("button", { name: "Remove" }) as HTMLButtonElement;
  }

  it("asks only for the name of a group with nothing in it, and sets the name apart", () => {
    openRemoving("Work", vi.fn());

    expect(screen.getByRole("dialog", { name: "Remove group" })).toBeTruthy();
    // Named once, below, where it is typed from: not in the title as well.
    expect(screen.getByRole("heading").textContent).toBe("Remove group");
    expect(screen.getByText("To remove the group, type its name")).toBeTruthy();
    // The name on a line of its own, not inside the sentence and not as the field's grey hint.
    expect(screen.getByText("Work").tagName).toBe("P");
    expect(field().placeholder).toBe("");
    expect(
      screen.queryByText("The mailboxes in this group stay in the sidebar where the group was, with all their mail"),
    ).toBeNull();
    expect(document.activeElement).toBe(field());
    expect(removeButton().disabled).toBe(true);
  });

  it("says where the mailboxes go when the group holds some", () => {
    openRemoving("Work", vi.fn(), 2);

    const said = screen.getByRole("dialog").textContent ?? "";
    expect(said).toContain(
      "The mailboxes in this group stay in the sidebar where the group was, with all their mail",
    );
    expect(said).toContain("To remove the group, type its name");
    // In that order: what happens to them, then what to do.
    expect(said.indexOf("stay in the sidebar")).toBeLessThan(said.indexOf("To remove the group"));
  });

  it("stays shut on a name that is nearly right", async () => {
    const onRemove = vi.fn().mockResolvedValue(true);
    openRemoving("Work", onRemove);

    for (const nearly of ["work", "Wor", "Workk", "W ork"]) {
      fireEvent.change(field(), { target: { value: nearly } });
      expect(removeButton().disabled, nearly).toBe(true);
      send();
    }
    await aMoment();

    expect(onRemove).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("opens for the name as it is shown, whatever the spaces around it or how its letters were composed", () => {
    openRemoving("Café", vi.fn());

    fireEvent.change(field(), { target: { value: "  Café  " } });
    expect(removeButton().disabled).toBe(false);

    // The same word with its accent as a letter of its own, as some keyboards type it.
    fireEvent.change(field(), { target: { value: "Café" } });
    expect(removeButton().disabled).toBe(false);
  });

  it("removes once for the exact name and closes", async () => {
    let answer: (gone: boolean) => void = () => {};
    const onRemove = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          answer = resolve;
        }),
    );
    openRemoving("Work", onRemove);
    fireEvent.change(field(), { target: { value: "Work" } });
    const form = field().closest("form")!;

    act(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await act(async () => answer(true));

    expect(onRemove).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("says so and stays when the group was not removed", async () => {
    openRemoving("Work", vi.fn().mockResolvedValue(false));
    fireEvent.change(field(), { target: { value: "Work" } });

    send();

    expect((await screen.findByRole("alert")).textContent).toBe(
      "The group could not be removed",
    );
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(removeButton().disabled).toBe(false);
  });

  it("leaves by Cancel or Escape without removing, back where it was opened", () => {
    const onRemove = vi.fn();
    const opener = openRemoving("Work", onRemove);
    fireEvent.change(field(), { target: { value: "Work" } });

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);

    fireEvent.click(opener);
    fireEvent.keyDown(field(), { key: "Escape", bubbles: true });

    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onRemove).not.toHaveBeenCalled();
  });

  it("types into the search box's kind of field, so the cross is the one the search box has", () => {
    openRemoving("Work", vi.fn());

    // The cross itself is the engine's, drawn for a search field and not drawn by a test document;
    // what can be held here is that the field is one it draws the cross for.
    expect(field().getAttribute("type")).toBe("search");
  });
});
