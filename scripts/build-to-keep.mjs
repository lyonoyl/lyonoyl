// Builds an application to keep and use, and then says where it is and what to do with it.
//
// The step that was missing. `tauri build` finishes by printing a path into a `target` directory
// nobody thinks of as a place to keep things, and leaves the reader to work out whether that is the
// thing they wanted, whether it can be moved, and whether anybody else can open it. Each of those
// has an answer, and none of them was written down.

import { spawn } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

const where = {
  darwin: { at: "target/release/bundle/macos", ends: ".app", move: "your Applications folder" },
  win32: { at: "target/release", ends: ".exe", move: "wherever you keep programs" },
  linux: { at: "target/release", ends: "", move: "somewhere on your path" },
};

const run = (command, args) =>
  new Promise((done, fail) => {
    const it = spawn(command, args, { stdio: "inherit", shell: process.platform === "win32" });
    it.on("exit", (code) => (code === 0 ? done() : fail(new Error(`${command} exited ${code}`))));
  });

// What a build needs is asked first, all of it at once. The build is the one the preflight makes as
// its last step: the `.app` on macOS, the binary elsewhere.
await run("node", ["scripts/check-machine.mjs", "--build"]);
await run("npx", ["tauri", "build", ...(process.platform === "darwin" ? ["--bundles", "app"] : ["--no-bundle"])]);

const here = where[process.platform];
const built =
  here && existsSync(here.at)
    ? readdirSync(here.at).find((one) => (here.ends ? one.endsWith(here.ends) : one === "lyonoyl"))
    : undefined;

const bold = (what) => `\u001b[1m${what}\u001b[0m`;

console.log(`\n${bold("Done")}\n`);

if (!built) {
  console.log("The build finished, but not where this script looks -- see the path it printed above.\n");
  process.exit(0);
}

console.log(`  ${join(here.at, built)}\n`);
console.log(`  Move it to ${here.move} and open it. It is yours to keep: it does not need this`);
console.log("  folder, and nothing here has to stay installed for it to run.\n");

if (process.platform === "darwin") {
  console.log("  It is signed with nothing, so macOS will refuse to open it on any machine but");
  console.log("  this one. That is expected for a build you made yourself, and it is why the first");
  console.log("  open asks you to confirm. A published build is signed and notarized instead.\n");
  console.log("  Signed with nothing means named by its own contents, so the next build you make");
  console.log("  is a different application as far as the keychain is concerned: it will ask for");
  console.log("  the key to your databases again. Allow it when it asks, or set up a signing");
  console.log("  identity and use `npm run app:dev-build`, which keeps one name across builds.\n");
}
