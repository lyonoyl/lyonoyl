import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The schemes the interface offers and the palettes the stylesheet defines have to be the same set.
 *
 * They are written in two places because they are two different things — a name to choose and a
 * set of colours to wear — and nothing but this holds them together. A scheme offered with no
 * palette leaves the window wearing the one before it, which reads as the click having done
 * nothing; a palette nobody can choose is dead weight nobody notices.
 */

const root = join(__dirname, "..", "..");
const CSS = readFileSync(join(root, "src", "index.css"), "utf8");
const SCREEN = readFileSync(
  join(root, "src", "features", "settings", "SettingsScreen.tsx"),
  "utf8",
);
const generated = (name: string) =>
  readFileSync(join(root, "src", "shared", "ipc", "generated", name), "utf8");

/** The schemes the core will accept, which is the list everything else is measured against. */
const namesIn = (dto: string) =>
  [...dto.matchAll(/"([A-Za-z]+)"/g)].map((found) => found[1]);

/**
 * Each side: the schemes the core accepts, the one that needs no attribute because it is the block
 * every palette of that side restates, and how a chosen one is named in the stylesheet.
 */
const SIDES = [
  {
    side: "dark",
    known: namesIn(generated("DarkSchemeDto.ts")),
    offered: 6,
    plain: "graphite",
    plainBlock: /^\.dark \{/m,
    selector: (scheme: string) => `.dark[data-scheme="${scheme}"] {`,
  },
  {
    side: "light",
    known: namesIn(generated("LightSchemeDto.ts")),
    offered: 5,
    plain: "paper",
    plainBlock: /^:root \{/m,
    selector: (scheme: string) =>
      `:root:not(.dark)[data-light-scheme="${scheme}"] {`,
  },
] as const;

/** The body of the first block that opens with `opening`. */
const blockAfter = (opening: string | RegExp) =>
  (typeof opening === "string"
    ? CSS.split(opening)[1]
    : CSS.split(opening)[1]
  )?.split("}")[0] ?? "";

/** The colours a block sets, by name. A size in the same block — the corners' radius — is not one. */
const tokens = (block: string) =>
  new Set(
    [...block.matchAll(/--([a-z-]+):\s*(?:#|oklch\()/g)].map((f) => f[1]),
  );

describe.each(SIDES)("the $side palettes", (side) => {
  it("are a set worth offering at all", () => {
    expect(side.known.length).toBe(side.offered);
    expect(side.known).toContain(side.plain);
  });

  it("each have colours in the stylesheet", () => {
    for (const scheme of side.known) {
      // The plain one is the side's own block — the palette this application has always worn, and
      // the one a person who never opened settings is wearing right now.
      const defined =
        scheme === side.plain
          ? side.plainBlock.test(CSS)
          : CSS.includes(side.selector(scheme));

      expect(defined, `${scheme} has no colours`).toBe(true);
    }
  });

  it("each state the whole palette rather than leaning on the one above", () => {
    // A token left out quietly takes the plain palette's value, and a colour belonging to no scheme
    // is the kind of thing nobody notices until a screenshot.
    const plain = tokens(blockAfter(side.plainBlock));
    expect(plain.size).toBeGreaterThan(20);

    for (const scheme of side.known.filter((name) => name !== side.plain)) {
      const block = tokens(blockAfter(side.selector(scheme)));
      const missing = [...plain].filter((token) => !block.has(token));

      expect(
        missing,
        `${scheme} leaves these to another scheme: ${missing.join(", ")}`,
      ).toEqual([]);
    }
  });

  it("are every one of them offered on the settings screen", () => {
    for (const scheme of side.known) {
      expect(SCREEN.includes(`"${scheme}"`), `${scheme} is not offered`).toBe(
        true,
      );
    }
  });
});

/**
 * How light a colour is, as WCAG measures it, from the two ways the stylesheet writes one: a hex
 * value, and the `oklch()` the plain palettes are written in.
 */
function luminance(colour: string): number {
  const linear = (channel: number) =>
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  const clamp = (value: number) => Math.min(1, Math.max(0, value));
  const weigh = ([r, g, b]: [number, number, number]) =>
    0.2126 * clamp(r) + 0.7152 * clamp(g) + 0.0722 * clamp(b);

  const hex = /^#([0-9a-f]{6})$/i.exec(colour);
  if (hex) {
    const [r, g, b] = [0, 2, 4].map(
      (at) => parseInt(hex[1].slice(at, at + 2), 16) / 255,
    );
    return weigh([linear(r), linear(g), linear(b)]);
  }

  const oklch = /^oklch\(([\d.]+) ([\d.]+) ([\d.]+)\)$/.exec(colour);
  if (!oklch) throw new Error(`not a colour this test reads: ${colour}`);
  const [lightness, chroma, hue] = oklch.slice(1).map(Number);
  const a = chroma * Math.cos((hue * Math.PI) / 180);
  const b = chroma * Math.sin((hue * Math.PI) / 180);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;

  return weigh([
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ]);
}

/** WCAG's contrast between two colours, from 1 to 21. */
function contrast(one: string, other: string): number {
  const [lighter, darker] = [luminance(one), luminance(other)].sort(
    (x, y) => y - x,
  );
  return (lighter + 0.05) / (darker + 0.05);
}

/** Every palette the stylesheet defines, by a name to report it under, as token → colour. */
function palettes(): Array<[string, Map<string, string>]> {
  const read = (block: string) =>
    new Map(
      [...block.matchAll(/--([a-z-]+):\s*([^;]+);/g)].map((f) => [
        f[1],
        f[2].trim(),
      ]),
    );
  const found: Array<[string, Map<string, string>]> = [
    ["paper", read(blockAfter(/^:root \{/m))],
    ["graphite", read(blockAfter(/^\.dark \{/m))],
  ];
  for (const side of SIDES) {
    for (const scheme of side.known.filter((name) => name !== side.plain)) {
      found.push([scheme, read(blockAfter(side.selector(scheme)))]);
    }
  }
  return found;
}

/** AA for text of an ordinary size. */
const READABLE = 4.5;

describe("the palettes keep text readable", () => {
  // The pairs text is read on everywhere: the page, cards and menus, a row lit by the pointer or
  // the selection, the column of mailboxes, and the quieter text beside it all.
  const READ_ON = [
    ["foreground", "background"],
    ["card-foreground", "card"],
    ["popover-foreground", "popover"],
    ["secondary-foreground", "secondary"],
    ["accent-foreground", "accent"],
    ["muted-foreground", "background"],
    ["sidebar-foreground", "sidebar"],
    ["sidebar-accent-foreground", "sidebar-accent"],
  ] as const;

  it.each(palettes())("%s", (_name, palette) => {
    for (const [text, ground] of READ_ON) {
      const measured = contrast(palette.get(text)!, palette.get(ground)!);
      expect(
        measured,
        `${text} on ${ground} is ${measured.toFixed(2)}`,
      ).toBeGreaterThanOrEqual(READABLE);
    }
  });

  // The chosen light palettes keep the rest as well: text on a button, quieter text on a lit row,
  // and an error said in red. Paper and the dark ones are kept as they have always been worn, and
  // as their projects publish them.
  const AND_ON_LIGHT = [
    ["primary-foreground", "primary"],
    ["sidebar-primary-foreground", "sidebar-primary"],
    ["muted-foreground", "muted"],
    ["destructive", "background"],
  ] as const;

  it.each(
    palettes().filter(
      ([name]) =>
        name !== SIDES[1].plain && SIDES[1].known.includes(name as never),
    ),
  )("%s, every pair", (_name, palette) => {
    for (const [text, ground] of AND_ON_LIGHT) {
      const measured = contrast(palette.get(text)!, palette.get(ground)!);
      expect(
        measured,
        `${text} on ${ground} is ${measured.toFixed(2)}`,
      ).toBeGreaterThanOrEqual(READABLE);
    }
  });
});
