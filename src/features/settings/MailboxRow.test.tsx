import { readFileSync } from "node:fs";
import { join } from "node:path";

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { I18nProvider } from "@/shared/i18n";
import { setSignature } from "@/shared/ipc/client";
import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import { LONGEST_SIGNATURE } from "@/shared/ui/SignatureText";

import { MailboxRow } from "./MailboxRow";

vi.mock("@/shared/ipc/client", () => ({ setSignature: vi.fn() }));

const keeping = vi.mocked(setSignature);

const WORK: MailAccountSummaryDto = {
  id: "acc-1",
  email: "kappa@example.net",
  displayName: "Work",
  provider: "imap",
  canSend: true,
  signature: "Kappa Vandel\nkappa@example.net",
};

function renderRow(account: MailAccountSummaryDto = WORK) {
  const kept = vi.fn();
  const shown = (account: MailAccountSummaryDto) => (
    <I18nProvider initial="en">
      <ul>
        <MailboxRow
          account={account}
          groups={[]}
          onChooseGroup={vi.fn()}
          onOpen={vi.fn()}
          onSignatureKept={kept}
        />
      </ul>
    </I18nProvider>
  );
  const { rerender } = render(shown(account));
  return { kept, keptAs: (next: MailAccountSummaryDto) => rerender(shown(next)) };
}

const opener = () => screen.getByRole("button", { name: "Signature for Work" });
const field = () =>
  screen.getByLabelText("Signature for Work", {
    selector: "textarea",
  }) as HTMLTextAreaElement;

beforeEach(() => {
  keeping.mockReset();
  keeping.mockImplementation(async (_account, typed) => typed.trim());
});

describe("a mailbox's signature", () => {
  it("opens under the mailbox from a button that names it, as the core keeps it, ready to write in", () => {
    renderRow();
    expect(opener().getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("textbox")).toBeNull();

    fireEvent.click(opener());

    expect(opener().getAttribute("aria-expanded")).toBe("true");
    expect(opener().getAttribute("aria-controls")).toBe(field().id);
    expect(field().value).toBe("Kappa Vandel\nkappa@example.net");
    expect(document.activeElement).toBe(field());
    expect(
      document.getElementById(field().getAttribute("aria-describedby") ?? "")
        ?.textContent,
    ).toBe(
      "Added below what you write in every message you start from this mailbox",
    );
  });

  it("opens empty for a mailbox that has none", () => {
    renderRow({ ...WORK, signature: undefined });

    fireEvent.click(opener());

    expect(field().value).toBe("");
  });

  it("keeps what was typed once the field is left, and passes on the signature as the core kept it", async () => {
    keeping.mockResolvedValue("Sigma Ordrin");
    const { kept } = renderRow();
    fireEvent.click(opener());

    fireEvent.change(field(), { target: { value: "Sigma Ordrin  \n" } });
    fireEvent.blur(field());

    expect(keeping).toHaveBeenCalledWith("acc-1", "Sigma Ordrin  \n");
    await waitFor(() =>
      expect(kept).toHaveBeenCalledWith("acc-1", "Sigma Ordrin"),
    );
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps nothing when the field is left as it was", () => {
    renderRow();
    fireEvent.click(opener());

    fireEvent.blur(field());

    expect(keeping).not.toHaveBeenCalled();
  });

  it("keeps what was typed when the button closes the field with the focus still in it", async () => {
    // On macOS a button takes no focus when it is clicked, so nothing but the button leaves the field.
    const { kept } = renderRow();
    fireEvent.click(opener());
    fireEvent.change(field(), { target: { value: "Sigma Ordrin" } });
    expect(document.activeElement).toBe(field());

    fireEvent.click(opener());

    expect(keeping).toHaveBeenCalledWith("acc-1", "Sigma Ordrin");
    expect(screen.queryByRole("textbox")).toBeNull();
    await waitFor(() =>
      expect(kept).toHaveBeenCalledWith("acc-1", "Sigma Ordrin"),
    );
  });

  it("says so when the signature could not be kept, and opens the field again on what was typed", async () => {
    keeping.mockRejectedValueOnce({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    const { kept } = renderRow();
    fireEvent.click(opener());
    fireEvent.change(field(), { target: { value: "Sigma Ordrin" } });

    fireEvent.click(opener());

    expect(
      (await screen.findByRole("alert")).textContent,
    ).toBe("The signature could not be saved");
    expect(field().value).toBe("Sigma Ordrin");
    expect(kept).not.toHaveBeenCalled();

    fireEvent.blur(field());

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(kept).toHaveBeenCalledWith("acc-1", "Sigma Ordrin");
  });

  it("gives up an edit on Escape and stays, and lets Escape leave the settings when there is none", () => {
    const heard = vi.fn();
    window.addEventListener("keydown", heard);
    try {
      renderRow();
      fireEvent.click(opener());
      fireEvent.change(field(), { target: { value: "Sigma Ordrin" } });

      fireEvent.keyDown(field(), { key: "Escape" });

      expect(field().value).toBe("Kappa Vandel\nkappa@example.net");
      expect(heard).not.toHaveBeenCalled();

      fireEvent.keyDown(field(), { key: "Escape" });

      expect(heard).toHaveBeenCalledTimes(1);
    } finally {
      window.removeEventListener("keydown", heard);
    }
  });

  it("shows the signature the core kept when it changes underneath", () => {
    const { keptAs } = renderRow();
    fireEvent.click(opener());

    act(() => keptAs({ ...WORK, signature: "Sigma Ordrin" }));

    expect(field().value).toBe("Sigma Ordrin");
  });

  it("stops the typing where the core stops keeping", () => {
    // The two bounds are one rule written twice, in two languages: read from the core's source so
    // they cannot drift apart.
    const domain = readFileSync(
      join(
        __dirname,
        "..",
        "..",
        "..",
        "crates/lyonoyl-domain/src/signature/mod.rs",
      ),
      "utf8",
    );
    const core = /pub const LONGEST_SIGNATURE: usize = ([\d_]+);/.exec(domain);

    expect(Number(core?.[1].replace(/_/g, ""))).toBe(LONGEST_SIGNATURE);

    renderRow();
    fireEvent.click(opener());
    expect(field().maxLength).toBe(LONGEST_SIGNATURE);
  });
});

describe("a signature that could not be kept", () => {
  it("is given up on Escape, with the line saying so, and the field opens again on what is kept", async () => {
    keeping.mockRejectedValueOnce({
      code: "storageUnavailable",
      message: "",
      path: null,
    });
    renderRow();
    fireEvent.click(opener());
    fireEvent.change(field(), { target: { value: "Sigma Ordrin" } });
    fireEvent.blur(field());
    await screen.findByRole("alert");

    fireEvent.keyDown(field(), { key: "Escape" });

    expect(screen.queryByRole("alert")).toBeNull();
    expect(field().value).toBe("Kappa Vandel\nkappa@example.net");

    fireEvent.click(opener());
    fireEvent.click(opener());

    expect(field().value).toBe("Kappa Vandel\nkappa@example.net");
  });
});
