// One number for everything in this tree a check reads, so that a run can tell the tree in front of
// it from the one the preflight last passed.
//
// What goes in is every file git would carry: each one it tracks, as it stands on disk rather than
// as it was last committed, and each one it would add, which leaves out only what `.gitignore` leaves
// out. Each goes in with its path, whether it may be run, and every byte, so a file renamed, made
// runnable, or changed by one byte makes another tree, and so does one added or taken away. Two files
// git does not carry go in as well, because a run reads them: what npm installed,
// `node_modules/.package-lock.json`, and the Cargo settings kept for this tree alone,
// `.cargo/local.toml`.
//
//     node scripts/tree-fingerprint.mjs                         prints it
//     node scripts/tree-fingerprint.mjs --record <file> <it>    writes into <file> that this tree passed,
//                                                               where it is still the tree <it> was
//                                                               taken of, and otherwise removes <file>
//     node scripts/tree-fingerprint.mjs --passed <file>         whether <file> holds a pass of this tree
//
// `--passed` answers on one line: the seconds since the pass where the tree is the one that passed,
// and the status 0; `changed` and the seconds since the pass where it is another, or `none` where
// there is no pass to compare with, and the status 1. Anything that stops it from reading the tree
// is said on one line, with the status 2.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readlinkSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

/** The two files outside git's view that a run reads. */
export const BESIDE = ["node_modules/.package-lock.json", ".cargo/local.toml"];

/** Every file git tracks in the tree at `root`, and every one it would add. */
export function filesOf(root) {
  const listed = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
    cwd: root,
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
  return listed.toString("utf8").split("\0").filter(Boolean);
}

/**
 * The fingerprint of `paths` under `root`, whatever order they are given in. A path that is not
 * there is left out, which is what a deleted file is.
 *
 * Each file goes in after its path, what it is, and its length, so where one file ends and the next
 * begins is never in doubt.
 */
export function fingerprint(root, paths) {
  const hash = createHash("sha256");
  for (const path of [...new Set(paths)].sort()) {
    const at = join(root, path);
    let what;
    try {
      what = lstatSync(at);
    } catch {
      continue;
    }
    let kind = "other";
    let bytes = Buffer.alloc(0);
    if (what.isSymbolicLink()) {
      kind = "link";
      bytes = Buffer.from(readlinkSync(at), "utf8");
    } else if (what.isFile()) {
      kind = what.mode & 0o111 ? "runs" : "file";
      bytes = readFileSync(at);
    }
    hash.update(`${path}\0${kind}\0${bytes.length}\0`);
    hash.update(bytes);
  }
  return hash.digest("hex");
}

/** The fingerprint of the tree at `root`, as the preflight takes it. */
export function treeFingerprint(root) {
  return fingerprint(root, [...filesOf(root), ...BESIDE]);
}

/** What a record of a pass says, or nothing where `file` holds none. */
export function readRecord(file) {
  try {
    const { tree, passed } = JSON.parse(readFileSync(file, "utf8"));
    if (typeof tree === "string" && /^[0-9a-f]{64}$/.test(tree) && Number.isInteger(passed)) {
      return { tree, passed };
    }
  } catch {
    // No record, or one that is not a record: either way, nothing passed.
  }
  return undefined;
}

/**
 * Writes into `file` that the tree at `root` passed, where it is still the tree `taken` was the
 * fingerprint of, and says whether it did. Where it is not, `file` is removed, so an older pass of
 * another tree is not left standing for this one.
 */
export function record(root, file, taken, now = Date.now()) {
  if (treeFingerprint(root) !== taken) {
    rmSync(file, { force: true });
    return false;
  }
  const writing = `${file}.${process.pid}`;
  writeFileSync(writing, `${JSON.stringify({ tree: taken, passed: Math.floor(now / 1000) })}\n`);
  renameSync(writing, file);
  return true;
}

/** Whether `file` holds a pass of the tree at `root`, as `--passed` answers it. */
export function passed(root, file, now = Date.now()) {
  const kept = readRecord(file);
  if (!kept) return { same: false, said: "none" };
  const ago = Math.max(0, Math.floor(now / 1000) - kept.passed);
  return treeFingerprint(root) === kept.tree ? { same: true, said: `${ago}` } : { same: false, said: `changed ${ago}` };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [how, file, taken] = process.argv.slice(2);
  const root = process.cwd();
  try {
    if (how === undefined) {
      console.log(treeFingerprint(root));
    } else if (how === "--record" && file && taken) {
      process.exitCode = record(root, file, taken) ? 0 : 1;
    } else if (how === "--passed" && file) {
      const answer = passed(root, file);
      console.log(answer.said);
      process.exitCode = answer.same ? 0 : 1;
    } else {
      console.error("node scripts/tree-fingerprint.mjs [--record <file> <fingerprint> | --passed <file>]");
      process.exitCode = 2;
    }
  } catch (failed) {
    const said = failed?.stderr?.toString().trim() || failed?.message || String(failed);
    console.error(`The tree here cannot be read: ${said.split("\n")[0]}`);
    process.exitCode = 2;
  }
}
