import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { BESIDE, filesOf, fingerprint, passed, readRecord, record, treeFingerprint } from "./tree-fingerprint.mjs";

const SCRIPT = resolve("scripts/tree-fingerprint.mjs");
const POSIX = process.platform !== "win32";

const made = [];
afterEach(() => {
  while (made.length) rmSync(made.pop(), { recursive: true, force: true });
});

/** A directory holding `files`, each a path and what it holds. */
function directory(files) {
  const root = mkdtempSync(join(tmpdir(), "tree-fingerprint-"));
  made.push(root);
  for (const [path, holds] of Object.entries(files)) write(root, path, holds);
  return root;
}

function write(root, path, holds) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), holds);
}

/** Where a record is kept: outside the tree it is about, as `target/` is outside what git carries. */
function elsewhere() {
  return join(directory({}), "passed");
}

/** A git repository with nothing committed, so that every file in it is one git would add. */
function repository(files) {
  const root = directory(files);
  execFileSync("git", ["init", "-q"], { cwd: root });
  return root;
}

describe("the fingerprint of a set of files", () => {
  it("is the same for the same files, in whatever order they are named", () => {
    const root = directory({ "a.txt": "one", "b/c.txt": "two" });

    expect(fingerprint(root, ["a.txt", "b/c.txt"])).toBe(fingerprint(root, ["b/c.txt", "a.txt", "a.txt"]));
    expect(fingerprint(root, ["a.txt"])).toMatch(/^[0-9a-f]{64}$/);
  });

  it("changes with one byte of one file", () => {
    const root = directory({ "a.txt": "one", "b.txt": "two" });
    const before = fingerprint(root, ["a.txt", "b.txt"]);

    write(root, "b.txt", "twO");

    expect(fingerprint(root, ["a.txt", "b.txt"])).not.toBe(before);
  });

  it("changes with a file added, and leaves out a path that is not there", () => {
    const root = directory({ "a.txt": "one" });
    const before = fingerprint(root, ["a.txt"]);

    expect(fingerprint(root, ["a.txt", "gone.txt"])).toBe(before);
    write(root, "gone.txt", "");
    expect(fingerprint(root, ["a.txt", "gone.txt"])).not.toBe(before);
  });

  it("changes with a file renamed and nothing in it changed", () => {
    const root = directory({ "a.txt": "one" });
    const before = fingerprint(root, ["a.txt", "z.txt"]);

    renameSync(join(root, "a.txt"), join(root, "z.txt"));

    expect(fingerprint(root, ["a.txt", "z.txt"])).not.toBe(before);
  });

  it("tells where one file ends and the next begins", () => {
    // One file holding what two files would put in a row, the second one's path included.
    const one = directory({ "a.txt": "x" + "b.txt\0file\0" + "y" });
    const other = directory({ "a.txt": "x", "b.txt": "y" });

    expect(fingerprint(one, ["a.txt", "b.txt"])).not.toBe(fingerprint(other, ["a.txt", "b.txt"]));
  });

  it("is a fingerprint of nothing at all, rather than a failure, where no file is named", () => {
    const root = directory({});

    expect(fingerprint(root, [])).toBe(fingerprint(root, ["not-there.txt"]));
  });

  it.runIf(POSIX)("changes with a file made runnable", () => {
    const root = directory({ "run.sh": "echo" });
    const before = fingerprint(root, ["run.sh"]);

    chmodSync(join(root, "run.sh"), 0o755);

    expect(fingerprint(root, ["run.sh"])).not.toBe(before);
  });

  it.runIf(POSIX)("holds a link by where it points, not by what it points at", () => {
    const root = directory({ "one.txt": "same", "two.txt": "same" });
    symlinkSync("one.txt", join(root, "link"));
    const before = fingerprint(root, ["link"]);

    write(root, "one.txt", "changed");
    expect(fingerprint(root, ["link"])).toBe(before);

    rmSync(join(root, "link"));
    symlinkSync("two.txt", join(root, "link"));
    expect(fingerprint(root, ["link"])).not.toBe(before);
  });
});

describe("the tree git sees", () => {
  it("is every file git would add, and none it ignores", () => {
    const root = repository({ ".gitignore": "ignored/\n", "a.txt": "one", "ignored/x.txt": "two", "b/c.txt": "three" });

    expect(filesOf(root).sort()).toEqual([".gitignore", "a.txt", "b/c.txt"]);
  });

  it("does not change with what git ignores, and does with what npm installed and the local Cargo settings", () => {
    const root = repository({
      ".gitignore": "target/\nnode_modules/\n.cargo/local.toml\n",
      "a.txt": "one",
      "target/built": "one",
      "node_modules/.package-lock.json": "{}",
      "node_modules/some/index.js": "one",
    });
    const before = treeFingerprint(root);

    write(root, "target/built", "two");
    write(root, "node_modules/some/index.js", "two");
    expect(treeFingerprint(root)).toBe(before);

    write(root, "node_modules/.package-lock.json", '{"packages":{}}');
    const installed = treeFingerprint(root);
    expect(installed).not.toBe(before);

    write(root, ".cargo/local.toml", "[build]\n");
    expect(treeFingerprint(root)).not.toBe(installed);
    expect(BESIDE).toEqual(["node_modules/.package-lock.json", ".cargo/local.toml"]);
  });

  it("changes with a file git would add that was not there before", () => {
    const root = repository({ "a.txt": "one" });
    const before = treeFingerprint(root);

    write(root, "new.rs", "");

    expect(treeFingerprint(root)).not.toBe(before);
  });
});

describe("the record of a pass", () => {
  it("is written for the tree that was taken, and found again while the tree stays so", () => {
    const root = repository({ "a.txt": "one" });
    const file = elsewhere();
    const taken = treeFingerprint(root);

    expect(record(root, file, taken, 1_000_000)).toBe(true);
    expect(readRecord(file)).toEqual({ tree: taken, passed: 1000 });
    expect(passed(root, file, 1_090_000)).toEqual({ same: true, said: "90" });
  });

  it("is not written for a tree that changed while it was checked, and what was there goes", () => {
    const root = repository({ "a.txt": "one" });
    const file = elsewhere();
    expect(record(root, file, treeFingerprint(root), 1_000_000)).toBe(true);
    const taken = treeFingerprint(root);

    write(root, "a.txt", "two");

    expect(record(root, file, taken, 2_000_000)).toBe(false);
    expect(readRecord(file)).toBeUndefined();
  });

  it("does not hold for a tree changed since the pass", () => {
    const root = repository({ "a.txt": "one" });
    const file = elsewhere();
    record(root, file, treeFingerprint(root), 1_000_000);

    write(root, "a.txt", "two");

    expect(passed(root, file, 1_030_000)).toEqual({ same: false, said: "changed 30" });
  });

  it("is no pass at all where there is none, or where it is not a record", () => {
    const root = repository({ "a.txt": "one" });
    const file = elsewhere();

    expect(passed(root, file)).toEqual({ same: false, said: "none" });
    for (const kept of ["", "not json", '{"tree":"abc","passed":1}', `{"tree":"${"0".repeat(64)}","passed":"1"}`, "[]"]) {
      writeFileSync(file, kept);
      expect(passed(root, file)).toEqual({ same: false, said: "none" });
    }
  });

  it("is asked and written from the command line the way the preflight asks", () => {
    const root = repository({ "a.txt": "one" });
    const file = elsewhere();
    const run = (...args) => spawnSync(process.execPath, [SCRIPT, ...args], { cwd: root, encoding: "utf8" });
    const taken = run().stdout.trim();

    expect(run("--passed", file)).toMatchObject({ status: 1, stdout: "none\n" });
    expect(run("--record", file, taken).status).toBe(0);
    expect(run("--passed", file)).toMatchObject({ status: 0, stdout: expect.stringMatching(/^\d+\n$/) });

    write(root, "a.txt", "two");
    expect(run("--passed", file)).toMatchObject({ status: 1, stdout: expect.stringMatching(/^changed \d+\n$/) });
    expect(run("--record", file, taken).status).toBe(1);
    expect(() => readFileSync(file)).toThrow();
    expect(run("--something").status).toBe(2);
  });

  it("says it cannot read a tree that is not a git checkout, rather than calling it changed", () => {
    const root = directory({ "a.txt": "one" });
    const answer = spawnSync(process.execPath, [SCRIPT], {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, GIT_CEILING_DIRECTORIES: dirname(root) },
    });

    expect(answer.status).toBe(2);
    expect(answer.stderr).toMatch(/^The tree here cannot be read: /);
  });
});
