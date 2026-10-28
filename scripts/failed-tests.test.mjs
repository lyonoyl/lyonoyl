import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { causes, describeFrontend, describeRust, frontendRun, inTheTree, rustRun } from "./failed-tests.mjs";

// What `cargo test --no-fail-fast` printed for a crate whose tests fail in every way libtest
// reports: an assertion, a panic, a returned error, a `should_panic` that did not, a panic in a
// thread of the test's own, a test program that aborted, and a doc test.
const RUN = [
  "   Compiling sample v0.1.0 (/work/sample)",
  "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.37s",
  "     Running unittests src/lib.rs (target/debug/deps/sample-728e3d26d6a77701)",
  "",
  "running 7 tests",
  "test tests::ignored ... ignored",
  "test tests::should_have_panicked - should panic ... FAILED",
  "test tests::passes ... ok",
  "test tests::returns_an_error ... FAILED",
  "test tests::asks_the_server ... FAILED",
  "test tests::compares_two_numbers ... FAILED",
  "test tests::panics_in_a_thread_of_its_own ... FAILED",
  "",
  "failures:",
  "",
  "---- tests::should_have_panicked stdout ----",
  "note: test did not panic as expected at src/lib.rs:45:8",
  "---- tests::returns_an_error stdout ----",
  'Error: "the mailbox was not there"',
  "",
  "---- tests::asks_the_server stdout ----",
  "",
  "thread 'tests::asks_the_server' (131270242) panicked at src/lib.rs:31:9:",
  "LYONOYL_MAIL_SERVER_REQUIRED is set and nothing is answering on 127.0.0.1:1143: no connection within 300 ms. This run was told the development mail server is part of it.",
  "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
  "",
  "---- tests::compares_two_numbers stdout ----",
  "",
  "thread 'tests::compares_two_numbers' (131270243) panicked at src/lib.rs:24:9:",
  "assertion `left == right` failed: one more than one",
  "  left: 2",
  " right: 3",
  "",
  "---- tests::panics_in_a_thread_of_its_own stdout ----",
  "",
  "thread '<unnamed>' (131270248) panicked at src/lib.rs:53:31:",
  "a worker gave up",
  "",
  "thread 'tests::panics_in_a_thread_of_its_own' (131270244) panicked at src/lib.rs:55:14:",
  "called `Result::unwrap()` on an `Err` value: Any { .. }",
  "",
  "",
  "failures:",
  "    tests::asks_the_server",
  "    tests::compares_two_numbers",
  "    tests::panics_in_a_thread_of_its_own",
  "    tests::returns_an_error",
  "    tests::should_have_panicked",
  "",
  "test result: FAILED. 1 passed; 5 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s",
  "",
  "error: test failed, to rerun pass `--lib`",
  "     Running tests/aborts.rs (target/debug/deps/aborts-1410756189a7327b)",
  "",
  "running 1 test",
  "error: test failed, to rerun pass `--test aborts`",
  "",
  "Caused by:",
  "  process didn't exit successfully: `/work/sample/target/debug/deps/aborts-1410756189a7327b` (signal: 6, SIGABRT: process abort signal)",
  "     Running tests/fine.rs (target/debug/deps/fine-28a9f575c7f4e027)",
  "",
  "running 2 tests",
  "test all_good ... ok",
  "test port_exhausted ... FAILED",
  "",
  "failures:",
  "",
  "---- port_exhausted stdout ----",
  "",
  "thread 'port_exhausted' (131270265) panicked at tests/fine.rs:8:5:",
  "could not connect: Kind(AddrNotAvailable), Can't assign requested address (os error 49)",
  "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
  "",
  "",
  "failures:",
  "    port_exhausted",
  "",
  "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
  "",
  "error: test failed, to rerun pass `--test fine`",
  "   Doc-tests sample",
  "",
  "running 1 test",
  "test src/lib.rs - add_one (line 3) ... FAILED",
  "",
  "failures:",
  "",
  "---- src/lib.rs - add_one (line 3) stdout ----",
  "Test executable failed (exit status: 101).",
  "",
  "stderr:",
  "",
  "thread 'main' (131270332) panicked at src/lib.rs:5:1:",
  "assertion `left == right` failed",
  "  left: 2",
  " right: 3",
  "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
  "",
  "",
  "",
  "failures:",
  "    src/lib.rs - add_one (line 3)",
  "",
  "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.58s",
  "",
  "error: doctest failed, to rerun pass `--doc`",
  "error: 4 targets failed:",
  "    `--lib`",
  "    `--test aborts`",
  "    `--test fine`",
  "    `--doc`",
].join("\n");

// The same crate with a type error in it: nothing ran.
const UNBUILT = [
  "   Compiling sample v0.1.0 (/work/sample)",
  "error[E0308]: mismatched types",
  "  --> src/lib.rs:12:5",
  "   |",
  "11 | pub fn broken() -> u32 {",
  "   |                    --- expected `u32` because of return type",
  "12 |     \"not a number\"",
  "   |     ^^^^^^^^^^^^^^ expected `u32`, found `&str`",
  "",
  "For more information about this error, try `rustc --explain E0308`.",
  "error: could not compile `sample` (lib) due to 1 previous error",
  "warning: build failed, waiting for other jobs to finish...",
  "error: could not compile `sample` (lib test) due to 1 previous error",
].join("\n");

const passing = (count) =>
  [
    "     Running unittests src/lib.rs (target/debug/deps/sample-728e3d26d6a77701)",
    `running ${count} tests`,
    `test result: ok. ${count} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s`,
  ].join("\n");

/** A run of `count` tests in one program, every one of which failed with the same message. */
function failing(count, said) {
  const names = Array.from({ length: count }, (_, at) => `account_tests::test_number_${at}`);
  return [
    "     Running unittests src/lib.rs (target/debug/deps/lyonoyl-0123456789abcdef)",
    `running ${count} tests`,
    ...names.map((name) => `test ${name} ... FAILED`),
    "",
    "failures:",
    "",
    ...names.flatMap((name) => [
      `---- ${name} stdout ----`,
      "",
      `thread '${name}' (1) panicked at src/dev_mail_server.rs:85:5:`,
      said,
      "",
    ]),
    "failures:",
    ...names.map((name) => `    ${name}`),
    "",
    `test result: FAILED. 0 passed; ${count} failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s`,
    "",
    "error: test failed, to rerun pass `-p lyonoyl --lib`",
  ].join("\n");
}

describe("what a cargo test run printed, read back", () => {
  it("holds every way a test fails that the fixture is meant to show", () => {
    expect(RUN).toContain("test result: FAILED. 1 passed; 5 failed; 1 ignored");
    expect(RUN).toContain("(signal: 6, SIGABRT: process abort signal)");
    expect(RUN).toContain("   Doc-tests sample");
    expect(RUN).toContain("- should panic ... FAILED");
  });

  it("counts what passed, failed and was ignored across every test program", () => {
    const run = rustRun(RUN);

    expect([run.passed, run.failed, run.ignored, run.targets]).toEqual([2, 7, 1, 4]);
  });

  it("names every failed test under the command that reruns its program", () => {
    const run = rustRun(RUN);

    expect(run.failures.map((failure) => [failure.program, failure.name])).toEqual([
      ["cargo test --lib", "tests::should_have_panicked"],
      ["cargo test --lib", "tests::returns_an_error"],
      ["cargo test --lib", "tests::asks_the_server"],
      ["cargo test --lib", "tests::compares_two_numbers"],
      ["cargo test --lib", "tests::panics_in_a_thread_of_its_own"],
      ["cargo test --test fine", "port_exhausted"],
      ["cargo test --doc", "src/lib.rs - add_one (line 3)"],
    ]);
  });

  it("gives where an assertion stopped and its message with the values it compared", () => {
    const failure = rustRun(RUN).failures.find((one) => one.name === "tests::compares_two_numbers");

    expect(failure.at).toBe("src/lib.rs:24:9");
    expect(failure.said).toEqual(["assertion `left == right` failed: one more than one", "  left: 2", " right: 3"]);
  });

  it("takes the first panic of a test, which is the one in the thread that gave up", () => {
    const failure = rustRun(RUN).failures.find((one) => one.name === "tests::panics_in_a_thread_of_its_own");

    expect(failure.at).toBe("src/lib.rs:53:31");
    expect(failure.said).toEqual(["a worker gave up"]);
  });

  it("gives what a test that returned an error said, though it did not panic", () => {
    const failure = rustRun(RUN).failures.find((one) => one.name === "tests::returns_an_error");

    expect(failure.at).toBeNull();
    expect(failure.said).toEqual(['Error: "the mailbox was not there"']);
  });

  it("says a should_panic test did not panic, and where it is", () => {
    const failure = rustRun(RUN).failures.find((one) => one.name === "tests::should_have_panicked");

    expect(failure.at).toBe("src/lib.rs:45:8");
    expect(failure.said).toEqual(["did not panic as expected"]);
  });

  it("reads a failed doc test past the lines rustdoc puts before the panic", () => {
    const failure = rustRun(RUN).failures.find((one) => one.program === "cargo test --doc");

    expect(failure.at).toBe("src/lib.rs:5:1");
    expect(failure.said[0]).toBe("assertion `left == right` failed");
  });

  it("reports a test program that died before it reported, with the signal that ended it", () => {
    const run = rustRun(RUN);

    expect(run.crashes).toEqual([{ program: "cargo test --test aborts", why: "signal: 6, SIGABRT: process abort signal" }]);
    expect(run.unfinished).toEqual([]);
  });

  it("reports a test program whose output ends before its result as one that never reported", () => {
    const cut = RUN.slice(0, RUN.indexOf("test result: FAILED. 1 passed; 1 failed"));

    expect(rustRun(cut).unfinished).toEqual([{ program: "tests/fine.rs", why: null }]);
  });

  it("finds nothing wrong in a run where everything passed", () => {
    const run = rustRun(passing(2040));

    expect([run.passed, run.failed, run.failures, run.crashes]).toEqual([2040, 0, [], []]);
  });

  it("reads through the colours a terminal run carries", () => {
    const coloured = RUN.replace("test port_exhausted ... FAILED", "test port_exhausted ... \u001b[31mFAILED\u001b[0m");

    expect(rustRun(coloured).failures.map((failure) => failure.name)).toContain("port_exhausted");
  });

  it("gives the compiler's errors, with where each is, when the tests did not compile", () => {
    const run = rustRun(UNBUILT);

    expect(run.targets).toBe(0);
    expect(run.unbuilt[0]).toEqual({ said: "mismatched types", at: "src/lib.rs:12:5" });
  });

  it("says a run that printed nothing printed no results, rather than that nothing failed", () => {
    const lines = describeRust(rustRun(""), "target/preflight/rust-tests.log");

    expect(lines).toEqual(["cargo test printed no test results. What it did print: target/preflight/rust-tests.log"]);
  });
});

describe("a cargo test run, as the end of the preflight shows it", () => {
  it("names every failed test, however many there are", () => {
    const lines = describeRust(rustRun(failing(40, "assertion failed")), "rust-tests.log");

    for (let at = 0; at < 40; at += 1) expect(lines).toContain(`    account_tests::test_number_${at}`);
    expect(lines[0]).toBe("40 of 40 failed");
  });

  it("puts each test under its program, with where it stopped and what it said", () => {
    const lines = describeRust(rustRun(RUN), "rust-tests.log");
    const from = lines.indexOf("  cargo test --test fine");

    expect(lines.slice(from, from + 4)).toEqual([
      "  cargo test --test fine",
      "    port_exhausted",
      "      at tests/fine.rs:8:5",
      "      could not connect: Kind(AddrNotAvailable), Can't assign requested address (os error 49)",
    ]);
  });

  it("says how many died, and that which test a dead program was running is not known", () => {
    const lines = describeRust(rustRun(RUN), "rust-tests.log");

    expect(lines[0]).toBe("7 of 9 failed, and 1 test program died before reporting");
    expect(lines).toContain(
      "    the program died (signal: 6, SIGABRT: process abort signal): which test it was running is not reported",
    );
  });

  it("cuts a line of what a test said at the width, and not before", () => {
    const at = (said) => describeRust(rustRun(failing(1, said)), "log").find((line) => line.startsWith("      x"));

    expect(at("x".repeat(200))).toBe(`      ${"x".repeat(200)}`);
    expect(at("x".repeat(201))).toBe(`      ${"x".repeat(199)}…`);
  });

  it("ends with where the whole output is", () => {
    expect(describeRust(rustRun(RUN), "target/preflight/rust-tests.log").at(-1)).toBe(
      "The whole output: target/preflight/rust-tests.log",
    );
  });
});

describe("why the tests failed, where the reason is one this tree knows", () => {
  const said = (text) => [{ said: [text] }];
  const probe = (address, why) =>
    `LYONOYL_MAIL_SERVER_REQUIRED is set and nothing is answering on ${address}: ${why}. This run was told the development mail server is part of it`;

  it("says a refused connection is a mail server that stopped", () => {
    expect(causes(said(probe("127.0.0.1:1587", "the connection was refused")))).toEqual([
      "Why: nothing was listening on 127.0.0.1:1587: the development mail server stopped during the run. `docker ps -a` shows whether its two containers are up.",
    ]);
  });

  it("says a connection that never completed was held on this machine, and the test never ran", () => {
    expect(causes(said(probe("127.0.0.1:1143", "no connection within 300 ms")))).toEqual([
      "Why: the development mail server was listening, and a connection to 127.0.0.1:1143 did not complete in time: something on this machine held it. Nothing the test is about was reached.",
    ]);
  });

  it("says a machine with no port left for a connection is what failed, ahead of the server's check", () => {
    expect(causes(said(probe("127.0.0.1:1143", "Can't assign requested address (os error 49)")))).toEqual([
      "Why: this machine had no local port free for a new connection.",
    ]);
  });

  it.each([
    ["macOS", "Can't assign requested address (os error 49)"],
    ["Linux", "Cannot assign requested address (os error 99)"],
    ["Windows", "The requested address is not valid in its context. (os error 10049)"],
    ["Rust's own words", "address not available"],
    ["Rust's name for it", "Kind(AddrNotAvailable)"],
  ])("knows a machine with no port left in the words %s uses", (_, words) => {
    expect(causes(said(`could not connect: ${words}`))).toEqual(["Why: this machine had no local port free for a new connection."]);
  });

  it("knows the server's check by its older words too", () => {
    expect(causes(said("LYONOYL_MAIL_SERVER_REQUIRED is set and nothing is answering on 127.0.0.1:1143. This run"))).toEqual([
      "Why: the check that the development mail server answers on 127.0.0.1:1143 failed, before anything the test is about ran.",
    ]);
  });

  it("says how many of the failures each reason covers, when it covers only some", () => {
    const failures = [...said(probe("127.0.0.1:1143", "no connection within 300 ms")), ...said("assertion failed")];

    expect(causes(failures)).toEqual([
      "Why, for 1 of them: the development mail server was listening, and a connection to 127.0.0.1:1143 did not complete in time: something on this machine held it. Nothing the test is about was reached.",
      "For the other 1, no reason this summary knows: what each said is above.",
    ]);
  });

  it("says nothing about a failure it has no reason for", () => {
    expect(causes(said("assertion `left == right` failed"))).toEqual([]);
  });

  it("knows the words the server's check fails with now", () => {
    // The two reasons above are recognised by the words the check itself writes. Reworded there and
    // not here, every such failure would lose its explanation without a test going red.
    const check = readFileSync(join(__dirname, "..", "src-tauri", "src", "dev_mail_server.rs"), "utf8");

    expect(check).toContain('"the connection was refused"');
    expect(check).toContain('"no connection within {} ms"');
    expect(check).toContain("{REQUIRED} is set and nothing is answering on {address}: {why}.");
  });
});

describe("paths from a panic, as the repository holds them", () => {
  let root;

  afterEach(() => rmSync(root, { recursive: true, force: true }));

  function tree(files) {
    root = mkdtempSync(join(tmpdir(), "failed-tests-"));
    writeFileSync(join(root, "Cargo.toml"), '[workspace]\nmembers = ["crates/domain", "app"]\n');
    for (const file of files) {
      mkdirSync(join(root, file, ".."), { recursive: true });
      writeFileSync(join(root, file), "");
    }
    return inTheTree(root);
  }

  it("puts the member's directory before a file only that member holds", () => {
    const where = tree(["app/src/dev_mail_server.rs", "crates/domain/src/lib.rs"]);

    expect(where("src/dev_mail_server.rs:85:5")).toBe("app/src/dev_mail_server.rs:85");
  });

  it("leaves a file two members hold as the panic gave it", () => {
    const where = tree(["app/src/lib.rs", "crates/domain/src/lib.rs"]);

    expect(where("src/lib.rs:12:1")).toBe("src/lib.rs:12");
  });

  it("leaves a path that is already one from the root", () => {
    const where = tree(["app/src/lib.rs"]);

    expect(where("app/src/lib.rs:3:9")).toBe("app/src/lib.rs:3");
  });

  it("leaves what is not a path with a line as it is", () => {
    const where = tree([]);

    expect(where("<anon>")).toBe("<anon>");
  });
});

describe("what a vitest JSON report holds, read back", () => {
  const ROOT = "/work/lyonoyl";
  const REPORT = {
    numTotalTests: 2,
    numPassedTests: 1,
    numFailedTests: 1,
    success: false,
    testResults: [
      {
        name: `${ROOT}/src/features/inbox/broken.test.ts`,
        status: "failed",
        message: `Cannot find module './does-not-exist' imported from ${ROOT}/src/features/inbox/broken.test.ts`,
        assertionResults: [],
      },
      {
        name: `${ROOT}/src/features/inbox/list.test.ts`,
        status: "failed",
        message: "",
        assertionResults: [
          {
            ancestorTitles: ["The message list"],
            fullName: "The message list shows the newest first",
            status: "passed",
            title: "shows the newest first",
            failureMessages: [],
          },
          {
            ancestorTitles: ["The message list", "when a row is selected"],
            fullName: "The message list when a row is selected keeps the selection across a refresh",
            status: "failed",
            title: "keeps the selection across a refresh",
            failureMessages: [
              `AssertionError: expected { selected: 'b' } to deeply equal { selected: 'a' }\n    at ${ROOT}/src/features/inbox/list.test.ts:10:33\n    at file:///work/lyonoyl/node_modules/@vitest/runner/dist/index.js:145:11`,
            ],
          },
        ],
      },
    ],
  };

  it("names a failed test by its describe blocks and its own title, with its message and no stack", () => {
    const run = frontendRun(REPORT, ROOT);

    expect(run.failures).toEqual([
      {
        file: "src/features/inbox/list.test.ts",
        name: "The message list › when a row is selected › keeps the selection across a refresh",
        said: ["AssertionError: expected { selected: 'b' } to deeply equal { selected: 'a' }"],
      },
    ]);
  });

  it("reports a file that did not load, which vitest leaves out of its count of failed tests", () => {
    const run = frontendRun(REPORT, ROOT);

    expect(run.unloaded).toEqual([
      {
        file: "src/features/inbox/broken.test.ts",
        said: ["Cannot find module './does-not-exist' imported from src/features/inbox/broken.test.ts"],
      },
    ]);
    expect(describeFrontend(run, "report.json")[0]).toBe("1 of 2 failed, and 1 file did not load");
  });

  it("finds nothing wrong in a report where everything passed", () => {
    const run = frontendRun({ numPassedTests: 5, numFailedTests: 0, testResults: [] }, ROOT);

    expect(describeFrontend(run, "report.json")).toEqual(["0 of 5 failed", "The whole report: report.json"]);
  });

  it("reads a report with no results at all as zero of zero", () => {
    const run = frontendRun({}, ROOT);

    expect([run.passed, run.failed, run.failures, run.unloaded]).toEqual([0, 0, [], []]);
  });
});
