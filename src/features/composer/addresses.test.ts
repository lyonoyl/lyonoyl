import { describe, expect, it } from "vitest";

import {
  addressesIn,
  beingTyped,
  everyOneAnAddress,
  looksLikeAnAddress,
  withChosen,
} from "./addresses";

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

describe("the address being typed", () => {
  it("is the whole line while it holds one address", () => {
    expect(beingTyped("zet", 3)).toEqual({ start: 0, end: 3, typed: "zet" });
  });

  it("starts after the comma before the caret and the spaces past it", () => {
    expect(beingTyped("zeta@example.com,   kap", 23)).toEqual({
      start: 20,
      end: 23,
      typed: "kap",
    });
  });

  it("ends at the next comma, and holds only what is before the caret", () => {
    const line = "zeta@example.com, kapXYZ, sigma@example.org";

    expect(beingTyped(line, 21)).toEqual({ start: 18, end: 24, typed: "kap" });
  });

  it("is nothing typed on an empty line, after a comma, or among spaces", () => {
    expect(beingTyped("", 0)).toEqual({ start: 0, end: 0, typed: "" });
    expect(beingTyped("zeta@example.com,", 17)).toEqual({
      start: 17,
      end: 17,
      typed: "",
    });
    expect(beingTyped("zeta@example.com,  ", 19).typed).toBe("");
  });

  it("is read where the caret can be, whatever it is told", () => {
    expect(beingTyped("kap", 99)).toEqual({ start: 0, end: 3, typed: "kap" });
    expect(beingTyped("kap", -4)).toEqual({ start: 0, end: 3, typed: "" });
  });
});

describe("a line with an address chosen", () => {
  it("puts the chosen address in place of what was typed and leaves room for the next", () => {
    expect(withChosen("zeta@example.com, kap", 21, "kappa@example.net")).toEqual(
      {
        written: "zeta@example.com, kappa@example.net, ",
        caret: 37,
      },
    );
  });

  it("replaces all of the address the caret stands in and leaves the others as they were", () => {
    const line = "zeta@example.com, kapXYZ, sigma@example.org";

    expect(withChosen(line, 21, "kappa@example.net")).toEqual({
      written: "zeta@example.com, kappa@example.net, sigma@example.org",
      caret: 35,
    });
  });

  it("keeps the spaces somebody typed before the address", () => {
    expect(withChosen("zeta@example.com,   kap", 23, "kappa@example.net")).toEqual({
      written: "zeta@example.com,   kappa@example.net, ",
      caret: 39,
    });
  });

  it("chooses into an empty line too", () => {
    expect(withChosen("", 0, "kappa@example.net")).toEqual({
      written: "kappa@example.net, ",
      caret: 19,
    });
  });

  it("is read back as the addresses it holds", () => {
    const { written } = withChosen("zeta@example.com, kap", 21, "kappa@example.net");

    expect(addressesIn(written)).toEqual(["zeta@example.com", "kappa@example.net"]);
    expect(everyOneAnAddress(written)).toBe(true);
  });
});
