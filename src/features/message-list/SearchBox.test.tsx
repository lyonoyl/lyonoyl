import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { SearchBox } from "./SearchBox";

function show(onLookFor: (typed: string) => void, autoFocusKey = "inbox") {
  return render(
    <I18nProvider>
      <SearchBox onLookFor={onLookFor} autoFocusKey={autoFocusKey} />
    </I18nProvider>,
  );
}

function box(): HTMLInputElement {
  return screen.getByRole("searchbox") as HTMLInputElement;
}

function type(what: string) {
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: what } });
}

describe("the search box", () => {
  it("runs the window's way while empty, and the letters' way once there are some", () => {
    show(() => {});

    expect(box().getAttribute("dir")).toBeNull();
    type("Projects 2026");
    expect(box().getAttribute("dir")).toBe("auto");
  });

  it("reports what was typed once the typing settles", async () => {
    const asked = vi.fn();
    show(asked);

    type("quarterly");

    await waitFor(() => expect(asked).toHaveBeenCalledWith("quarterly"));
  });

  it("asks once for a word typed at speed rather than once per letter", async () => {
    // Every keystroke reaching the list is a question the index answers and a set of rows the
    // interface throws away, for an answer nobody sees.
    const asked = vi.fn();
    show(asked);

    for (const so_far of ["q", "qu", "qua", "quar"]) {
      type(so_far);
    }

    await waitFor(() => expect(asked).toHaveBeenCalledWith("quar"));
    expect(asked.mock.calls.filter(([typed]) => typed !== "")).toEqual([
      ["quar"],
    ]);
  });

  it("says nothing is being looked for once the box is empty again", async () => {
    const asked = vi.fn();
    show(asked);

    type("quarterly");
    await waitFor(() => expect(asked).toHaveBeenCalledWith("quarterly"));

    fireEvent.click(screen.getByRole("button"));

    await waitFor(() => expect(asked).toHaveBeenLastCalledWith(""));
  });

  it("is emptied by escape", async () => {
    // What somebody who has just searched expects the key to do, before it does anything else in
    // the window.
    const asked = vi.fn();
    show(asked);

    type("quarterly");
    await waitFor(() => expect(asked).toHaveBeenCalledWith("quarterly"));

    fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Escape" });

    expect(box().value).toBe("");
    await waitFor(() => expect(asked).toHaveBeenLastCalledWith(""));
  });

  it("does not carry what was looked for into another mailbox", async () => {
    // A different mailbox is a different question. Carrying the old one into it shows somebody a
    // list they did not ask for, under a folder they just picked.
    const asked = vi.fn();
    const { rerender } = show(asked, "inbox");

    type("quarterly");
    await waitFor(() => expect(asked).toHaveBeenCalledWith("quarterly"));

    rerender(
      <I18nProvider>
        <SearchBox onLookFor={asked} autoFocusKey="archive" />
      </I18nProvider>,
    );

    expect(box().value).toBe("");
    await waitFor(() => expect(asked).toHaveBeenLastCalledWith(""));
  });

  it("offers no way to clear a box that is already empty", () => {
    show(vi.fn());

    expect(screen.queryByRole("button")).toBeNull();
  });
});
