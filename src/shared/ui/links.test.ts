import { describe, expect, it } from "vitest";

import shared from "../../../tests/fixtures/links-in-text.json";
import { linksIn } from "./links";

describe("linksIn", () => {
  it("finds every shared example where the core finds it", () => {
    // The core links these in plain-text mail, in the markup it sends beside a message's text, and
    // above a message passed on in its own form; the composer marks them as they are typed, and a
    // message sent from here links them before its copy is read back. Answering the same examples
    // is what keeps a mark from promising a link the message will not have.
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

  it("costs what the text's length costs, made of almost names", () => {
    // A name is read once, from where one can start, and never from inside one read already.
    const chunk = `${"a,".repeat(63)}${")".repeat(128)} `;
    for (const text of [
      "a)".repeat(1 << 19),
      "a'".repeat(1 << 19),
      chunk.repeat(Math.floor((1 << 20) / chunk.length)),
      "a@".repeat(1 << 19),
      "a.".repeat(1 << 19),
      `a@${"b".repeat(1 << 20)}`,
      `${"a.".repeat(1 << 19)}@`,
    ]) {
      const began = performance.now();

      expect(linksIn(text), text.slice(0, 8)).toEqual([]);
      expect(performance.now() - began, text.slice(0, 8)).toBeLessThan(2000);
    }
  });

  it("costs what the text's length costs, with a run of marks after an address", () => {
    // The brackets an address holds are counted once, and nothing is copied as its end moves back.
    // Counted or copied again for every mark left behind, a megabyte of them would take hours.
    for (const mark of [")", "]", ".", "?"]) {
      const text = `https://example.com/${mark.repeat(1 << 20)}`;
      const began = performance.now();

      expect(linksIn(text), mark).toEqual([
        { start: 0, end: "https://example.com/".length },
      ]);
      expect(performance.now() - began, mark).toBeLessThan(2000);
    }
  });

  it("costs what the text's length costs, made of beginnings that open nothing", () => {
    // A beginning that opens nothing is read to where its stretch stops and handed to the parser,
    // so a stretch is read for four of them and no more. Read for every one, a megabyte of them
    // would take hours.
    for (const beginning of ["www.", "http://", "https://", "mailto:"]) {
      const text = `${beginning}%)`.repeat(
        Math.floor((1 << 20) / (beginning.length + 2)),
      );
      const began = performance.now();

      expect(linksIn(text), beginning).toEqual([]);
      expect(performance.now() - began, beginning).toBeLessThan(2000);
    }
  });
});
