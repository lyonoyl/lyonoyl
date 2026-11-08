import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The disk image opens on a window laid out to show where to drag the application: a picture, the
 * window's size, and where the two icons stand. Finder draws the picture at the window's size, from
 * the page made for the screen it draws on—an ordinary screen draws a point with one pixel, a Retina
 * screen with two each way. A picture without the second page is blurred on Retina, and a page of
 * another size does not fit the window.
 */

const root = join(__dirname, "..", "..");
const tauri = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8")) as {
  bundle: { macOS: { dmg: { background: string; windowSize: { width: number; height: number } } } };
};
const dmg = tauri.bundle.macOS.dmg;
const place = { x: expect.any(Number), y: expect.any(Number) };

/** Each page of a TIFF file: its size in points, and how many pixels a point takes each way. */
function pages(file: Buffer): { width: number; height: number; density: number }[] {
  const view = new DataView(file.buffer, file.byteOffset, file.byteLength);
  const order = file.toString("latin1", 0, 2);
  const little = order === "II";
  const short = (at: number) => view.getUint16(at, little);
  const long = (at: number) => view.getUint32(at, little);
  if ((order !== "II" && order !== "MM") || short(2) !== 42) throw new Error("not a TIFF file");

  const found = [];
  for (let directory = long(4); directory !== 0; directory = long(directory + 2 + short(directory) * 12)) {
    const entries = new Map<number, number>();
    for (let index = 0; index < short(directory); index += 1) {
      const entry = directory + 2 + index * 12;
      entries.set(short(entry), entry);
    }
    // Only the five tags read here are decoded: a size is a short or a long kept in the entry, a
    // resolution a fraction kept where the entry points.
    const value = (tag: number) => {
      const entry = entries.get(tag);
      if (entry === undefined) return undefined;
      const type = short(entry + 2);
      if (type === 3) return short(entry + 8);
      if (type === 4) return long(entry + 8);
      const at = long(entry + 8);
      return long(at) / long(at + 4);
    };
    if ((value(296) ?? 2) !== 2) throw new Error("a resolution not in pixels per inch");
    // A page that gives no resolution is drawn at 72 pixels per inch, a pixel for each point.
    const horizontal = value(282) ?? 72;
    const vertical = value(283) ?? 72;
    found.push({
      width: ((value(256) ?? 0) * 72) / horizontal,
      height: ((value(257) ?? 0) * 72) / vertical,
      density: horizontal / 72,
    });
  }
  return found;
}

describe("disk image window", () => {
  it("is laid out with a picture kept in the tree, a size, and a place for each icon", () => {
    expect(dmg).toMatchObject({
      background: expect.any(String),
      windowSize: { width: expect.any(Number), height: expect.any(Number) },
      appPosition: place,
      applicationFolderPosition: place,
    });
    expect(existsSync(join(root, "src-tauri", dmg.background))).toBe(true);
  });

  it("has a page of the window's size for an ordinary screen and one for a Retina screen", () => {
    const { width, height } = dmg.windowSize;
    const picture = pages(readFileSync(join(root, "src-tauri", dmg.background)));
    expect(picture.sort((one, other) => one.density - other.density)).toEqual([
      { width, height, density: 1 },
      { width, height, density: 2 },
    ]);
  });
});
