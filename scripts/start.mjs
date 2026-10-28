// The way in for somebody who has just cloned this and does not know what it offers.
//
// `npm start` is the one command worth remembering, and it is the one npm itself suggests. It asks
// what you want in plain words and runs it. Everything it offers has its own name as well, for
// anybody who already knows which one they want — this is a door, not a layer.
//
// It offers only what this machine can actually do: releasing is macOS with a certificate set up,
// and saying so is more use than offering a choice that fails three minutes later.

import { spawn } from "node:child_process";
import { createInterface } from "node:readline/promises";
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const mac = process.platform === "darwin";
const identity = process.env.LYONOYL_SIGNING_ENV ?? join(homedir(), ".config", "lyonoyl", "signing.env");
const canSign = mac && existsSync(identity);

// What this machine can actually do, which is not the same as what the project can do. The
// preflight is a bash script, and on Windows `npm` hands that to `cmd.exe`, which answers with a
// line about a file it does not know how to run. Offering it there is offering a door into a wall.
const canRunAShellScript = process.platform !== "win32";

const doors = [
  {
    say: "Work on the code",
    then: "Starts it with hot reload. Editing the interface updates the open window; editing Rust rebuilds and restarts it.",
    run: "app",
  },
  {
    say: "Build one to keep",
    then: "Produces an application you can move where you like and use. Takes a few minutes the first time.",
    run: "app:build",
  },
];

if (canRunAShellScript) {
  doors.push({
    say: "Check everything is sound",
    then: "Formatting, lints, the whole test suite, the architecture gates. What a release and a pull request pass.",
    run: "preflight",
  });
}

if (canSign) {
  doors.push({
    say: "Release it",
    then: "macOS only. Signs, notarizes, staples, and then asks Gatekeeper what it made. For whoever publishes this.",
    run: "app:release",
  });
}

const note = mac
  ? canSign
    ? null
    : `Releasing is not offered: no signing identity at ${identity}. docs/development/guide.md says how.`
  : "Releasing is not offered: it is macOS only, and this is not macOS.";

console.log("\nLyonoyl Mail\n");
doors.forEach((door, at) => {
  console.log(`  ${at + 1}  ${door.say}`);
  console.log(`     ${door.then}\n`);
});
if (note) console.log(`  ${note}\n`);

const asking = createInterface({ input: process.stdin, output: process.stdout });
const chosen = await asking.question(`  Which one? 1-${doors.length}, or Enter to stop: `);
asking.close();

const door = doors[Number(chosen) - 1];
if (!door) {
  console.log("\nNothing done.\n");
  process.exit(0);
}

console.log(`\n${door.say} — npm run ${door.run}\n`);

const running = spawn("npm", ["run", door.run], {
  stdio: "inherit",
  shell: process.platform === "win32",
});
running.on("exit", (code, signal) => process.exit(signal ? 0 : (code ?? 1)));
