import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import { MasterPasswordGate } from "./MasterPasswordGate";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    chooseMasterPassword: vi.fn(),
    unlockSecrets: vi.fn(),
  };
});

const { chooseMasterPassword, unlockSecrets } =
  await import("@/shared/ipc/client");
const choose = vi.mocked(chooseMasterPassword);
const unlock = vi.mocked(unlockSecrets);

function show(mode: "choose" | "unlock") {
  const onOpened = vi.fn();
  const onLater = vi.fn();
  render(
    <I18nProvider initial="en">
      <MasterPasswordGate mode={mode} onOpened={onOpened} onLater={onLater} />
    </I18nProvider>,
  );
  return { onOpened, onLater };
}

function type(label: RegExp, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

describe("MasterPasswordGate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    choose.mockResolvedValue(undefined);
    unlock.mockResolvedValue(undefined);
  });

  it("says what will happen before anything is written", () => {
    // The whole point of the screen: nobody may end up believing a password went to a system
    // store when it went into a file.
    show("choose");

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText(/no credential store of its own/i)).toBeTruthy();
    expect(screen.getByText(/no way to recover this password/i)).toBeTruthy();
    // And what a forgotten one costs, which is more than the mailbox passwords: the key the mail
    // is written under is in the same file.
    expect(screen.getByText(/drafts and unsent messages/i)).toBeTruthy();
    expect(choose).not.toHaveBeenCalled();
  });

  it("will not set a password that was typed differently twice", () => {
    show("choose");

    type(/^master password$/i, "correct horse battery");
    type(/^repeat it$/i, "correct horse batteryy");

    expect(screen.getByText(/two entries are different/i)).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: /set password/i })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("will not set a password short enough to guess", () => {
    // There is no server to slow an attacker down: the file is guessed at whatever speed their
    // hardware allows.
    show("choose");

    type(/^master password$/i, "short");
    type(/^repeat it$/i, "short");

    expect(screen.getByText(/at least 12 characters/i)).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: /set password/i })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("sets the password and says the store is open", async () => {
    const { onOpened } = show("choose");

    type(/^master password$/i, "correct horse battery");
    type(/^repeat it$/i, "correct horse battery");
    fireEvent.click(screen.getByRole("button", { name: /set password/i }));

    await waitFor(() => expect(onOpened).toHaveBeenCalled());
    expect(choose).toHaveBeenCalledWith("correct horse battery");
    expect(unlock).not.toHaveBeenCalled();
  });

  it("asks for one field when there is already a file", async () => {
    const { onOpened } = show("unlock");

    expect(screen.queryByLabelText(/^repeat it$/i)).toBeNull();
    type(/^master password$/i, "correct horse battery");
    fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));

    await waitFor(() => expect(onOpened).toHaveBeenCalled());
    expect(unlock).toHaveBeenCalledWith("correct horse battery");
  });

  it("says the password did not open it, and stays where it is", async () => {
    const rejected: CommandError = {
      code: "masterPasswordRejected",
      message: "core said so",
      path: null,
    };
    unlock.mockRejectedValue(rejected);
    const { onOpened } = show("unlock");

    type(/^master password$/i, "not it at all");
    fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toMatch(
        /master password did not open the file/i,
      ),
    );
    expect(onOpened).not.toHaveBeenCalled();
  });

  it("can be put off, without setting anything", () => {
    // Nothing behind it opens until it is given, and putting it off sets nothing.
    const { onLater, onOpened } = show("unlock");

    fireEvent.click(screen.getByRole("button", { name: /not now/i }));

    expect(onLater).toHaveBeenCalled();
    expect(onOpened).not.toHaveBeenCalled();
    expect(unlock).not.toHaveBeenCalled();
  });
});

describe("MasterPasswordGate, getting out of it without a mouse", () => {
  beforeEach(() => {
    choose.mockReset();
    choose.mockResolvedValue(undefined);
    unlock.mockReset();
    unlock.mockResolvedValue(undefined);
  });

  it("leaves on Escape, the same key on every system", async () => {
    // This panel covers the whole window and appears unasked, on the way to mail somebody was
    // already reading. One button out of it, reachable by mouse, is a room with a door somebody
    // has to see to use.
    const { onLater } = show("unlock");

    fireEvent.keyDown(screen.getByRole("dialog"), {
      key: "Escape",
      bubbles: true,
    });

    await waitFor(() => expect(onLater).toHaveBeenCalled());
  });

  it("does not leave while it is handing the password over", async () => {
    // Leaving mid-submit drops a password on its way to the credential store and says nothing
    // about what became of it.
    let release = () => {};
    unlock.mockImplementation(
      () => new Promise<void>((resolve) => (release = () => resolve())),
    );

    const { onLater } = show("unlock");
    type(/^master password$/i, "a long enough password");
    fireEvent.submit(screen.getByRole("dialog"));

    await waitFor(() => expect(unlock).toHaveBeenCalled());
    fireEvent.keyDown(screen.getByRole("dialog"), {
      key: "Escape",
      bubbles: true,
    });

    expect(onLater).not.toHaveBeenCalled();
    release();
  });
});
