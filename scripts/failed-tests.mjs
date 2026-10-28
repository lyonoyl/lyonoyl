// Which tests a run failed: every one of them, where it stopped and what it said, and why where the
// reason is one this tree knows.
//
//   node scripts/failed-tests.mjs rust target/preflight/rust-tests.log
//   node scripts/failed-tests.mjs frontend target/preflight/frontend-tests.json
//
// The first reads what `cargo test --no-fail-fast` printed, the second the report vitest writes with
// `--reporter=json`. The list of failed tests is never shortened; a line of what a test said is.

import { existsSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const COLOUR = /\u001b\[[0-9;]*[A-Za-z]/g;
const TARGET = /^\s+(?:Running (?:unittests )?(\S+) \(|Doc-tests (\S+)$)/;
const FAILED = /^test (.+?) \.\.\. FAILED$/;
const CAPTURED = /^---- (.+) stdout ----$/;
const RESULT = /^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;/;
const RERUN = /^error: (?:test|doctest) failed, to rerun pass `(.+)`$/;
const EXITED = /process didn't exit successfully: .* \((.+)\)$/;
const PANIC = /^thread '.*?'(?: \(\d+\))? panicked at (.+?):$/;
const NOT_PANICKED = /^note: test did not panic as expected at (.+)$/;
const COMPILE = /^error(?:\[E\d+\])?: (.+)$/;
const POINTER = /^\s*--> (.+)$/;
const STACK = /^\s+at /;

/** The longest a line of what a test said is shown here. The whole of it is in the log. */
const WIDE = 200;

/** As many lines of what one test said as are shown here. */
const LINES = 3;

/**
 * Failures whose cause this tree knows, by what the test said. The first that matches explains it.
 * `$1` in `why` is the address the message names.
 */
const CAUSES = [
  {
    said: /nothing is answering on (\S+): the connection was refused/,
    why: "nothing was listening on $1: the development mail server stopped during the run. `docker ps -a` shows whether its two containers are up.",
  },
  {
    said: /nothing is answering on (\S+): no connection within/,
    why: "the development mail server was listening, and a connection to $1 did not complete in time: something on this machine held it. Nothing the test is about was reached.",
  },
  {
    said: /os error (?:49|99|10049)\b|can(?:'|no)t assign requested address|AddrNotAvailable|address not available/i,
    why: "this machine had no local port free for a new connection.",
  },
  {
    said: /LYONOYL_MAIL_SERVER_REQUIRED is set and nothing is answering on (\S+?)[.:]?(?:\s|$)/,
    why: "the check that the development mail server answers on $1 failed, before anything the test is about ran.",
  },
];

/** What a `cargo test` run printed, read back: totals, failed tests, and test programs that died. */
export function rustRun(log, where = (at) => at) {
  const run = { passed: 0, failed: 0, ignored: 0, targets: 0, failures: [], crashes: [], unfinished: [], unbuilt: [] };
  const targets = [];
  let target = null;
  let captured = null;

  const close = () => {
    if (captured) target.told.set(captured.name, story(captured.lines));
    captured = null;
  };

  for (const line of log.replace(COLOUR, "").split(/\r?\n/)) {
    let found;
    if ((found = TARGET.exec(line))) {
      if (target) close();
      target = { path: found[1] ?? `doc-tests ${found[2]}`, rerun: null, failed: [], told: new Map(), reported: false, exited: null };
      targets.push(target);
    } else if (!target) {
      if ((found = COMPILE.exec(line))) run.unbuilt.push({ said: found[1], at: null });
      else if ((found = POINTER.exec(line)) && run.unbuilt.length && !run.unbuilt.at(-1).at) run.unbuilt.at(-1).at = found[1];
    } else if ((found = CAPTURED.exec(line))) {
      close();
      captured = { name: named(found[1]), lines: [] };
    } else if (line === "failures:") {
      close();
    } else if ((found = RESULT.exec(line))) {
      close();
      target.reported = true;
      run.passed += Number(found[1]);
      run.failed += Number(found[2]);
      run.ignored += Number(found[3]);
    } else if ((found = RERUN.exec(line))) {
      close();
      target.rerun = found[1];
    } else if ((found = EXITED.exec(line))) {
      target.exited = found[1];
    } else if ((found = FAILED.exec(line))) {
      target.failed.push(named(found[1]));
    } else if (captured) {
      captured.lines.push(line);
    }
  }
  if (target) close();

  run.targets = targets.length;
  for (const each of targets) {
    const program = each.rerun ? `cargo test ${each.rerun}` : each.path;
    for (const name of each.failed) {
      const told = each.told.get(name) ?? { at: null, said: [] };
      run.failures.push({ program, name, at: told.at && where(told.at), said: told.said });
    }
    if (!each.reported) (each.exited ? run.crashes : run.unfinished).push({ program, why: each.exited });
  }
  return run;
}

/** A test's name as its result line gives it, without what libtest adds to a `should_panic` one. */
function named(name) {
  return name.replace(/ - should panic$/, "");
}

/** Where a test stopped and what it said, from what it printed. */
function story(lines) {
  for (const [at, line] of lines.entries()) {
    let found;
    if ((found = PANIC.exec(line))) {
      const said = [];
      for (const next of lines.slice(at + 1)) {
        if (next === "" || next.startsWith("note: ") || next.startsWith("stack backtrace:")) break;
        said.push(next);
        if (said.length === LINES) break;
      }
      return { at: found[1], said };
    }
    if ((found = NOT_PANICKED.exec(line))) return { at: found[1], said: ["did not panic as expected"] };
  }
  const first = lines.find((line) => line.trim() !== "");
  return { at: null, said: first ? [first] : [] };
}

/**
 * Turns `src/lib.rs:31:9` from a panic, which is relative to its crate, into a path from the
 * repository's root, where exactly one workspace member holds that file.
 */
export function inTheTree(root) {
  const manifest = existsSync(join(root, "Cargo.toml")) ? readFileSync(join(root, "Cargo.toml"), "utf8") : "";
  const listed = /members\s*=\s*\[([^\]]*)\]/.exec(manifest)?.[1] ?? "";
  const members = [...listed.matchAll(/"([^"]+)"/g)].map((member) => member[1]);
  return (at) => {
    const [, file, line] = /^(.+?):(\d+)(?::\d+)?$/.exec(at) ?? [];
    if (!file) return at;
    if (existsSync(join(root, file))) return `${file}:${line}`;
    const holding = members.map((member) => `${member}/${file}`).filter((path) => existsSync(join(root, path)));
    return `${holding.length === 1 ? holding[0] : file}:${line}`;
  };
}

/** What a vitest JSON report holds, read back: totals, failed tests, and files that did not load. */
export function frontendRun(report, root) {
  const run = { passed: report.numPassedTests ?? 0, failed: report.numFailedTests ?? 0, failures: [], unloaded: [] };
  const local = (text) => text.split(`${root}/`).join("");
  for (const file of report.testResults ?? []) {
    const path = relative(root, file.name);
    const failed = (file.assertionResults ?? []).filter((test) => test.status === "failed");
    for (const test of failed) {
      const name = [...(test.ancestorTitles ?? []), test.title].join(" › ");
      run.failures.push({ file: path, name, said: message(local(test.failureMessages?.[0] ?? "")) });
    }
    if (file.status === "failed" && failed.length === 0) run.unloaded.push({ file: path, said: message(local(file.message ?? "")) });
  }
  return run;
}

/** The lines of an error before its stack. */
function message(text) {
  const said = [];
  for (const line of text.split(/\r?\n/)) {
    if (STACK.test(line)) break;
    if (line.trim() !== "") said.push(line);
    if (said.length === LINES) break;
  }
  return said;
}

/** One line of what a test said, cut to a width a terminal shows on one line or two. */
function cut(line) {
  return line.length > WIDE ? `${line.slice(0, WIDE - 1)}…` : line;
}

/** Why the failures happened, where what they said names a cause this tree knows. */
export function causes(failures) {
  const found = new Map();
  for (const failure of failures) {
    const said = failure.said.join("\n");
    for (const cause of CAUSES) {
      const matched = cause.said.exec(said);
      if (!matched) continue;
      const why = cause.why.replace("$1", matched[1] ?? "");
      found.set(why, (found.get(why) ?? 0) + 1);
      break;
    }
  }
  const explained = [...found.values()].reduce((sum, count) => sum + count, 0);
  const lines = [...found].map(([why, count]) =>
    count === failures.length ? `Why: ${why}` : `Why, for ${count} of them: ${why}`,
  );
  if (explained > 0 && explained < failures.length) {
    lines.push(`For the other ${failures.length - explained}, no reason this summary knows: what each said is above.`);
  }
  return lines;
}

const plural = (count, one, many) => `${count} ${count === 1 ? one : many}`;

/** A Rust run as the lines the end of the preflight shows. */
export function describeRust(run, log) {
  if (run.targets === 0) {
    if (run.unbuilt.length === 0) return [`cargo test printed no test results. What it did print: ${log}`];
    return [
      "The tests did not compile:",
      ...run.unbuilt.slice(0, 5).map((error) => `  ${cut(error.said)}${error.at ? `  (${error.at})` : ""}`),
      `The whole output: ${log}`,
    ];
  }

  const ran = run.passed + run.failed;
  const head = [`${run.failed} of ${ran} failed`];
  if (run.crashes.length) head.push(`${plural(run.crashes.length, "test program", "test programs")} died before reporting`);
  if (run.unfinished.length) head.push(`${plural(run.unfinished.length, "test program", "test programs")} never reported`);
  const lines = [head.join(", and ")];

  const programs = [...new Set([...run.failures, ...run.crashes, ...run.unfinished].map((one) => one.program))];
  for (const program of programs) {
    lines.push(`  ${program}`);
    for (const failure of run.failures.filter((one) => one.program === program)) {
      lines.push(`    ${failure.name}`);
      if (failure.at) lines.push(`      at ${failure.at}`);
      for (const said of failure.said) lines.push(`      ${cut(said)}`);
    }
    for (const crash of run.crashes.filter((one) => one.program === program)) {
      lines.push(`    the program died (${crash.why}): which test it was running is not reported`);
    }
    for (const unfinished of run.unfinished.filter((one) => one.program === program)) {
      lines.push(`    the program printed no result: the run ended before it did`);
    }
  }
  lines.push(...causes(run.failures), `The whole output: ${log}`);
  return lines;
}

/** A frontend run as the lines the end of the preflight shows. */
export function describeFrontend(run, report) {
  const head = [`${run.failed} of ${run.passed + run.failed} failed`];
  if (run.unloaded.length) head.push(`${plural(run.unloaded.length, "file", "files")} did not load`);
  const lines = [head.join(", and ")];

  const files = [...new Set([...run.failures, ...run.unloaded].map((one) => one.file))];
  for (const file of files) {
    lines.push(`  ${file}`);
    for (const failure of run.failures.filter((one) => one.file === file)) {
      lines.push(`    ${failure.name}`);
      for (const said of failure.said) lines.push(`      ${cut(said)}`);
    }
    for (const unloaded of run.unloaded.filter((one) => one.file === file)) {
      lines.push("    did not load");
      for (const said of unloaded.said) lines.push(`      ${cut(said)}`);
    }
  }
  lines.push(`The whole report: ${report}`);
  return lines;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [kind, file] = process.argv.slice(2);
  const root = process.cwd();
  if (!existsSync(file ?? "")) {
    console.log(`Nothing to read: ${file ?? "no file was named"} is not there.`);
  } else if (kind === "rust") {
    console.log(describeRust(rustRun(readFileSync(file, "utf8"), inTheTree(root)), file).join("\n"));
  } else if (kind === "frontend") {
    console.log(describeFrontend(frontendRun(JSON.parse(readFileSync(file, "utf8")), root), file).join("\n"));
  } else {
    console.error("usage: node scripts/failed-tests.mjs rust|frontend FILE");
    process.exit(2);
  }
}
