import { describe, expect, it } from "vitest";

import {
  addedBy,
  itemAt,
  placeOf,
  rowsOnScreen,
  type Unfolded,
} from "./places";

/**
 * The arithmetic the list, the virtualizer and the keyboard all have to agree on.
 *
 * Two open conversations rather than one throughout: with a single one the running total is only
 * ever zero when it is read, so a lost accumulator would pass every case.
 */

/** Conversations open on rows 2 and 5, adding three messages and two. */
const TWO: Unfolded[] = [
  { index: 2, messages: 3 },
  { index: 5, messages: 2 },
];

describe("places", () => {
  it("counts the places the open conversations add", () => {
    expect(addedBy([])).toBe(0);
    expect(addedBy(TWO)).toBe(5);
  });

  it("leaves every place a row of the list when nothing is open", () => {
    for (const place of [0, 1, 7, 99_999]) {
      expect(itemAt(place, [])).toEqual({ index: place });
      expect(placeOf(place, [])).toBe(place);
    }
  });

  it("leaves the rows above an open conversation where they were", () => {
    expect(itemAt(0, TWO)).toEqual({ index: 0 });
    expect(itemAt(1, TWO)).toEqual({ index: 1 });
    expect(placeOf(0, TWO)).toBe(0);
    expect(placeOf(1, TWO)).toBe(1);
  });

  it("leaves the row a conversation hangs under where it was", () => {
    expect(itemAt(2, TWO)).toEqual({ index: 2 });
    expect(placeOf(2, TWO)).toBe(2);
  });

  it("hangs a conversation's messages under its row, in order", () => {
    expect(itemAt(3, TWO)).toEqual({ index: 2, at: 0 });
    expect(itemAt(4, TWO)).toEqual({ index: 2, at: 1 });
    expect(itemAt(5, TWO)).toEqual({ index: 2, at: 2 });
  });

  it("moves the rows under an open conversation down by what it added", () => {
    expect(itemAt(6, TWO)).toEqual({ index: 3 });
    expect(itemAt(7, TWO)).toEqual({ index: 4 });
    expect(placeOf(3, TWO)).toBe(6);
    expect(placeOf(4, TWO)).toBe(7);
  });

  it("counts both conversations for a row under the second one", () => {
    // Row 5 is moved down by the three the first one added, and not by its own two.
    expect(itemAt(8, TWO)).toEqual({ index: 5 });
    expect(placeOf(5, TWO)).toBe(8);

    expect(itemAt(9, TWO)).toEqual({ index: 5, at: 0 });
    expect(itemAt(10, TWO)).toEqual({ index: 5, at: 1 });

    // And rows below both are moved by all five.
    expect(itemAt(11, TWO)).toEqual({ index: 6 });
    expect(placeOf(6, TWO)).toBe(11);
  });

  it("reads a row's place and reads it back to the same row", () => {
    for (let index = 0; index < 40; index += 1) {
      const place = placeOf(index, TWO);
      expect(itemAt(place, TWO)).toEqual({ index });
    }
  });

  it("gives every place exactly one thing, and misses none", () => {
    const seen = new Set<string>();
    for (let place = 0; place < 20; place += 1) {
      const { index, at } = itemAt(place, TWO);
      seen.add(at === undefined ? `row-${index}` : `inside-${index}-${at}`);
    }
    expect(seen.size).toBe(20);
    // The twenty places are fifteen rows and the five messages hanging under two of them.
    expect([...seen].filter((one) => one.startsWith("inside-")).sort()).toEqual(
      ["inside-2-0", "inside-2-1", "inside-2-2", "inside-5-0", "inside-5-1"],
    );
  });

  it("hangs a conversation opened on the first row under it", () => {
    const first: Unfolded[] = [{ index: 0, messages: 2 }];
    expect(itemAt(0, first)).toEqual({ index: 0 });
    expect(itemAt(1, first)).toEqual({ index: 0, at: 0 });
    expect(itemAt(2, first)).toEqual({ index: 0, at: 1 });
    expect(itemAt(3, first)).toEqual({ index: 1 });
    expect(placeOf(0, first)).toBe(0);
    expect(placeOf(1, first)).toBe(3);
  });

  it("hangs nothing under a conversation the rows say holds nothing", () => {
    // A conversation opened before its messages have arrived. Its row keeps its place and every
    // row under it stays where it was, rather than a gap opening under it.
    const waiting: Unfolded[] = [{ index: 1, messages: 0 }];
    expect(addedBy(waiting)).toBe(0);
    expect(itemAt(1, waiting)).toEqual({ index: 1 });
    expect(itemAt(2, waiting)).toEqual({ index: 2 });
    expect(placeOf(2, waiting)).toBe(2);
  });
});

describe("the rows on the screen", () => {
  const LIST = { places: 100, height: 100, gutter: 6 };

  it("are the rows wholly on it, from the top and from the bottom", () => {
    // Six hundred pixels from the top, after the gap above the first row: rows 0 to 4 are whole,
    // and the sixth is cut by the edge.
    expect(rowsOnScreen(0, 600, LIST, [])).toEqual({
      top: 0,
      first: 0,
      last: 4,
    });
  });

  it("start at the first row whose top is on the screen once scrolled", () => {
    // Scrolled to the middle of row 2: it is at the top, and row 3 is the first one whole.
    expect(rowsOnScreen(256, 600, LIST, [])).toEqual({
      top: 2,
      first: 3,
      last: 7,
    });
  });

  it("read a list scrolled back to a row as scrolled to that row, visit after visit", () => {
    // Scrolled back with the gutter above its top row, the offset is `top * height`. Read as the
    // row the top of the screen cuts through, that is the row above, and each return to the
    // mailbox would put the list a row higher than the last.
    for (const top of [1, 5, 99]) {
      const restored = top * LIST.height;
      expect(rowsOnScreen(restored, 600, LIST, []).top).toBe(top);
    }
  });

  it("read the top as the row whose top is nearest it, however it was scrolled", () => {
    // Twelve pixels of row 5 are left on the screen: coming back puts row 6 at the top rather than
    // pulling the list back almost a whole row.
    expect(rowsOnScreen(594, 600, LIST, []).top).toBe(6);
  });

  it("count a conversation opened above as the row it hangs under", () => {
    // Three messages open under row 1 take places 2 to 4, so the place on screen at the top is a
    // message of that conversation and the row it stands under is where a key starts.
    const open: Unfolded[] = [{ index: 1, messages: 3 }];
    expect(rowsOnScreen(306, 600, LIST, open).first).toBe(1);
  });

  it("stay inside a list shorter than the screen", () => {
    expect(rowsOnScreen(0, 600, { ...LIST, places: 2 }, [])).toEqual({
      top: 0,
      first: 0,
      last: 1,
    });
  });
});
