#!/usr/bin/env node
// The licences of code that ships inside this application without being a dependency of it.
//
// `cargo about` reads the `license` field of each crate, and a `-sys` crate declares the licence of
// its Rust wrapper. The C project it vendors and compiles in carries its own, and that one never
// appears: `libsqlite3-sys` says MIT while the SQLCipher it builds is Zetetic's BSD, and
// `openssl-src` says MIT/Apache-2.0 while the OpenSSL it builds is Apache-2.0 with its own
// copyright. Both licences ask for the notice to travel with the binary, and `cargo about` alone
// leaves both out of the list this application shows.
//
// The same hole has a second shape: source copied into this tree. The files in
// `src/shared/ui/primitives/` are shadcn's, taken by its own command and kept unmodified, and the
// variants they are styled through are copied into `src/index.css`. All of it is built into the
// window, and nothing in `package.json` marks it as shipping: the command that wrote it is run when
// a primitive is added and is not installed. So the notice it came with is kept in this tree, in
// `src-tauri/licences-copied/`.
//
// A compiled-in entry names where the text lives inside its crate rather than where the crate lives:
// the directory is resolved through `cargo metadata`, so nothing here holds a path to anybody's
// machine. A copied entry names a file in this tree.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

/**
 * A C project that is compiled into the application, and the licence that comes with it.
 *
 * `crate` is what pulls it in, `text` is the licence file inside that crate's own directory. A
 * crate in the graph whose licence file is not where this says fails the run rather than dropping
 * the notice — which is the whole reason this file exists.
 */
const VENDORED = [
  {
    crate: "libsqlite3-sys",
    project: "SQLCipher",
    licence: "BSD-3-Clause",
    text: "sqlcipher/LICENSE",
    // Compiled in by the `bundled-sqlcipher-vendored-openssl` feature in src-tauri/Cargo.toml,
    // which is how every page of both databases comes to be encrypted.
  },
  {
    crate: "openssl-src",
    project: "OpenSSL",
    licence: "Apache-2.0",
    text: "openssl/LICENSE.txt",
    // Pulled in by the same feature: SQLCipher takes its ciphers from it.
  },
];

/**
 * Source taken into this tree from somewhere else, and the licence that came with it.
 *
 * `text` is the notice exactly as the project publishes it. `covers` is what in this tree came from
 * it, so a reader of the notice can tell what it is about.
 */
const COPIED = [
  {
    project: "shadcn/ui",
    licence: "MIT",
    text: "src-tauri/licences-copied/shadcn-ui.md",
    covers: "src/shared/ui/primitives/ and src/index.css",
  },
  // The palettes, dark and light: each scheme's colours in src/index.css are the project's own,
  // under the name people know them by. Solarized and Gruvbox give a dark and a light side under
  // the one notice each.
  {
    project: "Dracula",
    licence: "MIT",
    text: "src-tauri/licences-copied/dracula.md",
    covers: "src/index.css",
  },
  {
    project: "Nord",
    licence: "MIT",
    text: "src-tauri/licences-copied/nord.md",
    covers: "src/index.css",
  },
  {
    project: "Solarized",
    licence: "MIT",
    text: "src-tauri/licences-copied/solarized.md",
    covers: "src/index.css",
  },
  {
    // The project publishes no licence file of its own beside its MIT; the maintained Gruvbox
    // publishes the text under the name of the palette's author.
    project: "Gruvbox",
    licence: "MIT",
    text: "src-tauri/licences-copied/gruvbox.md",
    covers: "src/index.css",
  },
  {
    project: "One Dark",
    licence: "MIT",
    text: "src-tauri/licences-copied/one-dark.md",
    covers: "src/index.css",
  },
  {
    project: "Catppuccin",
    licence: "MIT",
    text: "src-tauri/licences-copied/catppuccin.md",
    covers: "src/index.css",
  },
  {
    project: "Rosé Pine",
    licence: "MIT",
    text: "src-tauri/licences-copied/rose-pine.md",
    covers: "src/index.css",
  },
];

const metadata = JSON.parse(
  execFileSync(
    "cargo",
    [
      "metadata",
      "--format-version",
      "1",
      "--manifest-path",
      "src-tauri/Cargo.toml",
    ],
    { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  ),
);

const notices = [];

for (const one of VENDORED) {
  const crate = metadata.packages.find((held) => held.name === one.crate);
  if (crate === undefined) {
    // Not in the graph, so nothing of it ships. A feature turned off is not a missing notice.
    continue;
  }

  const text = join(dirname(crate.manifest_path), one.text);
  if (!existsSync(text)) {
    console.error(
      `${one.project} ships inside ${one.crate} ${crate.version}, and its licence is not at ${one.text}.`,
    );
    console.error(
      "Find where it moved to and correct the table in this file. A notice that would ship missing is what this refuses.",
    );
    process.exit(1);
  }

  notices.push({
    licence: one.licence,
    text: readFileSync(text, "utf8").trim(),
    used_by: [`${one.project} (compiled into ${one.crate} ${crate.version})`],
  });
}

for (const one of COPIED) {
  if (!existsSync(one.text)) {
    console.error(
      `${one.project} is copied into ${one.covers}, and its licence is not at ${one.text}.`,
    );
    console.error(
      "Put the notice back where the table says, or correct the table in this file. Code that",
    );
    console.error("ships without its notice is what this refuses.");
    process.exit(1);
  }

  notices.push({
    licence: one.licence,
    text: readFileSync(one.text, "utf8").trim(),
    used_by: [`${one.project} (copied into ${one.covers})`],
  });
}

writeFileSync(
  "src-tauri/licences-vendored.json",
  JSON.stringify(notices, null, 2) + "\n",
);

console.log(`${notices.length} notices for what ships without declaring itself`);
