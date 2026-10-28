// `npm run app:screenshots`: builds the application made for screenshots, and opens it.
//
// It is this application in every part a person sees, over a profile of its own that is emptied and
// filled from a scene at every start: made-up accounts, their groups, their mail. It keeps its
// secrets in memory, reads its mail from the scene rather than from any server, never opens the real
// profile, and is a different application to the system, so it can run beside the real one. `docs/development/guide.md` says what a
// scene holds.
//
// The scenes are in `mocks/screenshots/`, a folder each, named by language. English unless another
// is asked for: `npm run app:screenshots -- --<language>` plays `mocks/screenshots/<language>`.
//
// macOS only, because the screenshots are of the macOS window.

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { setTimeout as after } from "node:timers/promises";
import { fileURLToPath } from "node:url";

// Every path below is the repository's, wherever this was started from.
process.chdir(join(dirname(fileURLToPath(import.meta.url)), ".."));

const bold = (what) => `\u001b[1m${what}\u001b[0m`;
const no = (what) => {
  console.error(`\u001b[31m${what}\u001b[0m`);
  process.exit(1);
};

if (process.platform !== "darwin") {
  no("The build made for screenshots is macOS only: the screenshots are of the macOS window.");
}

// Which scene: a language, as its file is named.
const asked = process.argv.slice(2);
const language = (asked[0] ?? "en").replace(/^-+/, "");
if (asked.length > 1 || !/^[a-z]{2,3}(-[A-Za-z0-9]+)*$/.test(language)) {
  no(`One language, as its scene is named: npm run app:screenshots -- --ru. Not: ${asked.join(" ")}`);
}

const scenes = resolve("mocks", "screenshots");
const folder = join(scenes, language);
const scene = join(folder, "scene.json");
if (!existsSync(scene)) {
  const there = readdirSync(scenes).filter((name) => existsSync(join(scenes, name, "scene.json")));
  no(`There is no scene for ${language} at ${folder}. There: ${there.join(", ") || "none"}.`);
}

// Read before the build rather than after it: a comma out of place is a second to find here and
// minutes to find once the build has finished. What the scene means is checked as the window starts.
try {
  JSON.parse(readFileSync(scene, "utf8"));
} catch (why) {
  no(`${scene} is not JSON: ${why.message}`);
}

const IDENTIFIER = "com.lyonoyl.mail.screenshots";
const built = "target/release/bundle/macos/Lyonoyl Mail.app";
const home = "target/screenshots";
const app = resolve(home, "Lyonoyl Mail.app");
const kept = resolve(home, "kept", "Lyonoyl Mail.app");
const log = resolve(home, "last-run.log");

// What the bundler writes over is the application `npm run app:build` left there, and nobody asked
// for that to go. It is set aside for the build and put back after it, whatever the build does.
mkdirSync(home, { recursive: true });
const setAside = existsSync(built);
if (setAside) {
  rmSync(kept, { recursive: true, force: true });
  mkdirSync(resolve(home, "kept"), { recursive: true });
  renameSync(built, kept);
}

const putBack = () => {
  if (setAside && existsSync(kept)) {
    rmSync(built, { recursive: true, force: true });
    renameSync(kept, built);
    rmSync(resolve(home, "kept"), { recursive: true, force: true });
  }
};

// A build that signs with nothing, whatever this shell holds: with a signing identity in the
// environment the bundler signs, and with an update key the build would look for updates. Neither
// belongs on an application that plays a scene.
const environment = { ...process.env };
for (const name of Object.keys(environment)) {
  if (name.startsWith("APPLE_") || name.startsWith("LYONOYL_UPDATE_")) delete environment[name];
}

// Its own identity, so the system sees another application: the rule that one copy runs at a time and
// the window's own storage both go by it. The scene's folder goes in among its resources, whole, so
// what a message names beside it travels with it.
const config = JSON.stringify({
  identifier: IDENTIFIER,
  bundle: { resources: { [`${folder}/`]: "scene/" } },
});

console.log(`\n${bold("Building the application made for screenshots")} — ${folder}\n`);

// Ctrl-C reaches the build as well, which stops; the application set aside is put back below
// rather than left where this script moved it.
process.on("SIGINT", () => {});

const building = await new Promise((done) => {
  const it = spawn(
    "npx",
    ["tauri", "build", "--bundles", "app", "--features", "screenshots", "--config", config],
    { stdio: "inherit", env: environment },
  );
  it.on("exit", (code) => done(code ?? 1));
});

if (building !== 0) {
  putBack();
  no("The build failed: what it said is above.");
}

// The copy already running, if there is one, is the old build: the system would bring it forward
// instead of starting the new one. Asked to stop and waited for, before its files are replaced.
const running = () => spawnSync("pgrep", ["-f", `${app}/Contents/MacOS/`]).status === 0;
if (running()) {
  spawnSync("pkill", ["-f", `${app}/Contents/MacOS/`]);
  for (let waited = 0; running() && waited < 50; waited++) await after(200);
  if (running()) {
    putBack();
    no(`The copy already running did not stop. Quit it, and run this again: ${app}`);
  }
}

rmSync(app, { recursive: true, force: true });
renameSync(built, app);
putBack();

// Opened the way Finder opens it, with what it says written down, so a scene it refuses is said
// here rather than left as a window that never came.
spawnSync("open", ["--stdout", log, "--stderr", log, app]);
await after(5000);

if (!running()) {
  const said = existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").slice(-20).join("\n") : "";
  no(`It stopped as it started. What it said:\n${said || "nothing"}`);
}

console.log(`\n${bold("Running")}\n`);
console.log(`  ${app}\n`);
console.log("  Every start plays the scene again from the beginning, dated from that moment: open");
console.log("  it again from there whenever it is wanted. It keeps nothing from one start to the next");
console.log("  and touches nothing of the real application's.\n");
