import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The shop window, held to the rules in AGENTS.md section 7.
 *
 * The rules themselves live in `scripts/check-readme.py`, which is what `lint:layers` runs;
 * this asks that script the question so a person working on the interface finds out from `npm test`
 * rather than from a pull request. It also pins the two things the script deliberately leaves
 * alone: that the file is the product's front page and not a document about the repository.
 */

const root = join(__dirname, "..", "..");
const README = readFileSync(join(root, "README.md"), "utf8");

describe("the README", () => {
  it("obeys every rule a script can check", () => {
    const said = execFileSync("python3", ["scripts/check-readme.py"], {
      cwd: root,
      encoding: "utf8",
    }).trim();

    expect(said).toBe("");
  });

  it("says what the product is before it says anything else", () => {
    // Nothing but markup — the icon, the block that centres the header — comes before the title, and
    // the first thing under the title is the promise, in one paragraph. A reader who has to scroll to
    // find out what this is has already gone.
    const lines = README.split("\n");
    const title = lines.findIndex((line) => line.startsWith("# "));
    const beforeTitle = lines.slice(0, title);
    const afterTitle =
      lines.slice(title + 1).find((line) => line.trim() !== "") ?? "";

    expect(title).toBeGreaterThanOrEqual(0);
    expect(
      beforeTitle.every((line) => line.trim() === "" || line.trim().startsWith("<")),
    ).toBe(true);
    expect(afterTitle.startsWith("**")).toBe(true);
    expect(README.indexOf("**")).toBeLessThan(README.indexOf("\n## "));
  });

  it("names as many themes as the product has", () => {
    // Counted from the colour schemes the core accepts, dark and light, so a scheme added or
    // removed fails here until the line under the screenshots says so.
    const schemes = ["DarkSchemeDto", "LightSchemeDto"].flatMap((type) => {
      const source = readFileSync(
        join(root, "src", "shared", "ipc", "generated", `${type}.ts`),
        "utf8",
      );
      return source.match(/^export type \w+ = (.+);$/m)?.[1]?.split("|") ?? [];
    });
    const named = [...README.matchAll(/(\d+) themes\b/gi)].map((found) =>
      Number(found[1]),
    );

    expect(named).toEqual(named.map(() => schemes.length));
  });

  it("keeps no table of contents, because GitHub draws its own", () => {
    expect(README).not.toContain("## Contents");
    expect(README).not.toContain("## Table of contents");
  });

  it("sends a developer to the documents rather than answering them here", () => {
    // Every command a contributor runs lives in one place, and the README's job is to point at it.
    for (const developerOnly of [
      "cargo clippy",
      "cargo fmt",
      "npm run typecheck",
      "check-layering",
      "--nocapture",
    ]) {
      expect(README).not.toContain(developerOnly);
    }
    expect(README).toContain("docs/development/guide.md");
  });
});
