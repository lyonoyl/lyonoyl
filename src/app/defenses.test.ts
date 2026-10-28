import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * `docs/security/defenses.md` lists every defense beside what holds it. A row names Rust tests by
 * their function names and anything else by its path, and every name must still be in the tree: a
 * test renamed or removed without its row changing fails here.
 */

const root = join(__dirname, "..", "..");
const document = readFileSync(join(root, "docs/security/defenses.md"), "utf8");

/** The last cell of every row of every table, the header and the rule under it left out. */
const heldBy = document
  .split("\n")
  .filter((line) => line.startsWith("| ") && !line.startsWith("| ---") && !line.startsWith("| Defense |"))
  .map((line) => {
    const cells = line.split(" | ");
    return { row: line, cell: cells[cells.length - 1].replace(/\s*\|\s*$/, "") };
  });

/** Every Rust source the tests could live in. */
function rustSources(directory: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(join(root, directory))) {
    const path = `${directory}/${entry}`;
    if (statSync(join(root, path)).isDirectory()) found.push(...rustSources(path));
    else if (entry.endsWith(".rs")) found.push(readFileSync(join(root, path), "utf8"));
  }
  return found;
}
const rust = [...rustSources("src-tauri/src"), ...rustSources("crates")];

/** A test function by that name, under a test attribute. */
function isATest(name: string): boolean {
  const declared = new RegExp(`fn ${name}\\s*\\(`);
  return rust.some((source) => {
    const at = source.search(declared);
    return at >= 0 && /#\[(?:tokio::)?test\b/.test(source.slice(Math.max(0, at - 300), at));
  });
}

describe("the defenses document", () => {
  it("names what holds every defense it lists", () => {
    expect(heldBy.length).toBeGreaterThan(0);
    for (const { row, cell } of heldBy) {
      expect(`${row}: ${/`[^`]+`/.test(cell)}`).toBe(`${row}: true`);
    }
  });

  it("names only tests and files the tree holds", () => {
    for (const { cell } of heldBy) {
      for (const [, name] of cell.matchAll(/`([^`]+)`/g)) {
        const held = name.includes("/") || name.includes(".") ? existsSync(join(root, name)) : isATest(name);
        expect(`${name}: ${held}`).toBe(`${name}: true`);
      }
    }
  });
});
