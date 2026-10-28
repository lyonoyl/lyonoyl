// @vitest-environment node
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * What the window may ask the core for, held against what it does ask for.
 *
 * A capability is the window's own reach: a permission granted there is a call a compromised
 * renderer makes without passing through anything this application wrote. A default set is a list
 * somebody else wrote and extends without asking — the core's own reaches menus, the home
 * directory, the windows and webviews, images and the tray — so the grants are named one by one,
 * and they are exactly what the imports below need. An import with no entry here fails until
 * somebody decides what it needs and grants that, and a grant nothing imports fails as well.
 */

const root = join(__dirname, "..", "..", "..");
const CAPABILITIES = join(root, "src-tauri", "capabilities");
const SOURCES = join(root, "src");

/** What each thing the window takes from Tauri asks the core for. */
const NEEDS: Record<string, string[]> = {
  "@tauri-apps/api/app getVersion": ["core:app:allow-version"],
  // The application's own commands are not gated by a capability.
  "@tauri-apps/api/core invoke": [],
  // Listening, and the function it answers with, which stops listening.
  "@tauri-apps/api/event listen": [
    "core:event:allow-listen",
    "core:event:allow-unlisten",
  ],
};

/** Every source file of the window, its tests left out: a test runs in Node, not in the window. */
function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);

    if (entry.isDirectory()) return sourceFiles(path);
    if (!/\.(ts|tsx)$/.test(entry.name)) return [];
    if (/\.test\.(ts|tsx)$/.test(entry.name)) return [];

    return [path];
  });
}

const NAMED =
  /import\s+(type\s+)?\{([^}]*)\}\s*from\s*["'](@tauri-apps\/[^"']+)["']/g;
const MENTIONED = /["'](@tauri-apps\/[^"']+)["']/g;

/**
 * What one file takes from Tauri, as `module name`.
 *
 * Only a named import says what is used. A module reached any other way — whole, by its default,
 * loaded on demand, passed on — is written as `module *`, which no entry matches.
 */
function takenFromTauri(source: string): string[] {
  const named = [...source.matchAll(NAMED)];
  const taken = named.flatMap(([, onlyTypes, names = "", module]) =>
    onlyTypes
      ? []
      : names
          .split(",")
          .map((entry) => entry.trim().split(/\s+as\s+/)[0]?.trim() ?? "")
          .filter((name) => name !== "" && !name.startsWith("type "))
          .map((name) => `${module} ${name}`),
  );

  // How many times a module is named by a match of one pattern; its own name is the last group.
  const times = (pattern: RegExp, module: string) =>
    [...source.matchAll(pattern)].filter(
      (match) => match[match.length - 1] === module,
    ).length;

  const otherwise = [
    ...new Set([...source.matchAll(MENTIONED)].map(([, module = ""]) => module)),
  ]
    .filter((module) => times(MENTIONED, module) > times(NAMED, module))
    .map((module) => `${module} *`);

  return [...taken, ...otherwise];
}

/** What the capabilities grant the main window, by identifier. */
function grantedToTheWindow(): string[] {
  const granted = readdirSync(CAPABILITIES)
    .filter((file) => file.endsWith(".json"))
    .map(
      (file) =>
        JSON.parse(readFileSync(join(CAPABILITIES, file), "utf8")) as {
          windows?: string[];
          permissions?: (string | { identifier: string })[];
        },
    )
    .filter(({ windows = [] }) =>
      windows.some((window) => window === "main" || window === "*"),
    )
    .flatMap(({ permissions = [] }) =>
      permissions.map((permission) =>
        typeof permission === "string" ? permission : permission.identifier,
      ),
    );

  return [...new Set(granted)].sort();
}

describe("what the window may ask the core for", () => {
  const taken = [
    ...new Set(
      sourceFiles(SOURCES).flatMap((file) =>
        takenFromTauri(readFileSync(file, "utf8")),
      ),
    ),
  ].sort();

  it("knows what everything the window takes from Tauri needs", () => {
    expect(taken).toEqual(Object.keys(NEEDS).sort());
  });

  it("grants the window exactly that, and nothing by default", () => {
    const needed = [
      ...new Set(taken.flatMap((thing) => NEEDS[thing] ?? [])),
    ].sort();

    expect(grantedToTheWindow()).toEqual(needed);
  });

  it("reads an import the table does not know as one it does not know", () => {
    // The two halves above are only as good as this reading: a window that reached Tauri in a
    // way it cannot see would pass both with nothing granted for it.
    expect(
      takenFromTauri(
        [
          'import { getCurrentWindow as here, type Window } from "@tauri-apps/api/window";',
          'import type { UnlistenFn } from "@tauri-apps/api/event";',
          'import * as path from "@tauri-apps/api/path";',
          'const opener = await import("@tauri-apps/plugin-opener");',
        ].join("\n"),
      ),
    ).toEqual([
      "@tauri-apps/api/window getCurrentWindow",
      "@tauri-apps/api/path *",
      "@tauri-apps/plugin-opener *",
    ]);
  });
});
