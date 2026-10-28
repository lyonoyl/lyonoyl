// `npm run app`, on all three systems.
//
// On macOS the Rust binary is handed to a runner that signs it before launching — see
// `scripts/dev-runner-macos.sh` for why, and `docs/development/guide.md` for what it buys. That runner is
// a shell script, which Windows cannot execute at all, and the signing it does would mean nothing
// on either of the other two: the keychain this is for is macOS's.
//
// So the choice is made here rather than in `package.json`, which has no way to make it.

import { spawn } from "node:child_process";

const signing =
  process.platform === "darwin"
    ? // Resolved against `src-tauri`, which is where Tauri runs the runner from.
      ["--runner", "../scripts/dev-runner-macos.sh"]
    : [];

const tauri = spawn(
  "npx",
  ["tauri", "dev", ...signing, ...process.argv.slice(2)],
  { stdio: "inherit", shell: process.platform === "win32" },
);

tauri.on("exit", (code, signal) => {
  // A signal is how this ends when somebody presses Ctrl-C, and it is not a failure.
  process.exit(signal ? 0 : (code ?? 1));
});
