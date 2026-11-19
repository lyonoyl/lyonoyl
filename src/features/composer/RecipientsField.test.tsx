import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SuggestedRecipientDto } from "@/shared/ipc/generated/SuggestedRecipientDto";

import { RecipientsField } from "./RecipientsField";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return { ...actual, suggestRecipients: vi.fn() };
});

const { suggestRecipients } = await import("@/shared/ipc/client");
const offering = vi.mocked(suggestRecipients);

const KAPPA: SuggestedRecipientDto = {
  address: "kappa@example.net",
  name: "Kappa Vandel",
};
const KAPPA_TWO: SuggestedRecipientDto = {
  address: "kappa.two@example.net",
  name: null,
};

beforeEach(() => {
  offering.mockReset();
  offering.mockResolvedValue([]);
});

afterEach(() => {
  vi.restoreAllMocks();
});

/** The field as a form holds it: its label beside it, and the line kept where the form keeps it. */
function Line({ accountId, start }: { accountId: string; start: string }) {
  const [value, setValue] = useState(start);
  return (
    <form>
      <label id="to-label" htmlFor="to">
        To
      </label>
      <RecipientsField
        id="to"
        labelId="to-label"
        accountId={accountId}
        value={value}
        onChange={setValue}
      />
    </form>
  );
}

function shown({ accountId = "acc-1", start = "" } = {}) {
  const { rerender } = render(<Line accountId={accountId} start={start} />);
  const field = screen.getByRole("combobox", { name: "To" }) as HTMLInputElement;
  field.focus();
  return {
    field,
    fromAnotherAccount: () =>
      rerender(<Line accountId="acc-2" start={start} />),
  };
}

/** Types a line as a person leaves it, with the caret where they left it, and lets the answer in. */
async function typed(field: HTMLInputElement, value: string, caret = value.length) {
  fireEvent.change(field, {
    target: { value, selectionStart: caret, selectionEnd: caret },
  });
  await act(async () => {});
}

/** What was asked with something typed, leaving out the gathering asked for as the line takes the focus. */
function questions(): [string, string][] {
  return offering.mock.calls.filter(([, typed]) => typed !== "");
}

function offeredAddresses(): string[] {
  return screen
    .getAllByRole("option")
    .map((option) => option.querySelector("[dir=ltr]")?.textContent ?? "");
}

describe("the people offered as an address is typed", () => {
  it("are asked for with the address being typed, from the account the message goes from", async () => {
    const { field } = shown();

    await typed(field, "zeta@example.com, kap");

    expect(questions()).toEqual([["acc-1", "kap"]]);
  });

  it("are gathered ahead as the line takes the focus, and nobody is drawn for it", async () => {
    offering.mockResolvedValue([KAPPA]);
    shown();
    await act(async () => {});

    expect(offering).toHaveBeenCalledWith("acc-1", "");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("combobox", { name: "To" })).toBeTruthy();
  });

  it("are gathered ahead quietly when the core cannot answer", async () => {
    offering.mockRejectedValue({ code: "storageUnavailable", message: "x" });
    shown();
    await act(async () => {});

    expect(offering).toHaveBeenCalledWith("acc-1", "");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("combobox", { name: "To" })).toBeTruthy();
  });

  it("are listed by name and address, under the field's own label", async () => {
    offering.mockResolvedValue([KAPPA, KAPPA_TWO]);
    const { field } = shown();

    await typed(field, "kap");

    const list = screen.getByRole("listbox", { name: "To" });
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(2);
    expect(options[0].textContent).toBe("Kappa Vandelkappa@example.net");
    expect(options[1].textContent).toBe("kappa.two@example.net");
    expect(options[0].querySelector("bdi")?.textContent).toBe("Kappa Vandel");
    expect(field.getAttribute("aria-expanded")).toBe("true");
    expect(field.getAttribute("aria-controls")).toBe(list.id);
  });

  it("start with the first chosen, which Enter takes, leaving room for the next", async () => {
    offering.mockResolvedValue([KAPPA, KAPPA_TWO]);
    const { field } = shown();
    await typed(field, "zeta@example.com, kap");

    expect(field.getAttribute("aria-activedescendant")).toBe(
      screen.getAllByRole("option")[0].id,
    );
    expect(screen.getAllByRole("option")[0].getAttribute("aria-selected")).toBe(
      "true",
    );
    const leftToTheForm = fireEvent.keyDown(field, { key: "Enter" });

    expect(leftToTheForm).toBe(false);
    expect(field.value).toBe("zeta@example.com, kappa@example.net, ");
    expect(field.selectionStart).toBe(field.value.length);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(field.getAttribute("aria-expanded")).toBe("false");
  });

  it("are walked with the arrows, and Tab takes the one chosen", async () => {
    offering.mockResolvedValue([KAPPA, KAPPA_TWO]);
    const { field } = shown();
    await typed(field, "kap");

    fireEvent.keyDown(field, { key: "ArrowDown" });
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(field.getAttribute("aria-activedescendant")).toBe(
      screen.getAllByRole("option")[1].id,
    );
    const leftToTheWindow = fireEvent.keyDown(field, { key: "Tab" });

    expect(leftToTheWindow).toBe(false);
    expect(field.value).toBe("kappa.two@example.net, ");
    expect(document.activeElement).toBe(field);
  });

  it("are left with nobody chosen above the first, and then Enter and Tab are the form's", async () => {
    offering.mockResolvedValue([KAPPA, KAPPA_TWO]);
    const { field } = shown();
    await typed(field, "kap");

    fireEvent.keyDown(field, { key: "ArrowUp" });

    expect(field.hasAttribute("aria-activedescendant")).toBe(false);
    expect(fireEvent.keyDown(field, { key: "Enter" })).toBe(true);
    expect(fireEvent.keyDown(field, { key: "Tab" })).toBe(true);
    expect(field.value).toBe("kap");
    expect(screen.getByRole("listbox")).toBeTruthy();
  });

  it("take the one clicked, and the field keeps the focus", async () => {
    offering.mockResolvedValue([KAPPA, KAPPA_TWO]);
    const { field } = shown();
    await typed(field, "kap");
    const second = screen.getAllByRole("option")[1];

    expect(fireEvent.mouseDown(second)).toBe(false);
    fireEvent.click(second);

    expect(field.value).toBe("kappa.two@example.net, ");
    expect(document.activeElement).toBe(field);
  });

  it("put the one chosen in place of the address the caret stands in, not the last one", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap, sigma@example.org", 3);

    expect(offering).toHaveBeenCalledWith("acc-1", "kap");
    fireEvent.keyDown(field, { key: "Enter" });

    expect(field.value).toBe("kappa@example.net, sigma@example.org");
    expect(field.selectionStart).toBe("kappa@example.net".length);
  });

  it("close on Escape without the key reaching the window, and the next Escape reaches it", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");
    const window_ = vi.fn();
    window.addEventListener("keydown", window_);

    try {
      fireEvent.keyDown(field, { key: "Escape" });
      expect(screen.queryByRole("listbox")).toBeNull();
      expect(field.value).toBe("kap");
      expect(window_).not.toHaveBeenCalled();

      fireEvent.keyDown(field, { key: "Escape" });
      expect(window_).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener("keydown", window_);
    }
  });

  it("are never drawn from an answer to an older question", async () => {
    const { field } = shown();
    await act(async () => {});
    let answerTheFirst: (people: SuggestedRecipientDto[]) => void = () => {};
    offering
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            answerTheFirst = resolve;
          }),
      )
      .mockResolvedValueOnce([KAPPA_TWO]);

    await typed(field, "k");
    await typed(field, "kappa.");
    await act(async () => answerTheFirst([KAPPA]));

    expect(offeredAddresses()).toEqual(["kappa.two@example.net"]);
  });

  it("are not asked for while no address is being typed, and what was offered goes", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");
    expect(screen.getByRole("listbox")).toBeTruthy();

    await typed(field, "kap,  ");

    expect(questions()).toHaveLength(1);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(field.value).toBe("kap,  ");
  });

  it("leave a whole address as it was typed unless somebody is chosen by hand", async () => {
    offering.mockResolvedValue([
      { address: "kappa@example.net.example", name: null },
    ]);
    const { field } = shown();
    await typed(field, "kappa@example.net");

    expect(screen.getByRole("listbox")).toBeTruthy();
    expect(field.hasAttribute("aria-activedescendant")).toBe(false);
    expect(fireEvent.keyDown(field, { key: "Tab" })).toBe(true);
    expect(fireEvent.keyDown(field, { key: "Enter" })).toBe(true);
    expect(field.value).toBe("kappa@example.net");

    fireEvent.keyDown(field, { key: "ArrowDown" });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(field.value).toBe("kappa@example.net.example, ");
  });

  it("show nothing when the question could not be answered", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");
    expect(screen.getByRole("listbox")).toBeTruthy();
    offering.mockRejectedValue({ code: "storageUnavailable", message: "x" });

    // Another line than the last: the same one again is no change to the field.
    await typed(field, "kapp");

    expect(offering).toHaveBeenLastCalledWith("acc-1", "kapp");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByDisplayValue("kapp")).toBe(field);
    expect(field.getAttribute("aria-expanded")).toBe("false");
  });

  it("close when the field is left", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");

    fireEvent.blur(field);

    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByDisplayValue("kap")).toBe(field);
  });

  it("close when the caret goes to another address, and the key still moves it", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");

    expect(fireEvent.keyDown(field, { key: "ArrowLeft" })).toBe(true);

    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("close when the message is to go from another account", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field, fromAnotherAccount } = shown();
    await typed(field, "kap");

    fromAnotherAccount();

    expect(screen.queryByRole("listbox")).toBeNull();
    await typed(field, "kapp");
    expect(offering).toHaveBeenLastCalledWith("acc-2", "kapp");
  });

  it("leave an input method's keys to it", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");

    expect(fireEvent.keyDown(field, { key: "Enter", isComposing: true })).toBe(
      true,
    );
    expect(fireEvent.keyDown(field, { key: "Enter", keyCode: 229 })).toBe(true);
    expect(field.value).toBe("kap");
    expect(screen.getByRole("listbox")).toBeTruthy();
  });

  it("leave a chord to whatever it is for", async () => {
    offering.mockResolvedValue([KAPPA]);
    const { field } = shown();
    await typed(field, "kap");

    expect(fireEvent.keyDown(field, { key: "Enter", metaKey: true })).toBe(true);
    expect(field.value).toBe("kap");
  });
});
