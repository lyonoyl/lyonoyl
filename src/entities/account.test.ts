import { describe, expect, it } from "vitest";

import type { WorkspaceOverviewDto } from "@/shared/ipc/generated/WorkspaceOverviewDto";

import { signedAs, withAccount } from "./account";

const mailbox = (id: string, signature?: string) => ({
  id,
  email: `${id}@example.com`,
  displayName: "",
  provider: "imap",
  canSend: true,
  signature,
});

describe("signedAs", () => {
  it("signs the one mailbox as the core kept it and leaves the others", () => {
    const workspace = {
      accounts: [mailbox("zeta", "Zeta Quorin"), mailbox("kappa")],
    } as unknown as WorkspaceOverviewDto;

    const signed = signedAs(workspace, "kappa", "Kappa Vandel");

    expect(signed.accounts.map((one) => one.signature)).toEqual([
      "Zeta Quorin",
      "Kappa Vandel",
    ]);
    expect(signed.accounts[0]).toBe(workspace.accounts[0]);
  });

  it("takes the signature away when the core kept none", () => {
    const workspace = {
      accounts: [mailbox("zeta", "Zeta Quorin")],
    } as unknown as WorkspaceOverviewDto;

    expect(signedAs(workspace, "zeta", "").accounts[0].signature).toBe(
      undefined,
    );
  });
});

describe("withAccount", () => {
  it("puts a corrected mailbox in its place, keeping the signature the form knows nothing of", () => {
    const workspace = {
      accounts: [mailbox("zeta", "Zeta Quorin"), mailbox("kappa")],
    } as unknown as WorkspaceOverviewDto;
    const corrected = { ...mailbox("zeta"), displayName: "Work" };

    const after = withAccount(workspace, corrected);

    expect(after.accounts.map((one) => [one.id, one.displayName, one.signature])).toEqual([
      ["zeta", "Work", "Zeta Quorin"],
      ["kappa", "", undefined],
    ]);
  });

  it("puts a new mailbox last, with no signature", () => {
    const workspace = {
      accounts: [mailbox("zeta", "Zeta Quorin")],
    } as unknown as WorkspaceOverviewDto;

    const after = withAccount(workspace, mailbox("sigma"));

    expect(after.accounts.map((one) => [one.id, one.signature])).toEqual([
      ["zeta", "Zeta Quorin"],
      ["sigma", undefined],
    ]);
  });
});
