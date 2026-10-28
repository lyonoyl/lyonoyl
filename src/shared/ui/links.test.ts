import { describe, expect, it } from "vitest";

import shared from "../../../tests/fixtures/links-in-text.json";
import { linksIn } from "./links";

describe("linksIn", () => {
  it("finds every shared example where the core finds it", () => {
    // The core links these in plain-text mail; the composer marks them as they are typed. Answering
    // the same examples is what keeps a mark from promising a link the message will not have.
    expect(shared.cases.length).toBeGreaterThan(20);

    for (const example of shared.cases) {
      const written = linksIn(example.text).map(({ start, end }) =>
        example.text.slice(start, end),
      );
      expect(written, example.text).toEqual(
        example.links.map((link) => link.written),
      );
    }
  });

  it("finds nothing in nothing", () => {
    expect(linksIn("")).toEqual([]);
    expect(linksIn(" \n\t")).toEqual([]);
  });

  it("counts in the string's own units, so a slice is the address as written", () => {
    // A letter outside the first plane takes two, and an address after one starts two along.
    const text = "\u{1F600} https://example.com/a";

    expect(linksIn(text)).toEqual([{ start: 3, end: text.length }]);
  });

  it("keeps its place past a letter whose lower case is longer", () => {
    // Lower-cased whole, this text would grow by one under the capital I with a dot, and every
    // position after it would point one place late.
    const text = "İ https://example.com/a";

    const [link] = linksIn(text);
    expect(text.slice(link!.start, link!.end)).toBe("https://example.com/a");
  });

  it("costs what the text's length costs, made of almost addresses", () => {
    const text = "xwww.".repeat(100_000);
    const began = performance.now();

    expect(linksIn(text)).toEqual([]);
    expect(performance.now() - began).toBeLessThan(2000);
  });
});
