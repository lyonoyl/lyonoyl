import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

// The bash macOS ships, which is the oldest these scripts run under, wherever there is one.
const BASH = process.platform === "darwin" ? "/bin/bash" : "bash";
const PROGRESS = resolve("scripts/progress.sh");

const made = [];
afterEach(() => {
  while (made.length) rmSync(made.pop(), { recursive: true, force: true });
});

/** A directory to run in, holding the times a last run kept, if any. */
function place(times) {
  const root = mkdtempSync(join(tmpdir(), "progress-"));
  made.push(root);
  if (times !== undefined) {
    mkdirSync(join(root, "target/progress"), { recursive: true });
    writeFileSync(join(root, "target/progress/demo.tsv"), times);
  }
  return root;
}

/** Runs `script` with `scripts/progress.sh` sourced, under the strict mode the scripts run under. */
function run(root, script, timeout = 20_000) {
  return spawnSync(BASH, ["-c", `set -euo pipefail\n. "${PROGRESS}"\n${script}`], {
    cwd: root,
    encoding: "utf8",
    timeout,
  });
}

const kept = (root) => readFileSync(join(root, "target/progress/demo.tsv"), "utf8");

describe.skipIf(process.platform === "win32")("the progress of a run", () => {
  it("says which step it is of how many, and that there is no estimate before a run has passed here", () => {
    const answer = run(place(), 'progress_begin demo Step "One\nTwo"\nprogress_step One');

    expect(answer.status).toBe(0);
    expect(answer.stdout).toContain("Step 1 of 2 · One");
    expect(answer.stdout).toContain("0 s in · no estimate yet");
  });

  it("estimates how long is left from what each step took the last time it passed", () => {
    const answer = run(
      place("One\t60\t\nTwo\t120\t\n"),
      'progress_begin demo Stage "One\nTwo"\nprogress_step One\nSECONDS=$((SECONDS + 30))\nprogress_step Two',
    );

    expect(answer.status).toBe(0);
    expect(answer.stdout).toContain("Stage 1 of 2 · One");
    expect(answer.stdout).toContain("0 s in · about 3 min to go · 0 %");
    // Half a minute in, and the first step done early: the second one's two minutes are left.
    expect(answer.stdout).toContain("Stage 2 of 2 · Two");
    expect(answer.stdout).toContain("30 s in · about 2 min to go · 20 %");
  });

  it("keeps the time of a step that passed, and the last time of one that failed", () => {
    const root = place("One\t60\t\nTwo\t120\t5\n");
    const answer = run(
      root,
      [
        'progress_begin demo Step "One\nTwo"',
        "progress_step One",
        "SECONDS=$((SECONDS + 7))",
        "progress_counted 42",
        "progress_step Two",
        "SECONDS=$((SECONDS + 3))",
        "progress_failed",
        "progress_end",
      ].join("\n"),
    );

    expect(answer.status).toBe(0);
    expect(kept(root)).toBe("One\t7\t42\nTwo\t120\t5\n");
  });

  it("keeps no time for the step a run stopped in, wherever it stopped", () => {
    const root = place("One\t60\t\nTwo\t120\t\n");
    const answer = run(
      root,
      [
        "trap 'progress_end $?' EXIT",
        'progress_begin demo Step "One\nTwo"',
        "progress_step One",
        "SECONDS=$((SECONDS + 9))",
        "progress_step Two",
        "SECONDS=$((SECONDS + 2))",
        "false",
      ].join("\n"),
    );

    expect(answer.status).toBe(1);
    expect(kept(root)).toBe("One\t9\t\nTwo\t120\t\n");
  });

  it("keeps nothing for a step that never ran, or has not run since its last time", () => {
    const root = place("Two\t120\t\n");
    const answer = run(root, 'progress_begin demo Step "One\nTwo\nThree"\nprogress_step One\nprogress_end');

    expect(answer.status).toBe(0);
    expect(kept(root)).toBe("One\t0\t\nTwo\t120\t\n");
  });

  it("goes by an estimate given for a step in place of the step's own last time", () => {
    const given = run(
      place("One\t60\t\nTwo\t120\t\n"),
      'progress_begin demo Step "One\nTwo"\nprogress_expect Two 600\nprogress_step One',
    );
    const none = run(
      place("One\t60\t\nTwo\t120\t\n"),
      'progress_begin demo Step "One\nTwo"\nprogress_expect Two ""\nprogress_step One',
    );

    expect(given.stdout).toContain("about 11 min to go");
    expect(none.stdout).toContain("no estimate yet");
  });

  it("costs an estimate, and never the run, where the times kept are not times", () => {
    const answer = run(
      place("One\tsixty\t\nnot a line at all\n\tTwo\nTwo\t-5\nThree\t9\tmany\nOne\n"),
      'progress_begin demo Step "One\nTwo\nThree"\nprogress_step One\nprogress_step Three\nprogress_end',
    );

    expect(answer.status).toBe(0);
    expect(answer.stdout).toContain("Step 1 of 3 · One");
    expect(answer.stdout).toContain("no estimate yet");
    expect(answer.stderr).toBe("");
  });

  it("names a step it was not told about without counting it", () => {
    const answer = run(place(), 'progress_begin demo Step "One"\nprogress_step Elsewhere\nprogress_end');

    expect(answer.status).toBe(0);
    expect(answer.stdout).toMatch(/\n\u001b\[1mElsewhere\u001b\[0m\n/);
  });

  it("does nothing, and fails nothing, where there is no run or no ticker", () => {
    const answer = run(place(), "progress_ticker_stop\nprogress_close\nprogress_end\nprogress_note hello\necho fine");

    expect(answer.status).toBe(0);
    expect(answer.stdout).toBe("fine\n");
  });

  it("says a span of time in words", () => {
    const spans = [0, 59, 60, 61, 3599, 3600, 3725].map((s) => `progress_span ${s}; echo "$progress_span_said"`);
    const abouts = [0, 59, 60, 61, 3600, 3661].map((s) => `progress_about ${s}; echo "$progress_about_said"`);
    const answer = run(place(), [...spans, ...abouts].join("\n"));

    expect(answer.stdout.trim().split("\n")).toEqual([
      "0 s",
      "59 s",
      "1 min",
      "1 min 1 s",
      "59 min 59 s",
      "1 h 0 min",
      "1 h 2 min",
      "under a minute",
      "under a minute",
      "about 1 min",
      "about 2 min",
      "about 1 h 0 min",
      "about 1 h 2 min",
    ]);
  });

  it("adds up what another run took, or says nothing where it has never run here", () => {
    const root = place("One\t60\t\nTwo\t120\t3\nThree\tbroken\n");
    const answer = run(
      root,
      'progress_total_of demo; echo "[$progress_total]"\nprogress_total_of never; echo "[$progress_total]"',
    );

    expect(answer.stdout).toBe("[180]\n[]\n");
  });

  it("says how long a step took in this run, and nothing for one that did not run", () => {
    const answer = run(
      place(),
      [
        'progress_begin demo Step "One\nTwo"',
        "progress_step One",
        "SECONDS=$((SECONDS + 75))",
        "progress_close",
        'progress_time_of One; echo "[$progress_span_said]"',
        'progress_time_of Two; echo "[$progress_span_said]"',
      ].join("\n"),
    );

    expect(answer.stdout).toContain("[1 min 15 s]\n[]\n");
  });

  it("passes every line through the relay, the last one without its newline too", () => {
    const root = place();
    const answer = run(root, "progress_begin demo Step One\nprintf 'a\\n\\nb %%s\\nlast' | progress_relay");

    expect(answer.status).toBe(0);
    expect(answer.stdout).toBe("a\n\nb %s\nlast\n");
    expect(readFileSync(join(root, "target/progress/demo.beat"), "utf8")).toMatch(/^\d+$/);
  });

  it("keeps saying a quiet step is still going, how far it has counted, and what it is waiting on", () => {
    const root = place("One\t600\t10\n");
    writeFileSync(join(root, "log"), "ok one\nok two\nnot this\nok three\n");
    const answer = run(
      root,
      [
        "progress_every=1",
        "progress_quiet=1",
        'progress_begin demo Step "One"',
        "progress_step One",
        "progress_note 'notary: In Progress'",
        "progress_ticker_start log '^ok ' probes",
        "sleep 3",
        "progress_ticker_stop",
        "echo after",
      ].join("\n"),
    );

    expect(answer.status).toBe(0);
    expect(answer.stdout).toMatch(/… still going, \d+ s · 3 of 10 probes · notary: In Progress · about \d+ min to go · \d+ %/);
    expect(answer.stdout.trimEnd().endsWith("after")).toBe(true);
  });

  it("prints the line into a quiet stretch only, where the output is not a terminal", () => {
    const answer = run(
      place(),
      [
        "progress_every=1",
        "progress_quiet=3",
        'progress_begin demo Step "One"',
        "progress_step One",
        "progress_ticker_start",
        "for i in 1 2 3 4; do echo line; sleep 0.5; done | progress_relay",
        "progress_ticker_stop",
      ].join("\n"),
    );

    expect(answer.status).toBe(0);
    expect(answer.stdout).not.toContain("still going");
  });

  it("stops its ticker when told, and by itself once the run it belongs to has gone", () => {
    const root = place();
    const told = run(
      root,
      [
        "progress_every=1",
        'progress_begin demo Step "One"',
        "progress_step One",
        "progress_ticker_start",
        "ticker=$progress_ticker",
        "progress_ticker_stop",
        'if kill -0 "$ticker" 2>/dev/null; then echo running; else echo gone; fi',
      ].join("\n"),
    );
    expect(told.stdout.trim().split("\n").pop()).toBe("gone");

    // Left running by a run that ended without a word: whatever reads the run's output waits for the
    // ticker as well, so the answer comes back only once the ticker has noticed and gone too.
    const began = Date.now();
    const left = run(
      root,
      ["progress_every=1", 'progress_begin demo Step "One"', "progress_step One", "progress_ticker_start", "exit 0"].join("\n"),
      15_000,
    );
    expect(left.status).toBe(0);
    expect(Date.now() - began).toBeLessThan(10_000);
  });
});
