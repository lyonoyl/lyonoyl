import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ConnectionReportDto } from "@/shared/ipc/generated/ConnectionReportDto";
import { ScreenPane } from "@/app/ScreenPane";
import { I18nProvider, LANGUAGES } from "@/shared/i18n";
import { secretStoreName } from "@/shared/platform";
import type { CommandError } from "@/shared/ipc/generated/CommandError";
import { AccountSetup } from "./AccountSetup";

vi.mock("@/shared/ipc/client", async () => {
  const actual = await vi.importActual<typeof import("@/shared/ipc/client")>(
    "@/shared/ipc/client",
  );
  return {
    ...actual,
    addAccount: vi.fn(),
    changeAccount: vi.fn(),
    testAccountConnection: vi.fn(),
    defaultPortFor: vi.fn(),
    removeAccount: vi.fn(),
    getAccountSettings: vi.fn(),
    findServerSettings: vi.fn(),
    accountHint: vi.fn(),
  };
});

const {
  addAccount,
  changeAccount,
  testAccountConnection,
  defaultPortFor,
  removeAccount,
  getAccountSettings,
  findServerSettings,
  accountHint,
} = await import("@/shared/ipc/client");
const add = vi.mocked(addAccount);
const change = vi.mocked(changeAccount);
const check = vi.mocked(testAccountConnection);
const port = vi.mocked(defaultPortFor);
const remove = vi.mocked(removeAccount);
const settingsOf = vi.mocked(getAccountSettings);
const looking = vi.mocked(findServerSettings);
const hinting = vi.mocked(accountHint);

const ACCOUNT = {
  id: "acc-1",
  email: "user@example.com",
  displayName: "User",
  provider: "imap",
  canSend: false,
};

function commandError(
  code: CommandError["code"],
  message = "core said so",
): CommandError {
  return { code, message, path: null };
}

/**
 * In the pane it ships in, because the way out of this screen is the pane's cross.
 *
 * The form has no leaving control of its own: leaving is refused while it is working, and the
 * cross reaches that through `leaveRef`. Rendered bare, these tests would prove nothing about the
 * button a person actually presses.
 */
function renderForm(
  onAdded = vi.fn(),
  onCancel = vi.fn(),
  secretsInAFile = false,
) {
  const leave: { current: (() => void) | null } = { current: null };
  render(
    <I18nProvider initial="en">
      <ScreenPane
        paneRef={{ current: null }}
        closeLabel="Close this screen"
        onClose={() => leave.current?.()}
      >
        <AccountSetup
          step="settings"
          onStep={vi.fn()}
          onAdded={onAdded}
          onCancel={onCancel}
          leaveRef={leave}
          secretsInAFile={secretsInAFile}
        />
      </ScreenPane>
    </I18nProvider>,
  );
  return { onAdded, onCancel };
}

/** The same form, correcting an account that is already here. */
function renderEdit(onAdded = vi.fn(), onCancel = vi.fn()) {
  const leave: { current: (() => void) | null } = { current: null };
  render(
    <I18nProvider initial="en">
      <ScreenPane
        paneRef={{ current: null }}
        closeLabel="Close this screen"
        onClose={() => leave.current?.()}
      >
        <AccountSetup
          step="settings"
          onStep={vi.fn()}
          onAdded={onAdded}
          onCancel={onCancel}
          leaveRef={leave}
          reconnecting="acc-1"
        />
      </ScreenPane>
    </I18nProvider>,
  );
  return { onAdded, onCancel };
}

/**
 * Both screens, wired the way the window wires them.
 *
 * The step is held outside the form and the way back is the arrow in the pane's corner, so a test
 * that rendered the form on its own would be testing neither.
 */
function renderFlow(
  onAdded = vi.fn(),
  onCancel = vi.fn(),
  secretsInAFile = false,
) {
  const leave: { current: (() => void) | null } = { current: null };

  function Flow() {
    const [step, setStep] = useState<"start" | "settings">("start");
    return (
      <ScreenPane
        paneRef={{ current: null }}
        closeLabel="Close this screen"
        onClose={() => leave.current?.()}
        onBack={step === "settings" ? () => setStep("start") : null}
        backLabel="Back"
      >
        <AccountSetup
          step={step}
          onStep={setStep}
          onAdded={onAdded}
          onCancel={onCancel}
          leaveRef={leave}
          secretsInAFile={secretsInAFile}
        />
      </ScreenPane>
    );
  }

  render(
    <I18nProvider initial="en">
      <Flow />
    </I18nProvider>,
  );
  return { onAdded, onCancel };
}

function type(label: RegExp, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

function field(label: RegExp): HTMLInputElement {
  return screen.getByLabelText(label) as HTMLInputElement;
}

function button(name: string): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

/** The three fields without which no request is worth making. */
function fillRequired() {
  type(/^Email address$/, "user@example.com");
  type(/^Server address$/, "imap.example.com");
  type(/^Password$/, "correct horse");
}

beforeEach(() => {
  looking.mockReset();
  looking.mockResolvedValue({ settings: null });
  add.mockReset();
  // Reset and given an answer: the screen chains onto what this returns, and a bare mock answers
  // with nothing to chain onto.
  change.mockReset();
  change.mockResolvedValue(undefined as never);
  check.mockReset();
  port.mockReset();
  port.mockResolvedValue(993);
  remove.mockReset();
  remove.mockResolvedValue(undefined);
  hinting.mockReset();
  hinting.mockResolvedValue(null);
  settingsOf.mockReset();
  settingsOf.mockResolvedValue({
    email: "user@example.com",
    displayName: "User",
    imapHost: "imap.example.com",
    imapPort: 993,
    imapEncryption: "tls",
    imapUsername: "user@example.com",
    submission: null,
  });
});

describe("AccountSetup", () => {
  it("calls the button Save when it is correcting an account already here", async () => {
    // It said "Add account" on a screen whose only call is changeAccount. A person correcting a
    // port read that as a second copy of their mailbox about to appear.
    renderEdit();

    expect(
      await screen.findByRole("button", { name: "Save" }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Add account" })).toBeNull();
  });

  it("still calls it Add account when there is no account yet", async () => {
    renderForm();

    expect(
      await screen.findByRole("button", { name: "Add account" }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("keeps the way to commit in view rather than at the end of the form", async () => {
    // Twelve fields are taller than any window this ships in. At the size it opens, the button sat
    // two hundred pixels below the fold, and opening an account's settings showed a form with no
    // way to commit it until the person scrolled to the end.
    renderEdit();

    const save = await screen.findByRole("button", { name: "Save" });
    const pinned = save.closest(".sticky");

    expect(pinned, "the way to commit is not in a pinned bar").toBeTruthy();
    expect(pinned?.className).toContain("bottom-0");
  });

  it("says why Save will not press", async () => {
    // Every other field is filled from what is stored and this one deliberately is not: the
    // password lives in this device's secret store and this window never reads it back. Without the
    // line, Save is grey for a reason nothing on the screen gives.
    renderEdit();

    const save = await screen.findByRole("button", { name: "Save" });
    expect(save).toHaveProperty("disabled", true);
    expect(screen.getByText(/Type the password again to save/)).toBeTruthy();
  });

  it("takes the line away once the password is typed", async () => {
    renderEdit();

    await screen.findByRole("button", { name: "Save" });
    fireEvent.change(screen.getByLabelText(/Password/), {
      target: { value: "secret" },
    });

    await waitFor(() =>
      expect(screen.queryByText(/Type the password again to save/)).toBeNull(),
    );
    expect(screen.getByRole("button", { name: "Save" })).toHaveProperty(
      "disabled",
      false,
    );
  });

  it("saves the corrected settings through the account it was opened for", async () => {
    renderEdit();

    await screen.findByRole("button", { name: "Save" });
    fireEvent.change(screen.getByLabelText(/Password/), {
      target: { value: "secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(change).toHaveBeenCalled());
    expect(change.mock.calls[0][0]).toBe("acc-1");
    expect(add).not.toHaveBeenCalled();
  });

  it("refuses to leave while the form is working, and lets go once it stops", async () => {
    // Walking out halfway through connecting leaves a person unable to say whether the account
    // was added. The button that held this refusal is gone -- the way out is the pane's cross --
    // so the refusal has to live in the leaving itself.
    let letItFinish: (outcome: ConnectionReportDto) => void = () => {};
    check.mockImplementation(
      () =>
        new Promise<ConnectionReportDto>((settle) => {
          letItFinish = settle;
        }),
    );

    const { onCancel } = renderForm();
    fillRequired();
    fireEvent.click(button("Check connection"));

    fireEvent.click(button("Close this screen"));
    expect(onCancel).not.toHaveBeenCalled();

    letItFinish({ mailboxCount: 1 });
    await waitFor(() =>
      expect(button("Check connection").disabled).toBe(false),
    );

    fireEvent.click(button("Close this screen"));
    expect(onCancel).toHaveBeenCalled();
  });

  it("sends what was typed and reports the account that came back", async () => {
    add.mockResolvedValue(ACCOUNT);
    const { onAdded } = renderForm();

    fillRequired();
    type(/^Display name$/, "User");
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add).toHaveBeenCalledWith(
      {
        email: "user@example.com",
        displayName: "User",
        imapHost: "imap.example.com",
        imapPort: 993,
        imapEncryption: "tls",
        imapUsername: "user@example.com",
        // Nobody typed a sending server, and that is an answer rather than a blank: the account is
        // added for reading, and says so when somebody tries to write from it.
        submission: null,
      },
      "correct horse",
    );
    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(ACCOUNT));
  });

  it("keeps the password out of the settings object", async () => {
    // The settings travel through logs and error messages; the password is a separate argument
    // exactly so it cannot be carried along with them.
    add.mockResolvedValue(ACCOUNT);
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    const settings = add.mock.calls[0][0] as Record<string, unknown>;
    expect(Object.values(settings)).not.toContain("correct horse");
    expect(JSON.stringify(settings)).not.toContain("correct horse");
  });

  it("uses the address as the user name when the field is left empty", async () => {
    add.mockResolvedValue(ACCOUNT);
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0].imapUsername).toBe("user@example.com");
  });

  it("sends the user name the server was given when it differs from the address", async () => {
    add.mockResolvedValue(ACCOUNT);
    renderForm();

    fillRequired();
    type(/^User name$/, "user.example");
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0].imapUsername).toBe("user.example");
  });

  it("says how many mailboxes the check found", async () => {
    check.mockResolvedValue({ mailboxCount: 7 });
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Check connection" }));

    // The answer to the question that was asked, and the evidence for it underneath.
    expect(await screen.findByText("Connection established")).toBeTruthy();
    expect(screen.getByText("Connected. 7 folders found")).toBeTruthy();
    expect(add).not.toHaveBeenCalled();
  });

  it("counts a single folder in the singular", async () => {
    check.mockResolvedValue({ mailboxCount: 1 });
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Check connection" }));

    expect(await screen.findByText("Connected. 1 folder found")).toBeTruthy();
  });

  it("shows a refused password as a refused password", async () => {
    add.mockRejectedValue(commandError("unauthorized"));
    const { onAdded } = renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    expect(
      await screen.findByText("The mail server rejected your sign-in"),
    ).toBeTruthy();
    expect(onAdded).not.toHaveBeenCalled();
  });

  it("names the file a master password opens when the password could not be put in it", async () => {
    // On a device whose secrets are in a file, the system's store named here is a place the device
    // does not have.
    add.mockRejectedValue(commandError("credentialsUnavailable"));
    const { onAdded } = renderForm(vi.fn(), vi.fn(), true);
    const named = LANGUAGES.en.catalog[secretStoreName()] as string;

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    const said = (await screen.findByRole("alert")).textContent;
    expect(said).toContain(
      "Could not access the file your master password opens on this device",
    );
    expect(said).not.toContain(named);
    expect(onAdded).not.toHaveBeenCalled();
  });

  it("points at the field the core rejected", async () => {
    // A form that reports "invalid request" makes the person re-read every field.
    add.mockRejectedValue(commandError("invalidHost"));
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    expect((await screen.findByRole("alert")).textContent).toContain(
      "That is not a server address.",
    );
  });

  it("says plainly that the address is already connected", async () => {
    add.mockRejectedValue(commandError("accountAlreadyAdded"));
    renderForm();

    fillRequired();
    fireEvent.click(button("Add account"));

    expect(
      await screen.findByText(
        "This address is already connected on this device",
      ),
    ).toBeTruthy();
  });

  it("tells a silent bridge apart from a server that refused", async () => {
    add.mockRejectedValue(new Error("the window has no core behind it"));
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    expect(
      await screen.findByText("The native core did not respond"),
    ).toBeTruthy();
  });

  it("clears a stale failure as soon as a field is edited", async () => {
    add.mockRejectedValue(commandError("unauthorized"));
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));
    await screen.findByRole("alert");

    type(/^Password$/, "the other one");

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("asks for nothing until there is an address, a server and a password", () => {
    renderForm();

    expect(button("Add account").disabled).toBe(true);
    expect(button("Check connection").disabled).toBe(true);

    fillRequired();

    expect(button("Add account").disabled).toBe(false);
    expect(button("Check connection").disabled).toBe(false);
  });

  it("accepts one submission at a time", async () => {
    // A second press while the first is in flight would add the account twice.
    add.mockReturnValue(new Promise(() => {}));
    renderForm();

    fillRequired();
    fireEvent.click(button("Add account"));

    await waitFor(() => expect(button("Adding…").disabled).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "Adding…" }));
    expect(add).toHaveBeenCalledTimes(1);
  });

  it("offers the port that goes with the chosen encryption", async () => {
    port.mockResolvedValue(143);
    renderForm();

    fireEvent.click(screen.getByRole("radio", { name: "STARTTLS" }));

    await waitFor(() => expect(field(/^Port$/).value).toBe("143"));
    expect(port).toHaveBeenCalledWith("startTls");
  });

  it("leaves a port alone once it was typed by hand", async () => {
    port.mockResolvedValue(143);
    renderForm();

    type(/^Port$/, "9993");
    fireEvent.click(screen.getByRole("radio", { name: "STARTTLS" }));

    await waitFor(() => expect(port).not.toHaveBeenCalled());
    expect(field(/^Port$/).value).toBe("9993");
  });

  it("sends the encryption that was chosen", async () => {
    // Silently sending TLS for a server that was set to plain would fail; the reverse would send
    // the password in the clear.
    add.mockResolvedValue(ACCOUNT);
    renderForm();

    fillRequired();
    fireEvent.click(screen.getByRole("radio", { name: "None" }));
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0].imapEncryption).toBe("none");
  });

  it("sends where to submit when somebody says where", async () => {
    add.mockResolvedValue(ACCOUNT);
    renderForm();
    fillRequired();

    type(/^Sending server$/, "smtp.example.com");
    fireEvent.click(screen.getByRole("button", { name: "Add account" }));

    await waitFor(() => expect(add).toHaveBeenCalledTimes(1));
    expect(add.mock.calls[0][0].submission).toEqual({
      host: "smtp.example.com",
      // Encrypted from the first byte, like reading: STARTTLS begins in the clear and asks to be
      // upgraded, and that request can be declined by whoever is in the middle.
      port: 465,
      encryption: "tls",
      // Nobody typed one, so it is the name the account already signs in with.
      username: "user@example.com",
    });
  });
});

describe("AccountSetup, removing an account", () => {
  // Every case here stands in the pane the screen ships in, with the window's own way of leaving
  // on Escape listened for: a dialog that let that key through would close the screen behind it.
  let windowSaw: string[] = [];
  const listen = (event: KeyboardEvent) => windowSaw.push(event.key);

  beforeEach(() => {
    windowSaw = [];
    window.addEventListener("keydown", listen);
    return () => window.removeEventListener("keydown", listen);
  });

  /**
   * The account screen for one account, as the window opens it: the account and the address its
   * list of accounts holds for it, handed in together. `again` turns the same screen to another.
   */
  function showEdit(
    { id, email }: { id: string; email: string | undefined } = {
      id: "acc-1",
      email: "user@example.com",
    },
    secretsInAFile = false,
  ) {
    const onRemoved = vi.fn();
    const onCancel = vi.fn();
    const leave: { current: (() => void) | null } = { current: null };
    const screenFor = (id: string, email: string | undefined) => (
      <I18nProvider initial="en">
        <ScreenPane
          paneRef={{ current: null }}
          closeLabel="Close this screen"
          onClose={() => leave.current?.()}
        >
          <AccountSetup
            step="settings"
            onStep={vi.fn()}
            onAdded={vi.fn()}
            onCancel={onCancel}
            onRemoved={onRemoved}
            leaveRef={leave}
            reconnecting={id}
            address={email}
            secretsInAFile={secretsInAFile}
          />
        </ScreenPane>
      </I18nProvider>
    );
    const { rerender } = render(screenFor(id, email));
    return {
      onRemoved,
      onCancel,
      again: (next: { id: string; email: string }) =>
        rerender(screenFor(next.id, next.email)),
    };
  }

  const trigger = () =>
    screen.findByRole("button", { name: "Remove this account" });

  async function ask() {
    fireEvent.click(await trigger());
    return screen.findByRole("alertdialog");
  }

  /** The field the address is typed into, whichever address the question names. */
  const address = () =>
    screen.getByLabelText(/^To confirm, type /) as HTMLInputElement;

  const typeAddress = (value: string) =>
    fireEvent.change(address(), { target: { value } });

  /** A removal the test answers when it chooses, so the moment in between can be looked at. */
  function removalOnItsWay() {
    let answer: () => void = () => {};
    remove.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          answer = resolve;
        }),
    );
    return { answer: () => answer() };
  }

  it("is not offered while an account is being added", async () => {
    renderForm();
    fillRequired();

    await screen.findByRole("button", { name: "Add account" });
    expect(
      screen.queryByRole("button", { name: "Remove this account" }),
    ).toBeNull();
  });

  it.each([
    ["none at all", undefined],
    ["an empty one", ""],
    ["only spaces", "   "],
  ])("is not offered with %s for the address", async (_, email) => {
    // The question asks for the address to be typed back. With none known, an empty field would
    // match it, and the one guard against a slip would be open before anything was typed.
    showEdit({ id: "acc-1", email });

    await screen.findByDisplayValue("imap.example.com");
    expect(
      screen.queryByRole("button", { name: "Remove this account" }),
    ).toBeNull();
  });

  it("is offered for an account whose settings will not read, and removes that account", async () => {
    // A row an older version wrote can list perfectly well and still refuse to read whole. That
    // is the account somebody opens in order to take it away, and nothing else offers to.
    settingsOf.mockRejectedValue(commandError("storageUnavailable"));
    const { onRemoved } = showEdit();
    await waitFor(() => expect(settingsOf).toHaveBeenCalledWith("acc-1"));

    const question = await ask();
    expect(
      screen.getByRole("alertdialog", {
        name: "Remove user@example.com from this device?",
      }),
    ).toBe(question);
    // Aligned to the start of the line, which is the right in a window that runs right to left.
    const header = question.querySelector('[data-slot="alert-dialog-header"]');
    expect(header?.className).toContain("text-start");
    expect(header?.className).not.toContain("text-left");

    typeAddress("user@example.com");
    fireEvent.click(button("Remove"));

    await waitFor(() => expect(onRemoved).toHaveBeenCalledWith("acc-1"));
    expect(remove).toHaveBeenCalledWith("acc-1");
  });

  it.each([
    ["before its settings arrive", () => new Promise<never>(() => {})],
    [
      "when its settings will not read",
      () => Promise.reject(commandError("storageUnavailable")),
    ],
  ])(
    "names the account it removes after the screen turns to another, %s",
    async (_, secondSettings) => {
      // The screen stays open while the sidebar or the settings turn it to another account. A
      // question still naming the first while removing the second would take away a mailbox
      // nobody typed the address of.
      const { again } = showEdit();
      await screen.findByDisplayValue("imap.example.com");
      settingsOf.mockImplementation(secondSettings);

      again({ id: "acc-2", email: "zeta@example.com" });
      await waitFor(() => expect(settingsOf).toHaveBeenCalledWith("acc-2"));

      await ask();
      expect(
        screen.getByRole("alertdialog", {
          name: "Remove zeta@example.com from this device?",
        }),
      ).toBeTruthy();

      typeAddress("user@example.com");
      expect(button("Remove").disabled).toBe(true);

      typeAddress("zeta@example.com");
      fireEvent.click(button("Remove"));

      await waitFor(() => expect(remove).toHaveBeenCalledWith("acc-2"));
      expect(remove).not.toHaveBeenCalledWith("acc-1");
    },
  );

  it("hands no letter pressed inside the dialog on to the window", async () => {
    // The window answers plain letters with shortcuts, and one taken behind the dialog -- `c`
    // opens a message -- happens where nobody can see it, under a question that then seems to be
    // about something else.
    showEdit();
    await ask();

    const keep = button("Keep it");
    keep.focus();
    for (const key of ["c", "r", "/", "?"]) {
      fireEvent.keyDown(keep, { key });
      fireEvent.keyDown(address(), { key });
    }

    expect(windowSaw).toEqual([]);
  });

  it("is red from the palette being worn, not a red of its own", async () => {
    showEdit();

    const classes = (await trigger()).className;
    expect(classes).toContain("text-destructive");
    expect(classes).not.toMatch(/red-|#[0-9a-f]{3}|rgb|oklch/i);
  });

  it("asks in a dialog that names the address and what goes with it", async () => {
    showEdit();

    const question = await ask();

    expect(
      screen.getByRole("alertdialog", {
        name: "Remove user@example.com from this device?",
      }),
    ).toBe(question);
    expect(question.textContent).toContain(
      "Its mail, its password and anything waiting to be sent go with it",
    );
    // The field says which address it wants, not only the title above it.
    expect(address()).toBe(
      screen.getByLabelText("To confirm, type user@example.com"),
    );
    expect(remove).not.toHaveBeenCalled();
  });

  it("puts the focus in the field the address is typed into", async () => {
    showEdit();

    await ask();

    await waitFor(() => expect(document.activeElement).toBe(address()));
  });

  it.each([
    ["nothing typed", ""],
    ["another address", "zeta@example.com"],
    ["the address half typed", "user@example.c"],
    ["the address with more after it", "user@example.com."],
    ["only spaces", "   "],
  ])("keeps Remove shut with %s", async (_, typed) => {
    showEdit();
    await ask();

    typeAddress(typed);

    expect(button("Remove").disabled).toBe(true);
  });

  it.each([
    ["exactly as stored", "user@example.com"],
    ["in another case", "USER@Example.COM"],
    ["with spaces around it", "  user@example.com  "],
  ])("opens Remove for the address typed %s", async (_, typed) => {
    showEdit();
    await ask();

    typeAddress(typed);

    expect(button("Remove").disabled).toBe(false);
  });

  it("does nothing on Enter until the address matches", async () => {
    showEdit();
    await ask();

    typeAddress("user@example.c");
    fireEvent.keyDown(address(), { key: "Enter" });

    expect(remove).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeTruthy();
  });

  it("removes on Enter once the address matches, and saves nothing on the way", async () => {
    // The field lives in a dialog opened from the account form. A submit carried out to that form
    // would save the settings as a side effect of removing the account.
    showEdit();
    await ask();

    typeAddress("user@example.com");
    fireEvent.keyDown(address(), { key: "Enter" });

    await waitFor(() => expect(remove).toHaveBeenCalledWith("acc-1"));
    expect(change).not.toHaveBeenCalled();
    expect(add).not.toHaveBeenCalled();
  });

  it("leaves the account alone on Keep it, and gives the focus back", async () => {
    showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Keep it"));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(remove).not.toHaveBeenCalled();
    await waitFor(async () =>
      expect(document.activeElement).toBe(await trigger()),
    );
  });

  it("leaves the account and the screen alone on Escape", async () => {
    const { onCancel } = showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.keyDown(address(), { key: "Escape" });

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(remove).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
    expect(windowSaw).not.toContain("Escape");
    await waitFor(async () =>
      expect(document.activeElement).toBe(await trigger()),
    );
  });

  it("removes the account it was opened for, and says so only once it is gone", async () => {
    const removal = removalOnItsWay();
    const { onRemoved } = showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Remove"));

    await waitFor(() => expect(remove).toHaveBeenCalledWith("acc-1"));
    expect(onRemoved).not.toHaveBeenCalled();

    removal.answer();

    await waitFor(() => expect(onRemoved).toHaveBeenCalledWith("acc-1"));
    expect(remove).toHaveBeenCalledTimes(1);
  });

  it("cannot be put away while the removal is on its way", async () => {
    const removal = removalOnItsWay();
    const { onRemoved } = showEdit();
    await ask();
    typeAddress("user@example.com");
    fireEvent.click(button("Remove"));

    const going = await screen.findByRole("button", { name: "Removing…" });
    expect((going as HTMLButtonElement).disabled).toBe(true);
    expect(button("Keep it").disabled).toBe(true);

    fireEvent.keyDown(address(), { key: "Escape" });
    fireEvent.click(button("Keep it"));

    expect(screen.getByRole("alertdialog")).toBeTruthy();
    expect(windowSaw).not.toContain("Escape");

    removal.answer();
    await waitFor(() => expect(onRemoved).toHaveBeenCalledWith("acc-1"));
  });

  it("asks the core once however many times Enter is pressed", async () => {
    removalOnItsWay();
    showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.keyDown(address(), { key: "Enter" });
    fireEvent.keyDown(address(), { key: "Enter" });
    fireEvent.click(await screen.findByRole("button", { name: "Removing…" }));

    expect(remove).toHaveBeenCalledTimes(1);
  });

  it("stays open and says the account was not removed when the core refuses", async () => {
    remove.mockRejectedValue(commandError("storageUnavailable"));
    const { onRemoved } = showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Remove"));

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("The account was not removed");
    expect(said.textContent).toContain(
      "The local database could not be opened",
    );
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    expect(onRemoved).not.toHaveBeenCalled();
    // And it can be tried again from where it stands.
    expect(button("Remove").disabled).toBe(false);
    expect(button("Keep it").disabled).toBe(false);
  });

  it("names the file a master password opens when a password could not be taken out of it", async () => {
    // The core keeps an account whose password it could not delete: removed over the top of it,
    // the password would stay on the device with nothing left to say whose it is.
    remove.mockRejectedValue(commandError("credentialsUnavailable"));
    const { onRemoved } = showEdit(undefined, true);
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Remove"));

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("The account was not removed");
    expect(said.textContent).toContain(
      "Could not access the file your master password opens on this device",
    );
    expect(onRemoved).not.toHaveBeenCalled();
  });

  it("says the same when the core does not answer at all", async () => {
    remove.mockRejectedValue(new Error("bridge down"));
    showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Remove"));

    const said = await screen.findByRole("alert");
    expect(said.textContent).toContain("The account was not removed");
    expect(said.textContent).toContain("The native core did not respond");
    expect(screen.getByRole("alertdialog")).toBeTruthy();
  });

  it("removes on a second try after a refusal", async () => {
    remove.mockRejectedValueOnce(commandError("storageUnavailable"));
    const { onRemoved } = showEdit();
    await ask();
    typeAddress("user@example.com");

    fireEvent.click(button("Remove"));
    await screen.findByRole("alert");
    fireEvent.click(button("Remove"));

    await waitFor(() => expect(onRemoved).toHaveBeenCalledWith("acc-1"));
    expect(remove).toHaveBeenCalledTimes(2);
  });

  it("asks afresh each time it is opened", async () => {
    remove.mockRejectedValueOnce(commandError("storageUnavailable"));
    showEdit();
    await ask();
    typeAddress("user@example.com");
    fireEvent.click(button("Remove"));
    await screen.findByRole("alert");

    fireEvent.click(button("Keep it"));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    await ask();

    expect(address().value).toBe("");
    expect(button("Remove").disabled).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("asks for the address the account has, not one typed over it", async () => {
    showEdit();
    await waitFor(() =>
      expect(field(/^Email address$/).value).toBe("user@example.com"),
    );
    type(/^Email address$/, "zeta@example.com");

    await ask();
    expect(
      screen.getByRole("alertdialog", {
        name: "Remove user@example.com from this device?",
      }),
    ).toBeTruthy();

    typeAddress("zeta@example.com");
    expect(button("Remove").disabled).toBe(true);

    typeAddress("user@example.com");
    expect(button("Remove").disabled).toBe(false);
  });
});

describe("AccountSetup, the two screens of adding one", () => {
  it("tells a person where the password went, by the name their system uses", () => {
    // "The credential store" is not what any of these systems calls it, so it is a sentence a
    // person cannot go and check. Keychain is one they can.
    renderFlow();

    // Whatever this system is called here, rather than a name written down twice.
    const named = LANGUAGES.en.catalog[secretStoreName()] as string;

    expect(
      screen.getByText(
        `Goes straight to ${named} on this device. It is never written to the app's database or to a log`,
      ),
    ).toBeTruthy();
    expect(
      screen.queryByText(/credential store/i),
      "the words nobody's system uses are gone",
    ).toBeNull();
  });

  it("names the file a master password opens, on a device whose secrets are in one", () => {
    // Saying a password went to the system's store when it went into a file is the confusion the
    // master password exists to prevent, and this is the first screen after choosing one.
    renderFlow(vi.fn(), vi.fn(), true);
    const named = LANGUAGES.en.catalog[secretStoreName()] as string;

    expect(
      screen.getByText(/your password is kept in the file your master password opens/i),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "Goes straight to the file your master password opens on this device. It is never written to the app's database or to a log",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(new RegExp(named, "i"))).toBeNull();
  });

  it("asks for three things and nothing else", () => {
    // A person who has to answer twelve questions to read their mail closes the window at the
    // fourth. The other nine are behind this screen, for the mailbox nobody can answer for.
    renderFlow();

    expect(screen.getByLabelText(/^Display name$/)).toBeTruthy();
    // A name runs the window's way until a letter decides it.
    expect(screen.getByLabelText(/^Display name$/).getAttribute("dir")).toBeNull();
    type(/^Display name$/, "Zeta Q.");
    expect(screen.getByLabelText(/^Display name$/).getAttribute("dir")).toBe("auto");
    expect(screen.getByLabelText(/^Email address$/)).toBeTruthy();
    expect(screen.getByLabelText(/^Password$/)).toBeTruthy();
    expect(screen.queryByLabelText(/^Server address$/)).toBeNull();
  });

  it("shows the password only while the eye is held down", () => {
    // A switch that stays on is a password left in the clear for whoever is standing behind this
    // person; held, it is on screen exactly as long as somebody is looking at it on purpose.
    renderFlow();

    const field = screen.getByLabelText(/^Password$/) as HTMLInputElement;
    expect(field.type).toBe("password");
    // Nothing to reveal yet, so nothing offering to reveal it.
    expect(
      screen.queryByRole("button", { name: /show the password/i }),
    ).toBeNull();

    type(/^Password$/, "correct horse");
    const eye = screen.getByRole("button", { name: /show the password/i });
    // Field and eye run left to right together, so the room the field keeps for the eye is on the
    // side the eye stands in a window that runs either way.
    expect(eye.parentElement?.getAttribute("dir")).toBe("ltr");
    expect(eye.parentElement?.contains(field)).toBe(true);
    fireEvent.pointerDown(eye);
    expect(field.type).toBe("text");

    fireEvent.pointerUp(eye);
    expect(field.type).toBe("password");

    // And when the press ends somewhere this button did not hear about.
    fireEvent.pointerDown(eye);
    fireEvent.pointerLeave(eye);
    expect(field.type).toBe("password");
  });

  it("connects without the long form when the address is one somebody knows", async () => {
    looking.mockResolvedValue({
      settings: {
        email: "user@example.com",
        displayName: "",
        imapHost: "imap.example.com",
        imapPort: 993,
        imapEncryption: "tls",
        imapUsername: "user@example.com",
        submission: null,
      },
    });
    add.mockResolvedValue(ACCOUNT);

    const { onAdded } = renderFlow();
    type(/^Email address$/, "user@example.com");
    type(/^Password$/, "correct horse");

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => expect(onAdded).toHaveBeenCalledWith(ACCOUNT));
    // The eight fields it would have asked for were never shown, which is the point of it.
    expect(screen.queryByLabelText(/^Server address$/)).toBeNull();
  });

  /**
   * What the core answers for a second mailbox at a domain one account here already reads.
   *
   * The servers are that account's, and the user names are this address spelled the way that
   * account signs in — here, the part before the `@` for reading and the whole address for sending.
   */
  const FROM_AN_ACCOUNT_HERE = {
    settings: {
      email: "kappa@example.com",
      displayName: "",
      imapHost: "imap.example.net",
      imapPort: 143,
      imapEncryption: "startTls" as const,
      imapUsername: "kappa",
      submission: {
        host: "smtp.example.net",
        port: 587,
        encryption: "startTls" as const,
        username: "kappa@example.com",
      },
    },
  };

  it("adds a second mailbox at a domain already here from the first screen alone", async () => {
    // Several mailboxes on one domain: from the second one on, nothing
    // more to type than the address and the password.
    looking.mockResolvedValue(FROM_AN_ACCOUNT_HERE);
    add.mockResolvedValue({ ...ACCOUNT, email: "kappa@example.com" });

    const { onAdded } = renderFlow();
    type(/^Email address$/, "kappa@example.com");
    type(/^Password$/, "correct horse");

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    await waitFor(() => expect(onAdded).toHaveBeenCalledTimes(1));
    expect(looking).toHaveBeenCalledWith("kappa@example.com", undefined);
    expect(add).toHaveBeenCalledTimes(1);
    expect(add).toHaveBeenCalledWith(
      {
        email: "kappa@example.com",
        displayName: "",
        imapHost: "imap.example.net",
        imapPort: 143,
        imapEncryption: "startTls",
        imapUsername: "kappa",
        submission: {
          host: "smtp.example.net",
          port: 587,
          encryption: "startTls",
          username: "kappa@example.com",
        },
      },
      // Typed on the first screen, and the only password in it: nothing of the other account's
      // travels with its servers.
      "correct horse",
    );
    expect(
      screen.queryByLabelText(/^Server address$/),
      "the second screen was shown for servers already known",
    ).toBeNull();
  });

  it("opens the long form on the reused servers when they refuse, and says why", async () => {
    looking.mockResolvedValue(FROM_AN_ACCOUNT_HERE);
    add.mockRejectedValue(commandError("unauthorized"));

    renderFlow();
    type(/^Email address$/, "kappa@example.com");
    type(/^Password$/, "wrong horse");

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    // Every field the refusal might be about, filled in with what was tried rather than empty.
    expect(
      ((await screen.findByLabelText(/^Server address$/)) as HTMLInputElement)
        .value,
    ).toBe("imap.example.net");
    expect(field(/^Port$/).value).toBe("143");
    expect(
      screen.getByRole("radio", { name: "STARTTLS" }).getAttribute(
        "aria-checked",
      ),
    ).toBe("true");
    expect(field(/^User name$/).value).toBe("kappa");
    expect(field(/^Sending server$/).value).toBe("smtp.example.net");
    expect(field(/^Sending port$/).value).toBe("587");
    expect(field(/^Sending user name$/).value).toBe("kappa@example.com");
    expect(field(/^Password$/).value).toBe("wrong horse");
    expect(
      await screen.findByText("The mail server rejected your sign-in"),
    ).toBeTruthy();
  });

  it("asks nobody about an address while it is being typed", async () => {
    // A cascade on every keystroke would send half-finished domains to whoever owns them, and
    // every prefix of a domain on its way to being typed may be one somebody owns. Until now this held only
    // because the lookup had one call site, so a second one added anywhere would have made it
    // false with every suite still green.
    //
    // The timers are faked and then wound a long way forward, because a debounce is the shape this
    // would arrive in: typing that asks nobody for three hundred milliseconds and then asks is
    // still typing that asks.
    vi.useFakeTimers();
    try {
      renderFlow();

      const address = "user@example.com";
      for (let at = 1; at <= address.length; at += 1) {
        type(/^Email address$/, address.slice(0, at));
      }
      type(/^Password$/, "correct horse");
      vi.advanceTimersByTime(30_000);

      expect(looking).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }

    // And the other half, so that a screen which failed to draw at all cannot satisfy this by
    // having no field to type in: pressing Continue does ask.
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    // Asked about the address alone: adding an account corrects none.
    await waitFor(() =>
      expect(looking).toHaveBeenCalledWith("user@example.com", undefined),
    );
  });

  it("asks nobody about an address typed into the long form either", async () => {
    // The second door. The long form has an address field of its own, wired to the same editing
    // function, and a lookup added there would be invisible to the test above — which never
    // reaches this screen with an address to type.
    vi.useFakeTimers();
    try {
      renderForm();

      const address = "user@example.com";
      for (let at = 1; at <= address.length; at += 1) {
        type(/^Email address$/, address.slice(0, at));
      }
      vi.advanceTimersByTime(30_000);

      expect(looking).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }

    expect(field(/^Email address$/).value).toBe("user@example.com");
  });

  it("names the file a master password opens when an account's password is missing from it", async () => {
    render(
      <I18nProvider initial="en">
        <AccountSetup
          step="settings"
          onStep={vi.fn()}
          onAdded={vi.fn()}
          onCancel={vi.fn()}
          reconnecting="acc-1"
          passwordMissing
          secretsInAFile
        />
      </I18nProvider>,
    );
    await screen.findByDisplayValue("imap.example.com");
    const named = LANGUAGES.en.catalog[secretStoreName()] as string;

    expect(
      screen.getByText(
        /is not in the file your master password opens any more/i,
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(/the one kept in the file your master password opens/i),
    ).toBeTruthy();
    expect(screen.queryByText(new RegExp(named, "i"))).toBeNull();
  });

  it("asks nobody about the address of an account already on this device", async () => {
    // The third door, and the one nobody would think to look at: signing in again fills the
    // address from what is stored, without a keystroke. A lookup keyed on the address rather than
    // on the press would therefore send the address of an existing account the moment this screen
    // opened — the same leak, with nobody typing at all.
    render(
      <I18nProvider initial="en">
        <AccountSetup
          step="settings"
          onStep={vi.fn()}
          onAdded={vi.fn()}
          onCancel={vi.fn()}
          reconnecting="acc-1"
        />
      </I18nProvider>,
    );

    // The stored settings have landed, so the address is in the field and any lookup keyed on it
    // would have run by now.
    await screen.findByDisplayValue("imap.example.com");
    expect(field(/^Email address$/).value).toBe("user@example.com");

    expect(looking).not.toHaveBeenCalled();
  });

  it("goes to the long form when nobody knows the address, carrying what was typed", async () => {
    looking.mockResolvedValue({ settings: null });

    renderFlow();
    type(/^Display name$/, "Work");
    type(/^Email address$/, "someone@nowhere.test");
    type(/^Password$/, "correct horse");

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    const host = await screen.findByLabelText(/^Server address$/);
    expect(host).toBeTruthy();
    // Typed once. A second screen that asked for the address again would be a form that lost it.
    expect(
      (screen.getByLabelText(/^Email address$/) as HTMLInputElement).value,
    ).toBe("someone@nowhere.test");
    expect(
      (screen.getByLabelText(/^Display name$/) as HTMLInputElement).value,
    ).toBe("Work");
    expect(add).not.toHaveBeenCalled();
  });

  it("goes to the long form when the connection is refused, and says why", async () => {
    // Three fields is nowhere to act on a refusal: the trouble is usually in one of the eight
    // this screen does not show.
    looking.mockResolvedValue({
      settings: {
        email: "user@example.com",
        displayName: "",
        imapHost: "imap.example.com",
        imapPort: 993,
        imapEncryption: "tls",
        imapUsername: "user@example.com",
        submission: null,
      },
    });
    add.mockRejectedValue(commandError("unauthorized"));

    renderFlow();
    type(/^Email address$/, "user@example.com");
    type(/^Password$/, "wrong horse");

    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(await screen.findByLabelText(/^Server address$/)).toBeTruthy();
    expect(
      await screen.findByText("The mail server rejected your sign-in"),
      "the refusal is said where it can be acted on",
    ).toBeTruthy();
  });

  it("comes back to the first screen by the arrow, with what was typed still there", async () => {
    looking.mockResolvedValue({ settings: null });

    renderFlow();
    type(/^Email address$/, "someone@nowhere.test");
    type(/^Password$/, "correct horse");
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByLabelText(/^Server address$/);

    fireEvent.click(screen.getByRole("button", { name: "Back" }));

    expect(screen.queryByLabelText(/^Server address$/)).toBeNull();
    expect(
      (screen.getByLabelText(/^Email address$/) as HTMLInputElement).value,
    ).toBe("someone@nowhere.test");
  });

  it("has no way back from the first screen, because there is nothing behind it", () => {
    renderFlow();

    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Close this screen" }),
    ).toBeTruthy();
  });

  it("says which password a provider wants before the password is typed", () => {
    // The shop window promises this in as many words, and until now nothing in the window asked:
    // the domain knew which providers want an app password, the core would answer, and no screen
    // had ever called it. A person typed the password they sign in with and their own server told
    // them it was wrong, which is true and useless.
    hinting.mockResolvedValue("account.needsAppPassword");
    renderFlow();

    fireEvent.change(screen.getByLabelText(/^Email address$/), {
      // Which domain it is decides nothing here: what a provider wants is the core's answer, and
      // the core is the mock two lines up. A domain that reaches somebody is refused by the
      // anonymity gate whatever a test meant by it.
      target: { value: "somebody@example.org" },
    });
    // On leaving the field, which is the moment before the password below it is typed.
    fireEvent.blur(screen.getByLabelText(/^Email address$/));

    return waitFor(() => {
      expect(
        screen.getByText(/app password/i),
        "nothing warned about which password this provider wants",
      ).toBeTruthy();
      // Two sentences, two lines: the warning and where the password goes are each their own,
      // since no line ends with a stop and run together they read as one.
      expect(screen.getByText(/app password/i).textContent).not.toMatch(
        /Goes straight to/,
      );
      expect(screen.getByText(/^Goes straight to/)).toBeTruthy();
    });
  });

  it("says nothing extra about a provider that wants nothing said", () => {
    // The other half, and the one that decides whether the sentence above means anything: a hint
    // shown for every address is a hint that says nothing about any of them.
    hinting.mockResolvedValue(null);
    renderFlow();

    fireEvent.change(screen.getByLabelText(/^Email address$/), {
      target: { value: "somebody@example.com" },
    });
    fireEvent.blur(screen.getByLabelText(/^Email address$/));

    return waitFor(() => {
      expect(screen.queryByText(/app password/i)).toBeNull();
      // And the field is still there with its ordinary hint, rather than gone.
      expect(screen.getByLabelText(/^Password$/)).toBeTruthy();
    });
  });
});
