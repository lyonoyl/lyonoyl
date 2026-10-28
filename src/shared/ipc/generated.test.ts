import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The generated types describe what actually crosses the bridge, and the bridge is JSON.
 * A type that JSON cannot deliver compiles but breaks at runtime.
 */

const GENERATED = join(__dirname, "generated");

const files = readdirSync(GENERATED).filter((file) => file.endsWith(".ts"));

describe("generated DTOs", () => {
  it("has files to check", () => {
    expect(files.length).toBeGreaterThan(0);
  });

  it.each(files)("%s never declares bigint", (file) => {
    const source = readFileSync(join(GENERATED, file), "utf8");
    const declarations = source
      .split("\n")
      .filter(
        (line) => !line.trim().startsWith("*") && line.includes("bigint"),
      );

    // JSON.parse produces a number for every Rust integer, so a bigint here is a type that lies.
    // A 64-bit value that really needs the range crosses as a string instead.
    expect(declarations).toEqual([]);
  });
});
