import { describe, expect, it } from "vitest";

import { typedDirection } from "./direction";

describe("the direction of a field somebody types into", () => {
  it("is the window's while the field is empty", () => {
    expect(typedDirection("")).toBeUndefined();
  });

  it("is the window's while nothing typed is a letter", () => {
    // Digits and punctuation take their direction from what is around them.
    expect(typedDirection("2026 ...")).toBeUndefined();
  });

  it("is the letters' own once there is one", () => {
    expect(typedDirection("Re: Lunch tomorrow?")).toBe("auto");
    expect(typedDirection("\u0645")).toBe("auto");
    expect(typedDirection("\u4f1a\u8bae")).toBe("auto");
  });
});
