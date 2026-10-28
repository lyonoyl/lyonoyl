// Whether this machine has what building and checking Lyonoyl Mail needs, and how to get what it
// lacks. `scripts/preflight.sh` runs it first; `npm run check:machine` runs it alone. With `--build`
// it asks only what a build needs, which is what `npm run app:dev-build` runs first.
//
// Every check runs, so one run names everything that is missing. It installs nothing.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

/** The cargo-about that `src-tauri/licences-rust.json` is collected with. */
export const ABOUT = "0.9.2";

/** The oldest macOS a build is made on. */
const MACOS = 13;

/** Seconds of uptime after which macOS's TCP clock, a 32-bit count of milliseconds, has wrapped. */
export const WRAPS_AFTER = 2 ** 32 / 1000;

/** Seconds added to twice the maximum segment lifetime before a closed connection counts as kept. */
const MARGIN = 5;

/** The pkg-config modules a Linux build links against, and the Debian and Ubuntu package of each. */
const LINUX_LIBRARIES = [
  ["webkit2gtk-4.1", "libwebkit2gtk-4.1-dev"],
  ["gtk+-3.0", "libgtk-3-dev"],
  ["libsoup-3.0", "libsoup-3.0-dev"],
  ["ayatana-appindicator3-0.1", "libayatana-appindicator3-dev"],
  ["librsvg-2.0", "librsvg2-dev"],
  ["openssl", "libssl-dev"],
];

/** The oldest WebKitGTK a Linux build is made against. */
const WEBKITGTK = [2, 44];

const DOCKER = {
  darwin: "Docker Desktop: https://docs.docker.com/desktop/setup/install/mac-install/",
  linux: "Docker Engine with its Compose plugin: https://docs.docker.com/engine/install/",
  win32: "Docker Desktop: https://docs.docker.com/desktop/setup/install/windows-install/",
};

/** How a command that comes with the system's developer tools is installed on each system. */
const SYSTEM_TOOL = {
  darwin: () => "xcode-select --install",
  linux: (packages) => `sudo apt-get install ${packages}`,
  win32: (name) => ({ python3: "https://www.python.org/downloads/windows/", git: "https://git-scm.com/download/win" })[name],
};

const found = (name) => ({ ok: true, name });
const missing = (name, fix, ...why) => ({ ok: false, name, fix, why });
const noted = (name, note) => ({ ok: true, name, note });

/** The first version in a command's answer: `Docker Compose version v5.5.1` gives `5.5.1`. */
export function versionIn(text) {
  return /(\d+\.\d+(?:\.\d+)?)/.exec(text ?? "")?.[1] ?? null;
}

/** Whether a version is in a range written the way `engines` writes one: `^20.19.0 || >=24.0.0`. */
export function satisfies(version, range) {
  const [major, minor, patch] = version.replace(/^v/, "").split(".").map(Number);
  const atLeast = (a, b, c) => major > a || (major === a && (minor > b || (minor === b && patch >= c)));
  return range.split("||").some((part) => {
    const want = part.trim();
    const caret = /^\^(\d+)\.(\d+)\.(\d+)$/.exec(want);
    if (caret) return major === Number(caret[1]) && atLeast(...caret.slice(1).map(Number));
    const floor = /^>=\s*(\d+)\.(\d+)\.(\d+)$/.exec(want);
    if (floor) return atLeast(...floor.slice(1).map(Number));
    throw new Error(`${want} is a range this check does not read`);
  });
}

/** The toolchain `rust-toolchain.toml` pins, and the components it names. */
export function pinnedToolchain(toml) {
  const channel = /^channel\s*=\s*"(.+)"\s*$/m.exec(toml)?.[1] ?? null;
  const listed = /^components\s*=\s*\[([^\]]*)\]/m.exec(toml)?.[1] ?? "";
  return { channel, components: [...listed.matchAll(/"([^"]+)"/g)].map((component) => component[1]) };
}

/** The directory holding a command, from `PATH` or from the places a tool is installed to. */
function holding(machine, command, also = []) {
  const names = machine.platform === "win32" ? [`${command}.exe`, `${command}.cmd`] : [command];
  for (const directory of [...(machine.env.PATH ?? "").split(delimiter), ...also]) {
    if (directory && names.some((name) => machine.exists(join(directory, name)))) return directory;
  }
  return null;
}

function cargoBin(machine) {
  return machine.env.CARGO_HOME ? join(machine.env.CARGO_HOME, "bin") : join(machine.home, ".cargo", "bin");
}

/** Node.js, in the range `package.json` names in `engines`. */
function node(machine) {
  const range = JSON.parse(machine.read("package.json")).engines?.node;
  const version = machine.node.replace(/^v/, "");
  try {
    if (!range || satisfies(version, range)) return found(`Node.js ${version}`);
  } catch (error) {
    return missing(`Node.js ${version}: engines.node in package.json says ${error.message}`, "write it as ^X.Y.Z and >=X.Y.Z joined by ||");
  }
  return missing(`Node.js ${version}, where ${range} is needed`, "an LTS release from https://nodejs.org");
}

/** What `npm ci` installs, present and matching `package.json`. */
function packages(machine) {
  if (!machine.exists(join(machine.root, "node_modules"))) return missing("npm packages are not installed", "npm ci");
  const listed = machine.run("npm", ["ls", "--depth=0"]);
  if (listed.ok) return found("npm packages");
  const wrong = listed.out.split("\n").filter((line) => /missing|invalid|ERR/.test(line)).slice(0, 3);
  return missing("npm packages do not match package.json", "npm ci", ...wrong);
}

/** rustup, and the toolchain the tree pins, with its components. */
function rust(machine, state) {
  const rustup = machine.run("rustup", ["--version"]);
  if (!rustup.ok) {
    const fix =
      machine.platform === "darwin"
        ? "brew install rustup, and put $(brew --prefix rustup)/bin first on PATH; or https://rustup.rs"
        : "https://rustup.rs";
    return missing("rustup is not installed", fix);
  }
  const { channel, components } = pinnedToolchain(machine.read("rust-toolchain.toml"));
  state.channel = channel;
  const install = `rustup toolchain install ${channel} --component ${components.join(",")}`;
  const toolchains = machine.run("rustup", ["toolchain", "list"]).out.split("\n");
  if (!toolchains.some((line) => line === channel || line.startsWith(`${channel}-`))) {
    return missing(`Rust ${channel}, which rust-toolchain.toml pins, is not installed`, install);
  }
  const installed = machine.run("rustup", ["component", "list", "--installed", "--toolchain", channel]).out.split("\n");
  const absent = components.filter((name) => !installed.some((line) => line === name || line.startsWith(`${name}-`)));
  if (absent.length) return missing(`Rust ${channel} lacks ${absent.join(", ")}`, install);
  state.toolchain = true;
  return found(`Rust ${channel}, with ${components.join(", ")}`);
}

/** The `rustc` on `PATH` is rustup's own, which reads `rust-toolchain.toml`. */
function compiler(machine, state) {
  if (!state.toolchain) return null;
  const running = versionIn(machine.run("rustc", ["--version"]).out);
  if (running === state.channel) return found(`rustc on PATH is ${running}, as rust-toolchain.toml pins`);
  const proxies = holding(machine, "rustup", [cargoBin(machine), "/opt/homebrew/opt/rustup/bin", "/usr/local/opt/rustup/bin"]);
  return missing(
    `rustc on PATH is ${running ?? "missing"}, and the tree pins ${state.channel}`,
    `put ${proxies ?? "the directory holding rustup"} first on PATH: only rustup's own cargo and rustc read rust-toolchain.toml`,
  );
}

/** A cargo subcommand, found where cargo finds one: on `PATH` or in cargo's own `bin`. */
function subcommand(machine, tool, install) {
  const directory = holding(machine, tool, [cargoBin(machine)]);
  if (!directory) return { version: null, fix: install };
  return { version: versionIn(machine.run(join(directory, tool), ["--version"]).out), fix: install };
}

function audit(machine) {
  const { version, fix } = subcommand(machine, "cargo-audit", "cargo install cargo-audit --locked");
  return version ? found(`cargo-audit ${version}`) : missing("cargo-audit is not installed", fix);
}

function about(machine) {
  const { version, fix } = subcommand(machine, "cargo-about", `cargo install cargo-about --locked --version ${ABOUT}`);
  if (!version) return missing("cargo-about is not installed", fix);
  if (version === ABOUT) return found(`cargo-about ${version}`);
  return noted(
    `cargo-about ${version}`,
    `the licence list in the tree is collected with ${ABOUT}; if the licence step finds a list nobody changed different, ${fix}`,
  );
}

function python(machine) {
  const version = versionIn(machine.run("python3", ["--version"]).out);
  return version ? found(`Python ${version}`) : missing("python3 is not installed", SYSTEM_TOOL[machine.platform]("python3"));
}

function git(machine) {
  const version = versionIn(machine.run("git", ["--version"]).out);
  return version ? found(`git ${version}`) : missing("git is not installed", SYSTEM_TOOL[machine.platform]("git"));
}

function docker(machine) {
  const version = versionIn(machine.run("docker", ["--version"]).out);
  if (!version) return missing("Docker is not installed, and the development mail server runs in it", DOCKER[machine.platform]);
  const compose = versionIn(machine.run("docker", ["compose", "version"]).out);
  if (!compose) return missing(`Docker ${version} has no Compose, and the mail server is started with it`, DOCKER[machine.platform]);
  return found(`Docker ${version}, Compose ${compose}`);
}

function commandLineTools(machine) {
  return machine.run("xcode-select", ["-p"]).ok
    ? found("Xcode Command Line Tools")
    : missing("the Xcode Command Line Tools are not installed", "xcode-select --install");
}

function macos(machine) {
  const version = versionIn(machine.run("sw_vers", ["-productVersion"]).out) ?? "0";
  return Number(version.split(".")[0]) >= MACOS
    ? found(`macOS ${version}`)
    : missing(`macOS ${version}, where ${MACOS} Ventura or newer is needed`, "update macOS");
}

function linux(machine) {
  const results = [];
  if (!machine.run("cc", ["--version"]).ok) results.push(missing("no C compiler", "sudo apt-get install build-essential"));
  if (!machine.run("pkg-config", ["--version"]).ok) {
    results.push(missing("pkg-config is not installed", "sudo apt-get install pkg-config"));
    return results;
  }
  const absent = LINUX_LIBRARIES.filter(([module]) => !machine.run("pkg-config", ["--exists", module]).ok);
  if (absent.length) {
    results.push(
      missing(
        `libraries the build links against are missing: ${absent.map(([module]) => module).join(", ")}`,
        `on Debian and Ubuntu, sudo apt-get install ${absent.map(([, name]) => name).join(" ")}`,
      ),
    );
    return results;
  }
  const webkit = versionIn(machine.run("pkg-config", ["--modversion", "webkit2gtk-4.1"]).out) ?? "0";
  const [major, minor] = webkit.split(".").map(Number);
  const [wantMajor, wantMinor] = WEBKITGTK;
  results.push(
    major > wantMajor || (major === wantMajor && minor >= wantMinor)
      ? found(`WebKitGTK ${webkit}, and the other libraries the build links against`)
      : missing(`WebKitGTK ${webkit}, where ${WEBKITGTK.join(".")} or newer is needed`, "a newer release of the system"),
  );
  return results;
}

/** The TCP connections in TIME_WAIT that `netstat` lists, one line each. */
function waiting(machine) {
  return new Set(
    machine
      .run("netstat", ["-anp", "tcp"])
      .out.split("\n")
      .filter((line) => /\bTIME_WAIT\s*$/.test(line))
      .map((line) => line.trim().split(/\s+/).slice(3, 5).join(" ")),
  );
}

/**
 * Whether this Mac still clears connections that have closed.
 *
 * Past 2^32 ms of uptime macOS keeps every closed connection in TIME_WAIT for good. Each holds a
 * local port, and a new connection given one of those ports hangs: to the development mail server,
 * that is a test failing as if nothing answered. Asked only past that uptime, by watching whether
 * the connections in TIME_WAIT now outlast twice the maximum segment lifetime.
 */
export async function closedConnections(machine) {
  const boot = Number(/sec = (\d+)/.exec(machine.run("sysctl", ["-n", "kern.boottime"]).out)?.[1]);
  if (!boot) return found("closed connections: uptime unknown, not asked");
  const days = (machine.now() - boot) / 86400;
  const up = `${days.toFixed(1)} days since the last restart`;
  if (machine.now() - boot < WRAPS_AFTER) return found(`closed connections are cleared: ${up}`);

  const before = waiting(machine);
  if (before.size === 0) return found(`closed connections: none are held, ${up}`);
  const lifetime = Number(machine.run("sysctl", ["-n", "net.inet.tcp.msl"]).out) / 1000 || 15;
  const seconds = Math.ceil(2 * lifetime + MARGIN);
  machine.say(`  … this Mac has run ${up}; watching whether it clears closed connections, ~${seconds} s\n`);
  await machine.wait(seconds);
  const after = waiting(machine);
  const kept = [...before].filter((connection) => after.has(connection)).length;
  if (kept * 2 <= before.size) return found(`closed connections are cleared: ${up}`);

  return missing(
    `this Mac has run ${days.toFixed(0)} days without a restart and no longer clears closed connections`,
    "restart the Mac, and run this again",
    `${kept} of the ${before.size} closed connections held ${seconds} s ago are still held, though macOS lets one go after ${2 * lifetime} s.`,
    "macOS stops clearing them once it has run 49.7 days without a restart, and a new connection given a port one of them holds",
    "hangs: the tests that ask the development mail server then fail at random, as if nothing answered.",
  );
}

/** Every check, in the order they are shown, with the systems each applies to and whether a build needs it. */
export const NEEDS = [
  { on: ["darwin"], build: true, check: commandLineTools },
  { on: ["darwin"], build: true, check: macos },
  { on: ["linux"], build: true, check: linux },
  { build: true, check: node },
  { build: true, check: packages },
  { build: true, check: rust },
  { build: true, check: compiler },
  { check: audit },
  { check: about },
  { check: python },
  { check: git },
  { check: docker },
  { on: ["darwin"], check: closedConnections },
];

/** Runs every check that applies here, or a build's alone, says each answer as it comes, and returns them. */
export async function checkMachine(machine, { build = false } = {}) {
  const state = {};
  const answers = [];
  for (const need of NEEDS) {
    if (need.on && !need.on.includes(machine.platform)) continue;
    if (build && !need.build) continue;
    const answer = await need.check(machine, state);
    for (const one of [answer].flat().filter(Boolean)) {
      answers.push(one);
      machine.say(shown(one));
    }
  }
  return answers;
}

function shown(answer) {
  if (answer.ok && !answer.note) return `  \u001b[32m✓\u001b[0m ${answer.name}\n`;
  if (answer.ok) return `  \u001b[33m!\u001b[0m ${answer.name}\n      ${answer.note}\n`;
  const why = answer.why.map((line) => `      ${line}\n`).join("");
  return `  \u001b[31m✗ ${answer.name}\u001b[0m\n${why}      → ${answer.fix}\n`;
}

/** This machine, as the checks above ask it. */
function here(root) {
  return {
    platform: process.platform,
    root,
    home: homedir(),
    env: process.env,
    node: process.version,
    exists: existsSync,
    read: (path) => readFileSync(join(root, path), "utf8"),
    run: (command, args) => {
      const ran = spawnSync(command, args, {
        cwd: root,
        encoding: "utf8",
        shell: process.platform === "win32",
        env: { ...process.env, RUSTUP_AUTO_INSTALL: "0" },
      });
      return { ok: ran.status === 0, out: `${ran.stdout ?? ""}${ran.stderr ?? ""}`.trim() };
    },
    now: () => Date.now() / 1000,
    wait: (seconds) => sleep(seconds * 1000),
    say: (text) => process.stdout.write(text),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const machine = here(fileURLToPath(new URL("..", import.meta.url)));
  const build = process.argv.includes("--build");
  machine.say(`What this machine has for ${build ? "building" : "building and checking"} Lyonoyl Mail\n\n`);
  const answers = await checkMachine(machine, { build });
  const lacking = answers.filter((answer) => !answer.ok).length;
  if (lacking) {
    const said = lacking === 1 ? "One thing stands in the way, and its line says" : `${lacking} things stand in the way, and each line says`;
    machine.say(`\n${said} what to do. Then run this again.\n`);
    process.exit(1);
  }
}
