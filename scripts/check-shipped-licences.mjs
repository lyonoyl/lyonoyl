#!/usr/bin/env node
// Every npm package in the built window has its notice in the list the application shows.
//
// `scripts/collect-npm-licences.mjs` decides what ships from what npm installed. This builds the
// window and reads what is actually in it, so a package the collector fails to find, or one that
// ships without being a production dependency, fails here instead of shipping without its notice.
// Built in memory with the project's own configuration: nothing is written, in the repository or
// out of it.
//
// Three kinds of thing ship, so the bundle is read three ways: the script's modules, where a module
// whose code was all shaken out ships nothing; the stylesheets the window's CSS pulls in from
// packages; and the files a package hands over whole, such as fonts.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { build } from "vite";

/** The package a path inside `node_modules` belongs to, the innermost one when they nest. */
function packageOf(path) {
  const parts = path.split(/(?:^|[\\/])node_modules[\\/]/);
  if (parts.length < 2) return null;
  const inner = parts[parts.length - 1].split(/[\\/]/);
  return inner[0].startsWith("@") ? `${inner[0]}/${inner[1]}` : inner[0];
}

/** An `@import` in a stylesheet, of a package or of a file beside it. */
const IMPORTED = /@import\s+(?:url\()?["']([^"']+)["']/g;

/**
 * The packages a stylesheet of the window's own pulls in, through its files as well.
 *
 * Each is inlined where it is imported, so the bundler lists none of them as a module; they are
 * read off the `@import`s instead, by the package each names.
 */
function stylesheetsImported(file, seen = new Set()) {
  if (seen.has(file)) return [];
  seen.add(file);

  return [...readFileSync(file, "utf8").matchAll(IMPORTED)].flatMap(([, named]) => {
    if (/^[a-z]+:/i.test(named)) return [];
    if (/^[./]/.test(named)) return stylesheetsImported(join(dirname(file), named), seen);
    const parts = named.split("/");
    return [named.startsWith("@") ? `${parts[0]}/${parts[1]}` : parts[0]];
  });
}

/**
 * The package a module of the script came from, or null for this project's own source.
 *
 * A module whose id opens with a zero byte is one a plugin made up. Those that wrap a real file
 * name it, and belong to that file's package; vite's own are its module preload polyfill and its
 * preload helper, under vite's notice, and the bundler's own are the helpers the script runs on,
 * under rolldown's. Anything else is a module nobody here can say the owner of, which is refused
 * rather than guessed.
 */
function moduleOwner(id) {
  if (!id.startsWith("\0")) return packageOf(id);
  const made = id.slice(1);
  const wrapped = packageOf(made);
  if (wrapped !== null) return wrapped;
  if (made.startsWith("vite/")) return "vite";
  if (made.startsWith("rolldown/")) return "rolldown";
  return undefined;
}

const shipped = new Map();
const nobodys = [];
const ships = (owner, why) => {
  if (owner && !shipped.has(owner)) shipped.set(owner, why);
};

await build({
  configFile: "vite.config.ts",
  logLevel: "error",
  build: { write: false, emptyOutDir: false },
  plugins: [
    {
      name: "lyonoyl:what-ships",
      enforce: "post",
      generateBundle(_options, bundle) {
        for (const file of Object.values(bundle)) {
          if (file.type === "chunk") {
            for (const [id, rendered] of Object.entries(file.modules)) {
              if (rendered.renderedLength === 0) continue;
              const owner = moduleOwner(id);
              if (owner === undefined) nobodys.push(JSON.stringify(id));
              else ships(owner, `code in ${file.fileName}`);
            }
          } else {
            for (const source of file.originalFileNames ?? []) {
              ships(packageOf(source), `the file ${file.fileName}`);
            }
          }
        }
        for (const id of this.getModuleIds()) {
          const file = id.replace(/\?.*$/, "");
          if (!file.endsWith(".css")) continue;
          const owner = packageOf(file);
          const owners = owner === null ? stylesheetsImported(file) : [owner];
          for (const one of owners) ships(one, "styles in the window's stylesheet");
        }
      },
    },
  ],
});

const listed = new Set(
  JSON.parse(readFileSync("src-tauri/licences-npm.json", "utf8")).map((one) => one.name),
);
const missing = [...shipped.keys()].filter((owner) => !listed.has(owner)).sort();

if (nobodys.length > 0) {
  console.error(`modules in the window that belong to no package anybody can name: ${nobodys.join(", ")}`);
  console.error("Say whose they are in scripts/check-shipped-licences.mjs, and list that package's notice.");
}
if (missing.length > 0) {
  for (const owner of missing) console.error(`${owner} ships (${shipped.get(owner)}) without a notice`);
  console.error("Name it in scripts/collect-npm-licences.mjs, or find why the collector does not.");
}
if (nobodys.length > 0 || missing.length > 0) process.exit(1);

console.log(`${shipped.size} packages in the window, each with its notice`);
