import { describe, expect, it } from "vitest";

import { addressesIn, everyOneAnAddress, looksLikeAnAddress } from "./addresses";

describe("what a line of recipients holds", () => {
  it("splits on commas and drops the spaces around each", () => {
    expect(addressesIn(" zeta@example.com , kappa@example.net ")).toEqual([
      "zeta@example.com",
      "kappa@example.net",
    ]);
  });

  it("holds nothing when nothing was written", () => {
    expect(addressesIn("  ,  , ")).toEqual([]);
  });
});

describe("whether something is an address", () => {
  it.each([
    "zeta@example.com",
    "zeta+list@example.com",
    "a@b",
    "zeta@localhost",
    "ζήτα@ζήτα.example",
  ])("takes %s", (written) => {
    // Deliberately loose. The rule is older than this application and stranger than anybody
    // expects, and a check that went further would refuse mail somebody can actually send.
    expect(looksLikeAnAddress(written)).toBe(true);
  });

  it.each([
    "zeta",
    "@example.com",
    "zeta@",
    "zeta@@example.com",
    "zeta example@com",
    "Zeta <zeta@example.com>",
  ])("refuses %s", (written) => {
    expect(looksLikeAnAddress(written)).toBe(false);
  });
});

describe("whether a line can be sent to", () => {
  it("refuses an empty line", () => {
    expect(everyOneAnAddress("   ")).toBe(false);
  });

  it("takes a line of them", () => {
    expect(everyOneAnAddress("zeta@example.com, kappa@example.net")).toBe(true);
  });

  it("refuses a line whose second address is half typed", () => {
    // Sent, this would reach the first and fail on the second: a message half sent.
    expect(everyOneAnAddress("zeta@example.com, bor")).toBe(false);
  });
});
