import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The version is written in three places — the workspace manifest and the two files that build the
 * app — and these keep them from disagreeing. A document names a version only in the notes of a
 * release that shipped, so none of them names the version being built, and none draws a road of
 * versions towards anything.
 */

const root = join(__dirname, "..", "..");
const read = (path: string) => readFileSync(join(root, path), "utf8");

const cargo = read("Cargo.toml");
const manifest = JSON.parse(read("package.json")) as { version: string };
const tauri = JSON.parse(read("src-tauri/tauri.conf.json")) as {
  version: string;
};

/** The one place the number is decided. */
const declared =
  /\[workspace\.package\][\s\S]*?version = "(\d+\.\d+\.\d+)"/.exec(cargo)?.[1] ?? "";

/** Every markdown file at the top of the repository and under `docs/`. */
function documents(): string[] {
  const found = readdirSync(root).filter((entry) => entry.endsWith(".md"));
  const walk = (directory: string) => {
    for (const entry of readdirSync(join(root, directory))) {
      const path = `${directory}/${entry}`;
      if (statSync(join(root, path)).isDirectory()) walk(path);
      else if (entry.endsWith(".md")) found.push(path);
    }
  };
  walk("docs");
  return found;
}

describe("release version", () => {
  it("is decided in one place", () => {
    expect(declared).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("is the same everywhere it is written", () => {
    expect(manifest.version).toBe(declared);
    expect(tauri.version).toBe(declared);
  });

  it("is named by no document", () => {
    for (const path of documents()) {
      expect(`${path}: ${read(path).includes(declared)}`).toBe(`${path}: false`);
    }
  });

  it("is not a step on a road any document draws", () => {
    for (const path of documents()) {
      const steps = read(path).match(/^\|\s*(✅|▶|○)\s*\|\s*\*\*\d+\.\d+\*\*/gm) ?? [];
      expect(`${path}: ${steps.length}`).toBe(`${path}: 0`);
    }
  });
});
