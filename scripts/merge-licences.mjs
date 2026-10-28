#!/usr/bin/env node
// The two halves, as one list the application reads.
//
// Grouped by the text rather than by the package, because that is what a notice is: sixty crates
// under one MIT with one copyright line is one notice, and listing it sixty times would be a
// screen nobody scrolls to the end of.

import { readFileSync, writeFileSync } from "node:fs";

const rust = JSON.parse(readFileSync("src-tauri/licences-rust.json", "utf8"));
const npm = JSON.parse(readFileSync("src-tauri/licences-npm.json", "utf8"));
// The C libraries compiled into the binary, whose licences no crate declares — see
// scripts/collect-vendored-licences.mjs for why they cannot be collected with the rest.
const vendored = JSON.parse(
  readFileSync("src-tauri/licences-vendored.json", "utf8"),
);

/** Text -> what it covers. */
const byText = new Map();

const note = (licence, text, used) => {
  const key = text.trim();
  if (!byText.has(key)) byText.set(key, { licence, text: key, used: [] });
  byText.get(key).used.push(...used);
};

for (const one of rust) note(one.licence, one.text, one.used_by);
for (const one of vendored) note(one.licence, one.text, one.used_by);
// A build tool says what of it ships: its name alone would read as the whole of it, and the whole
// of it stays on the machine that builds.
for (const one of npm) {
  note(one.licence ?? "?", one.notice ?? "", [one.covers ? `${one.name} (${one.covers})` : one.name]);
}

const notices = [...byText.values()]
  .map((one) => ({ ...one, used: [...new Set(one.used)].sort() }))
  .sort((left, right) => left.used[0].localeCompare(right.used[0]));

writeFileSync("src-tauri/licences.json", JSON.stringify(notices, null, 2) + "\n");

const covered = new Set(notices.flatMap((one) => one.used));
console.log(`${covered.size} packages under ${notices.length} notices`);
