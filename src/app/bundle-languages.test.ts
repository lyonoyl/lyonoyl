import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The languages the macOS bundle declares are the catalogues' own, and nothing else.
 *
 * The menu bar is made in the window's words, and macOS still writes a few things in it by itself —
 * Start Dictation and Emoji & Symbols in the Edit menu, the full-screen item's title, the Window and
 * Help menus' own — along with the panels it draws. It writes them in a language the bundle
 * declares and the system runs in: declaring none left them English whatever the window spoke.
 * Tauri merges `src-tauri/Info.plist` into the bundle's own, and embeds it in a development build.
 */

const root = join(__dirname, "..", "..");

describe("the languages the bundle declares", () => {
  it("are every catalogue's, and no other", () => {
    const plist = readFileSync(join(root, "src-tauri/Info.plist"), "utf8");
    const list = /<key>CFBundleLocalizations<\/key>\s*<array>([\s\S]*?)<\/array>/.exec(plist);
    const declared = [...(list?.[1] ?? "").matchAll(/<string>([^<]+)<\/string>/g)]
      .map((found) => found[1])
      .sort();
    const catalogues = readdirSync(join(root, "src/shared/i18n/locales"))
      .filter((file) => file.endsWith(".json"))
      .map((file) => file.replace(/\.json$/, ""))
      .sort();

    expect(list, "no CFBundleLocalizations in src-tauri/Info.plist").not.toBeNull();
    expect(declared).toEqual(catalogues);
  });
});
