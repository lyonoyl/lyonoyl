import { describe, expect, it } from "vitest";

import { signatureName, signed, standing, withSignatureOf } from "./signature";

const ANSWERED = "\n\nZeta Quorin wrote:\n> About Tuesday";
const KAPPA = "Kappa Vandel\nkappa@example.net";
const SIGMA = "Sigma Ordrin";

describe("signed", () => {
  it("puts the signature under the room to write, after the line that says it begins", () => {
    expect(signed("", KAPPA)).toBe("\n\n-- \nKappa Vandel\nkappa@example.net");
  });

  it("puts it over the message being answered", () => {
    expect(signed(ANSWERED, SIGMA)).toBe(
      "\n\n-- \nSigma Ordrin\n\nZeta Quorin wrote:\n> About Tuesday",
    );
  });

  it("leaves what the message opened with alone when there is no signature", () => {
    expect(signed("", "")).toBe("");
    expect(signed(ANSWERED, "")).toBe(ANSWERED);
  });

  it("adds no second line of dashes to a signature that opens with its own", () => {
    expect(signed("", "--\nSigma Ordrin")).toBe("\n\n--\nSigma Ordrin");
    expect(signed("", "-- \nSigma Ordrin")).toBe("\n\n-- \nSigma Ordrin");
  });

  it("does take a line that only begins with dashes for the person's own words", () => {
    expect(signed("", "--Sigma")).toBe("\n\n-- \n--Sigma");
  });
});

describe("withSignatureOf", () => {
  it("swaps one mailbox's signature for another's under what was written", () => {
    const body = `Hello${signed("", KAPPA)}`;

    expect(withSignatureOf(body, "", KAPPA, SIGMA)).toBe(
      "Hello\n\n-- \nSigma Ordrin",
    );
  });

  it("swaps it over the message being answered and keeps the quotation", () => {
    const body = `Thanks${signed(ANSWERED, KAPPA)}`;

    expect(withSignatureOf(body, ANSWERED, KAPPA, SIGMA)).toBe(
      "Thanks\n\n-- \nSigma Ordrin\n\nZeta Quorin wrote:\n> About Tuesday",
    );
  });

  it("takes the signature away for a mailbox that has none", () => {
    expect(withSignatureOf(`Thanks${signed(ANSWERED, KAPPA)}`, ANSWERED, KAPPA, "")).toBe(
      `Thanks${ANSWERED}`,
    );
    expect(withSignatureOf(signed("", KAPPA), "", KAPPA, "")).toBe("");
  });

  it("puts one in for a mailbox that has one where there was none", () => {
    expect(withSignatureOf(`Thanks${ANSWERED}`, ANSWERED, "", SIGMA)).toBe(
      `Thanks${signed(ANSWERED, SIGMA)}`,
    );
    expect(withSignatureOf("Hello", "", "", SIGMA)).toBe(
      "Hello\n\n-- \nSigma Ordrin",
    );
  });

  it("leaves a signature somebody changed alone", () => {
    const changed = "Hello\n\n-- \nKappa V.\nkappa@example.net";

    expect(withSignatureOf(changed, "", KAPPA, SIGMA)).toBe(changed);
  });

  it("leaves a message somebody wrote under the signature alone", () => {
    const under = `Hello${signed("", KAPPA)}\nP.S. Bring the plans`;

    expect(withSignatureOf(under, "", KAPPA, SIGMA)).toBe(under);
  });

  it("leaves a message whose quotation was edited alone", () => {
    const edited = `Thanks${signed("\n\nZeta Quorin wrote:\n> About", KAPPA)}`;

    expect(withSignatureOf(edited, ANSWERED, KAPPA, SIGMA)).toBe(edited);
  });
});

describe("standing", () => {
  it("is signed while the signature stands where it was put", () => {
    expect(standing(`Hello${signed("", KAPPA)}`, "", KAPPA)).toBe("signed");
    expect(standing(`Thanks${signed(ANSWERED, KAPPA)}`, ANSWERED, KAPPA)).toBe(
      "signed",
    );
  });

  it("is unsigned where no line of dashes stands over what the message opened with", () => {
    expect(standing("Hello", "", KAPPA)).toBe("unsigned");
    expect(standing(`Thanks${ANSWERED}`, ANSWERED, KAPPA)).toBe("unsigned");
    expect(standing("", "", "")).toBe("unsigned");
  });

  it("is changed once the signature was edited, written under, or written by hand", () => {
    expect(standing("Hello\n\n-- \nKappa V.", "", KAPPA)).toBe("changed");
    expect(
      standing(`Hello${signed("", KAPPA)}\nP.S. Bring the plans`, "", KAPPA),
    ).toBe("changed");
    expect(standing("Hello\n\n--\nMy own", "", "")).toBe("changed");
  });

  it("is changed once the quotation under it was edited", () => {
    expect(
      standing(`Thanks${signed("\n\nZeta Quorin wrote:", KAPPA)}`, ANSWERED, KAPPA),
    ).toBe("changed");
  });
});

describe("withSignatureOf, where nothing was signed", () => {
  it("puts a signature back into a message it was taken out of", () => {
    expect(withSignatureOf("Hello", "", KAPPA, KAPPA)).toBe(
      `Hello${signed("", KAPPA)}`,
    );
  });

  it("adds no second signature under one written by hand", () => {
    const own = "Hello\n\n-- \nMy own";

    expect(withSignatureOf(own, "", "", SIGMA)).toBe(own);
  });
});

describe("signatureName", () => {
  it("is the first line with something on it besides the dashes", () => {
    expect(signatureName(KAPPA)).toBe("Kappa Vandel");
    expect(signatureName("--\n\n  Sigma Ordrin  \nOffice")).toBe("Sigma Ordrin");
    expect(signatureName("")).toBe("");
  });
});
