import { describe, expect, it } from "vitest";
import { ABOUT, WRAPS_AFTER, checkMachine, closedConnections, pinnedToolchain, satisfies, versionIn } from "./check-machine.mjs";

const NOW = 2_000_000_000;
const ROOT = "/work/lyonoyl";
const HOME = "/work/account";

const TOOLCHAIN = [
  "[toolchain]",
  'channel = "1.98.1"',
  'components = ["clippy", "rustfmt", "llvm-tools"]',
].join("\n");

const PACKAGE = JSON.stringify({ engines: { node: "^20.19.0 || ^22.12.0 || >=24.0.0" } });

/** What every command on a complete Mac answers. */
const COMPLETE = {
  "xcode-select -p": "/Library/Developer/CommandLineTools",
  "sw_vers -productVersion": "26.1",
  "npm ls --depth=0": "lyonoyl@1.0.0",
  "rustup --version": "rustup 1.29.0",
  "rustup toolchain list": "stable-aarch64-apple-darwin\n1.98.1-aarch64-apple-darwin (active)",
  "rustup component list --installed --toolchain 1.98.1":
    "cargo-aarch64-apple-darwin\nclippy-aarch64-apple-darwin\nllvm-tools-aarch64-apple-darwin\nrustfmt-aarch64-apple-darwin",
  "rustc --version": "rustc 1.98.1 (48a229cea)",
  [`${HOME}/.cargo/bin/cargo-audit --version`]: "cargo-audit 0.22.2",
  [`${HOME}/.cargo/bin/cargo-about --version`]: `cargo-about ${ABOUT}`,
  "python3 --version": "Python 3.14.2",
  "git --version": "git version 2.50.1 (Apple Git-155)",
  "docker --version": "Docker version 29.8.1, build 4a63305",
  "docker compose version": "Docker Compose version v5.5.1",
  "sysctl -n kern.boottime": `{ sec = ${NOW - 3 * 86400}, usec = 0 }`,
};

/**
 * A machine that answers from a table: a command it does not list fails, as one that is not
 * installed does. `answers` replaces or removes (with `null`) entries of the complete Mac's.
 */
function machine({ platform = "darwin", answers = {}, files = [], node = "v20.20.0", netstat = [] } = {}) {
  const table = { ...COMPLETE, ...answers };
  const ran = [];
  const said = [];
  const waited = [];
  let listing = 0;
  return {
    ran,
    said,
    waited,
    platform,
    root: ROOT,
    home: HOME,
    env: { PATH: "/usr/bin:/bin" },
    node,
    exists: (path) =>
      [`${ROOT}/node_modules`, `${HOME}/.cargo/bin/cargo-audit`, `${HOME}/.cargo/bin/cargo-about`, ...files].includes(path),
    read: (path) => ({ "package.json": PACKAGE, "rust-toolchain.toml": TOOLCHAIN })[path],
    run: (command, args) => {
      const asked = [command, ...args].join(" ");
      ran.push(asked);
      if (asked === "netstat -anp tcp") return { ok: true, out: (netstat[listing++] ?? netstat.at(-1) ?? []).join("\n") };
      const out = table[asked];
      return out == null ? { ok: false, out: `${command}: command not found` } : { ok: true, out };
    },
    now: () => NOW,
    wait: async (seconds) => waited.push(seconds),
    say: (text) => said.push(text),
  };
}

const named = (answers) => answers.map((answer) => [answer.ok, answer.name]);
const lacking = (answers) => answers.filter((answer) => !answer.ok);

describe("what this machine has, asked", () => {
  it("finds everything on a complete Mac, and waits for nothing", async () => {
    const mac = machine();

    const answers = await checkMachine(mac);

    expect(lacking(answers)).toEqual([]);
    expect(named(answers)).toEqual([
      [true, "Xcode Command Line Tools"],
      [true, "macOS 26.1"],
      [true, "Node.js 20.20.0"],
      [true, "npm packages"],
      [true, "Rust 1.98.1, with clippy, rustfmt, llvm-tools"],
      [true, "rustc on PATH is 1.98.1, as rust-toolchain.toml pins"],
      [true, "cargo-audit 0.22.2"],
      [true, `cargo-about ${ABOUT}`],
      [true, "Python 3.14.2"],
      [true, "git 2.50.1"],
      [true, "Docker 29.8.1, Compose 5.5.1"],
      [true, "closed connections are cleared: 3.0 days since the last restart"],
    ]);
    expect(mac.waited).toEqual([]);
  });

  it("names everything that is missing in one run, each with how to get it", async () => {
    const mac = machine({
      answers: { "python3 --version": null, "docker --version": null, "xcode-select -p": null },
    });

    const answers = lacking(await checkMachine(mac));

    expect(answers.map((answer) => [answer.name, answer.fix])).toEqual([
      ["the Xcode Command Line Tools are not installed", "xcode-select --install"],
      ["python3 is not installed", "xcode-select --install"],
      [
        "Docker is not installed, and the development mail server runs in it",
        "Docker Desktop: https://docs.docker.com/desktop/setup/install/mac-install/",
      ],
    ]);
  });

  it("asks only what a build needs where it is asked for a build", async () => {
    const mac = machine({ answers: { "docker --version": null, "python3 --version": null } });

    const answers = await checkMachine(mac, { build: true });

    expect(lacking(answers)).toEqual([]);
    expect(answers.map((answer) => answer.name).at(-1)).toBe("rustc on PATH is 1.98.1, as rust-toolchain.toml pins");
    expect(mac.ran.some((command) => /cargo-audit|cargo-about|docker|python3|git |sysctl/.test(command))).toBe(false);
  });

  it("installs nothing while it asks", async () => {
    const mac = machine({ answers: { "rustup toolchain list": "stable-aarch64-apple-darwin" } });

    await checkMachine(mac);

    expect(mac.ran.filter((command) => / install\b/.test(command))).toEqual([]);
  });

  it("says each answer as it comes, the missing ones with their fix", async () => {
    const mac = machine({ answers: { "git --version": null } });

    await checkMachine(mac);

    expect(mac.said).toContain("  \u001b[31m✗ git is not installed\u001b[0m\n      → xcode-select --install\n");
  });
});

describe("Node.js and the npm packages", () => {
  it.each([
    ["v20.18.9", false],
    ["v20.19.0", true],
    ["v21.7.3", false],
    ["v22.11.0", false],
    ["v22.12.0", true],
    ["v23.9.0", false],
    ["v24.0.0", true],
    ["v25.1.0", true],
  ])("takes %s as %s against the range package.json names", async (node, ok) => {
    const answers = await checkMachine(machine({ node }));

    expect(answers.find((answer) => answer.name.startsWith("Node.js")).ok).toBe(ok);
  });

  it("refuses a range it cannot read rather than passing it", () => {
    expect(() => satisfies("20.19.0", "~20.19.0")).toThrow("~20.19.0 is a range this check does not read");
  });

  it("asks for npm ci where the packages are not installed at all", async () => {
    const mac = machine();
    mac.exists = (path) => path !== `${ROOT}/node_modules` && path.includes("/.cargo/bin/");

    const answer = lacking(await checkMachine(mac))[0];

    expect([answer.name, answer.fix]).toEqual(["npm packages are not installed", "npm ci"]);
  });

  it("asks for npm ci where the installed packages do not match package.json", async () => {
    const mac = machine({ answers: { "npm ls --depth=0": null } });

    const answer = lacking(await checkMachine(mac))[0];

    expect([answer.name, answer.fix]).toEqual(["npm packages do not match package.json", "npm ci"]);
  });
});

describe("Rust", () => {
  it("reads the pinned channel and its components from rust-toolchain.toml", () => {
    expect(pinnedToolchain(TOOLCHAIN)).toEqual({ channel: "1.98.1", components: ["clippy", "rustfmt", "llvm-tools"] });
  });

  it("asks for the pinned toolchain with its components where it is not installed", async () => {
    const mac = machine({ answers: { "rustup toolchain list": "stable-aarch64-apple-darwin" } });

    const answers = lacking(await checkMachine(mac));

    expect(answers.map((answer) => [answer.name, answer.fix])).toEqual([
      [
        "Rust 1.98.1, which rust-toolchain.toml pins, is not installed",
        "rustup toolchain install 1.98.1 --component clippy,rustfmt,llvm-tools",
      ],
    ]);
  });

  it("does not report cargo's tools missing because the toolchain is", async () => {
    const mac = machine({ answers: { "rustup toolchain list": "", "rustc --version": null } });

    const answers = await checkMachine(mac);

    expect(answers.find((answer) => answer.name.startsWith("cargo-audit")).ok).toBe(true);
    expect(answers.some((answer) => answer.name.startsWith("rustc on PATH"))).toBe(false);
  });

  it("names a component the pinned toolchain lacks", async () => {
    const mac = machine({
      answers: { "rustup component list --installed --toolchain 1.98.1": "clippy-aarch64-apple-darwin\nrustfmt-aarch64-apple-darwin" },
    });

    expect(lacking(await checkMachine(mac))[0].name).toBe("Rust 1.98.1 lacks llvm-tools");
  });

  it("names the directory of rustup's own cargo and rustc where another rustc is first on PATH", async () => {
    const mac = machine({ answers: { "rustc --version": "rustc 1.99.0 (aaaaaaaaa)" }, files: ["/opt/homebrew/opt/rustup/bin/rustup"] });

    const answer = lacking(await checkMachine(mac))[0];

    expect(answer.name).toBe("rustc on PATH is 1.99.0, and the tree pins 1.98.1");
    expect(answer.fix).toBe(
      "put /opt/homebrew/opt/rustup/bin first on PATH: only rustup's own cargo and rustc read rust-toolchain.toml",
    );
  });

  it("asks for rustup where it is not installed, the way a Mac installs it", async () => {
    const answer = lacking(await checkMachine(machine({ answers: { "rustup --version": null } })))[0];

    expect(answer.fix).toBe("brew install rustup, and put $(brew --prefix rustup)/bin first on PATH; or https://rustup.rs");
  });

  it("finds cargo's tools where cargo does, in the bin directory CARGO_HOME names", async () => {
    const mac = machine({ answers: { "/cargo/bin/cargo-audit --version": "cargo-audit 0.21.0" } });
    mac.env = { PATH: "/usr/bin", CARGO_HOME: "/cargo" };
    mac.exists = (path) => [`${ROOT}/node_modules`, "/cargo/bin/cargo-audit", `${HOME}/.cargo/bin/cargo-about`].includes(path);

    const answers = await checkMachine(mac);

    expect(answers.find((answer) => answer.name.startsWith("cargo-audit")).name).toBe("cargo-audit 0.21.0");
    expect(answers.find((answer) => answer.name.startsWith("cargo-about")).ok).toBe(false);
  });

  it("asks for cargo-about at the version the licence list is collected with", async () => {
    const mac = machine();
    mac.exists = (path) => [`${ROOT}/node_modules`, `${HOME}/.cargo/bin/cargo-audit`].includes(path);

    const answer = lacking(await checkMachine(mac))[0];

    expect([answer.name, answer.fix]).toEqual(["cargo-about is not installed", `cargo install cargo-about --locked --version ${ABOUT}`]);
  });

  it("notes another cargo-about without failing on it", async () => {
    const mac = machine({ answers: { [`${HOME}/.cargo/bin/cargo-about --version`]: "cargo-about 0.10.1" } });

    const answer = (await checkMachine(mac)).find((one) => one.name.startsWith("cargo-about"));

    expect(answer.ok).toBe(true);
    expect(answer.note).toContain(`collected with ${ABOUT}`);
  });
});

describe("Docker and the system", () => {
  it("asks for Compose where Docker has none", async () => {
    const answer = lacking(await checkMachine(machine({ answers: { "docker compose version": null } })))[0];

    expect(answer.name).toBe("Docker 29.8.1 has no Compose, and the mail server is started with it");
  });

  it("reads a version written with a v before it", () => {
    expect(versionIn("Docker Compose version v5.5.1")).toBe("5.5.1");
  });

  it("names where Windows gets Python and git", async () => {
    const box = machine({
      platform: "win32",
      answers: { "python3 --version": null, "git --version": null },
      files: [`${HOME}/.cargo/bin/cargo-audit.exe`, `${HOME}/.cargo/bin/cargo-about.exe`],
    });

    const answers = lacking(await checkMachine(box));

    expect(answers.map((answer) => answer.fix)).toEqual([
      "https://www.python.org/downloads/windows/",
      "https://git-scm.com/download/win",
    ]);
  });

  it("refuses a macOS older than a build is made on", async () => {
    const answer = lacking(await checkMachine(machine({ answers: { "sw_vers -productVersion": "12.7.6" } })))[0];

    expect([answer.name, answer.fix]).toEqual(["macOS 12.7.6, where 13 Ventura or newer is needed", "update macOS"]);
  });
});

describe("Linux", () => {
  const LINUX = {
    "cc --version": "cc (Ubuntu 13.2.0) 13.2.0",
    "pkg-config --version": "1.8.1",
    "pkg-config --exists webkit2gtk-4.1": "",
    "pkg-config --exists gtk+-3.0": "",
    "pkg-config --exists libsoup-3.0": "",
    "pkg-config --exists ayatana-appindicator3-0.1": "",
    "pkg-config --exists librsvg-2.0": "",
    "pkg-config --exists openssl": "",
    "pkg-config --modversion webkit2gtk-4.1": "2.48.3",
  };

  it("finds the libraries a Linux build links against, and asks nothing a Mac is asked", async () => {
    const box = machine({ platform: "linux", answers: LINUX });

    const answers = await checkMachine(box);

    expect(lacking(answers)).toEqual([]);
    expect(answers[0].name).toBe("WebKitGTK 2.48.3, and the other libraries the build links against");
    expect(box.ran.some((command) => command.startsWith("sysctl") || command.startsWith("xcode-select"))).toBe(false);
  });

  it("names the packages of exactly the libraries that are missing", async () => {
    const box = machine({
      platform: "linux",
      answers: { ...LINUX, "pkg-config --exists webkit2gtk-4.1": null, "pkg-config --exists librsvg-2.0": null },
    });

    const answer = lacking(await checkMachine(box))[0];

    expect(answer.name).toBe("libraries the build links against are missing: webkit2gtk-4.1, librsvg-2.0");
    expect(answer.fix).toBe("on Debian and Ubuntu, sudo apt-get install libwebkit2gtk-4.1-dev librsvg2-dev");
  });

  it("refuses a WebKitGTK older than 2.44", async () => {
    const box = machine({ platform: "linux", answers: { ...LINUX, "pkg-config --modversion webkit2gtk-4.1": "2.42.5" } });

    expect(lacking(await checkMachine(box))[0].name).toBe("WebKitGTK 2.42.5, where 2.44 or newer is needed");
  });

  it("asks for pkg-config before it can ask about a library", async () => {
    const box = machine({ platform: "linux", answers: { ...LINUX, "pkg-config --version": null } });

    const answers = lacking(await checkMachine(box));

    expect(answers.map((answer) => answer.fix)).toEqual(["sudo apt-get install pkg-config"]);
  });
});

describe("whether this Mac still clears closed connections", () => {
  const boot = (secondsAgo) => ({ "sysctl -n kern.boottime": `{ sec = ${NOW - secondsAgo}, usec = 0 }`, "sysctl -n net.inet.tcp.msl": "15000" });
  const held = (count, from = 0) =>
    Array.from({ length: count }, (_, at) => `tcp4  0  0  127.0.0.1.${50000 + from + at}  127.0.0.1.1143  TIME_WAIT  `);

  it("is not watched on a Mac that has run less than 2^32 ms", async () => {
    const mac = machine({ answers: boot(Math.floor(WRAPS_AFTER) - 1), netstat: [held(100)] });

    const answer = await closedConnections(mac);

    expect(answer.ok).toBe(true);
    expect(mac.waited).toEqual([]);
  });

  it("is watched for twice the segment lifetime and a margin from 2^32 ms of uptime on", async () => {
    const mac = machine({ answers: boot(Math.ceil(WRAPS_AFTER)), netstat: [held(100), []] });

    const answer = await closedConnections(mac);

    expect(answer).toEqual({ ok: true, name: "closed connections are cleared: 49.7 days since the last restart" });
    expect(mac.waited).toEqual([35]);
  });

  it("finds a Mac that keeps them, and says to restart it", async () => {
    const mac = machine({ answers: boot(67 * 86400), netstat: [held(100), held(100)] });

    const answer = await closedConnections(mac);

    expect(answer.ok).toBe(false);
    expect(answer.name).toBe("this Mac has run 67 days without a restart and no longer clears closed connections");
    expect(answer.why[0]).toBe("100 of the 100 closed connections held 35 s ago are still held, though macOS lets one go after 30 s.");
    expect(answer.fix).toBe("restart the Mac, and run this again");
  });

  it("takes half of them gone as a Mac that clears them, and more than half kept as one that does not", async () => {
    const half = machine({ answers: boot(60 * 86400), netstat: [held(100), held(50)] });
    const most = machine({ answers: boot(60 * 86400), netstat: [held(100), held(51)] });

    expect((await closedConnections(half)).ok).toBe(true);
    expect((await closedConnections(most)).ok).toBe(false);
  });

  it("counts only the connections it saw at first, not the ones that closed while it watched", async () => {
    const mac = machine({ answers: boot(60 * 86400), netstat: [held(100), held(1000, 100)] });

    expect((await closedConnections(mac)).ok).toBe(true);
  });

  it("does not wait where no connection is held at all", async () => {
    const mac = machine({ answers: boot(60 * 86400), netstat: [[]] });

    const answer = await closedConnections(mac);

    expect(answer.ok).toBe(true);
    expect(mac.waited).toEqual([]);
  });

  it("does not wait where the uptime cannot be read", async () => {
    const mac = machine({ answers: { "sysctl -n kern.boottime": null }, netstat: [held(100)] });

    expect((await closedConnections(mac)).name).toBe("closed connections: uptime unknown, not asked");
    expect(mac.waited).toEqual([]);
  });
});
