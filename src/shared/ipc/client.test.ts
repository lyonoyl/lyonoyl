import { describe, expect, it } from "vitest";

import { isCommandError } from "./client";

describe("isCommandError", () => {
  it("recognizes a typed error coming from Rust", () => {
    expect(
      isCommandError({
        code: "storageUnavailable",
        message: "unavailable",
        path: "/tmp/db",
      }),
    ).toBe(true);
  });

  it("rejects a bare Error, so a bridge failure is not mistaken for a command error", () => {
    expect(isCommandError(new Error("boom"))).toBe(false);
  });

  it("rejects null and undefined without throwing", () => {
    expect(isCommandError(null)).toBe(false);
    expect(isCommandError(undefined)).toBe(false);
  });

  it("rejects an object whose code is not a string", () => {
    expect(isCommandError({ code: 500, message: "nope" })).toBe(false);
  });
});
