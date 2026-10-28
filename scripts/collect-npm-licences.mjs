#!/usr/bin/env node
// Every npm package whose code travels inside the binary, with its licence and its notice.
//
// What ships is the production tree, as `npm ls --omit=dev` lists it, and the few build tools whose
// own output is compiled into the bundle, named below. Every permissive licence in it asks for the
// same thing — the notice travels with the code.
//
// Writes a JSON file the application reads. Committed, and compared against the tree by
// `scripts/preflight.sh`: a dependency added without its notice is a dependency shipped without its
// licence. `scripts/check-shipped-licences.mjs` then builds the window and holds every package in
// the bundle against this list, so a package this file fails to find is caught by something that
// does not share its way of looking.

import { execSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

/**
 * A notice for a package that publishes none, kept in this tree.
 *
 * `react-remove-scroll-bar` declares MIT and its published package carries no licence file; the
 * text kept is the one its own repository holds. The declared licence is matched as well, so a
 * package that changes its licence stops matching here rather than shipping under the old notice.
 */
const KEPT = {
  "react-remove-scroll-bar": {
    licence: "MIT",
    text: "src-tauri/licences-copied/react-remove-scroll-bar.md",
  },
};

/**
 * Build tools whose own output ships, which `--omit=dev` leaves out.
 *
 * `covers` says what of each is in the bundle. `sections` picks the parts of a notice that covers
 * more than ships: vite's carries the licence of every dependency it bundles for itself, and two
 * pieces of it reach the window — its own module preload polyfill, and a CommonJS helper from the
 * rollup plugins it bundles.
 */
const BUILT_INTO = [
  {
    name: "tailwindcss",
    covers: "its base styles, compiled into the window's stylesheet",
  },
  {
    name: "vite",
    covers: "its module preload polyfill and a CommonJS helper, in the window's script",
    sections: ["Vite core license", "@rollup/plugin-commonjs"],
  },
];

/**
 * Every package in the production tree, once each, where it really lives.
 *
 * `--parseable` prints one directory per installed package, wherever npm put it. The JSON form
 * prints a package's dependencies only where npm first meets it and a bare version everywhere
 * else, so a walk that stops at the first sighting of a name misses everything that hangs under the
 * one it skipped.
 */
const here = resolve(".");
const directories = execSync("npm ls --all --omit=dev --parseable", {
  maxBuffer: 64 * 1024 * 1024,
})
  .toString()
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line.length > 0 && resolve(line) !== here);

/** The name, version and declared licence a package's own manifest gives. */
function manifestOf(where) {
  const manifest = JSON.parse(readFileSync(join(where, "package.json"), "utf8"));
  const licence =
    typeof manifest.license === "string"
      ? manifest.license
      : (manifest.license?.type ?? manifest.licenses?.[0]?.type ?? null);
  return { name: manifest.name, version: manifest.version, licence };
}

/** The file a package keeps its notice in, under whichever of the usual names. */
const NOTICES = ["LICENSE", "LICENCE", "LICENSE.md", "LICENSE.txt", "COPYING", "NOTICE"];

/**
 * Every notice a package carries, whatever it called the files.
 *
 * A dual-licensed package ships both — `LICENSE_MIT` and `LICENSE_APACHE-2.0` — and a search for
 * the names above alone finds neither, which reads as a package with no licence at all. Both are
 * kept: the choice between them is the recipient's, and shipping one would be making it for them.
 */
function noticesIn(where) {
  const found = [];

  for (const name of readdirSync(where)) {
    const upper = name.toUpperCase();
    if (
      NOTICES.includes(upper) ||
      upper.startsWith("LICENSE") ||
      upper.startsWith("LICENCE") ||
      upper === "COPYING" ||
      upper === "NOTICE"
    ) {
      const text = readFileSync(join(where, name), "utf8").trim();
      if (text) found.push(text);
    }
  }

  return found.length > 0 ? found.join("\n\n") : null;
}

/** The kept notice for a package that publishes none, when its declared licence is the kept one. */
function keptFor(name, licence) {
  const kept = KEPT[name];
  if (kept === undefined || kept.licence !== licence || !existsSync(kept.text)) return null;
  return readFileSync(kept.text, "utf8").trim();
}

/**
 * The parts of a markdown notice whose headings name what ships, each with its heading.
 *
 * A heading this asks for and the notice no longer has fails the run: a notice that moved inside
 * the file is a notice that would ship missing.
 */
function sectionsOf(text, wanted) {
  const blocks = [];
  let current = null;
  for (const line of text.split("\n")) {
    if (/^#{1,6} /.test(line)) {
      current = { heading: line, lines: [line] };
      blocks.push(current);
    } else if (current) {
      current.lines.push(line);
    }
  }

  return wanted.map((one) => {
    const block = blocks.find((each) => each.heading.includes(one));
    if (block === undefined) return null;
    // The rule a notice draws between its sections belongs to the file, not to the section.
    return block.lines.join("\n").trim().replace(/\n-{3,}$/, "").trim();
  });
}

const packages = new Map();

for (const where of directories) {
  const { name, version, licence } = manifestOf(where);
  const key = `${name}@${version}`;
  if (packages.has(key)) continue;
  packages.set(key, {
    name,
    version,
    licence,
    notice: noticesIn(where) ?? keptFor(name, licence),
  });
}

for (const one of BUILT_INTO) {
  const where = join("node_modules", one.name);
  if (!existsSync(join(where, "package.json"))) {
    console.error(`${one.name} is named as shipping in its own output, and it is not installed.`);
    process.exit(1);
  }

  const { name, version, licence } = manifestOf(where);
  let notice = noticesIn(where);
  if (notice !== null && one.sections) {
    const parts = sectionsOf(notice, one.sections);
    notice = parts.includes(null) ? null : parts.join("\n\n");
  }
  packages.set(`${name}@${version}`, { name, version, licence, notice, covers: one.covers });
}

const listed = [...packages.values()].sort(
  (left, right) =>
    left.name.localeCompare(right.name) || left.version.localeCompare(right.version),
);

// A package whose licence nobody can read is a thing to fix rather than to ship past. Named, and
// the exit code says so, and `scripts/collect-licences.sh` stops there rather than merging a list
// with a hole in it.
const unknown = listed.filter((one) => one.licence === null || one.notice === null);

writeFileSync("src-tauri/licences-npm.json", JSON.stringify(listed, null, 2) + "\n");

console.log(`${listed.length} packages, ${listed.filter((o) => o.notice).length} with a notice`);

if (unknown.length > 0) {
  console.error(`no licence or no notice for: ${unknown.map((o) => o.name).join(", ")}`);
  process.exit(1);
}
