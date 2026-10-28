import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * Markdown here is written for the screen that wraps it: a paragraph, a list item and a quote are each
 * one line, and the dash in a sentence is closed, word—word. The licenses copied into the tree are
 * their authors' words, and they lie where nothing below walks.
 */

const root = join(__dirname, "..", "..");

/** Every Markdown file at the top of the repository and under the directories that hold prose. */
function documents(): string[] {
  const found = readdirSync(root).filter((entry) => entry.endsWith(".md"));
  const walk = (directory: string) => {
    for (const entry of readdirSync(join(root, directory))) {
      if (entry === "node_modules") continue;
      const path = `${directory}/${entry}`;
      if (statSync(join(root, path)).isDirectory()) walk(path);
      else if (entry.endsWith(".md")) found.push(path);
    }
  };
  for (const directory of [".github", "docs", "src", "tests"]) walk(directory);
  return found;
}

const FENCE = /^\s*(```|~~~)/;
/** Lines that are not prose: a table row, a heading, markup, a picture, a badge, a link's definition. */
const STRUCTURE = /^\s*(\||#|<|!\[|\[!\[|\[[^\]]+\]:)/;
const ITEM = /^\s*([-*+]|\d+[.)])\s+/;

/** Where a line of prose carries on the one above it, as `path:line`. */
function brokenParagraphs(path: string): string[] {
  const broken: string[] = [];
  let fenced = false;
  let markup = false;
  let previous: "blank" | "prose" | "structure" = "blank";
  readFileSync(join(root, path), "utf8")
    .split("\n")
    .forEach((line, index) => {
      if (FENCE.test(line)) {
        fenced = !fenced;
        previous = "structure";
        return;
      }
      if (fenced) return;
      if (!line.trim()) {
        previous = "blank";
        markup = false;
        return;
      }
      if (markup || (line.trimStart().startsWith("<") && previous === "blank")) {
        markup = true;
        previous = "structure";
        return;
      }
      if (STRUCTURE.test(line)) {
        previous = "structure";
        return;
      }
      if (!ITEM.test(line) && previous === "prose") broken.push(`${path}:${index + 1}`);
      previous = "prose";
    });
  return broken;
}

/** Where a dash between two words has a space on either side, outside code, as `path:line`. */
function spacedDashes(path: string): string[] {
  const spaced: string[] = [];
  let fenced = false;
  readFileSync(join(root, path), "utf8")
    .split("\n")
    .forEach((line, index) => {
      if (FENCE.test(line)) {
        fenced = !fenced;
        return;
      }
      if (fenced) return;
      if (/[^\s|] — [^\s|]/.test(line.replace(/`[^`]*`/g, "\u0000"))) spaced.push(`${path}:${index + 1}`);
    });
  return spaced;
}

describe("Markdown", () => {
  it("is found where the prose of the repository lies", () => {
    expect(documents()).toEqual(
      expect.arrayContaining(["AGENTS.md", "README.md", "docs/development/guide.md", "tests/fixtures/eml/README.md"]),
    );
  });

  it("writes a paragraph, a list item and a quote on one line", () => {
    expect(documents().flatMap(brokenParagraphs)).toEqual([]);
  });

  it("closes the dash in a sentence", () => {
    expect(documents().flatMap(spacedDashes)).toEqual([]);
  });
});
