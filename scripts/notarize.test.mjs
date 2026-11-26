import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const PROGRESS = resolve("scripts/progress.sh");
const NOTARIZE = resolve("scripts/notarize.sh");
const SUBMISSION = "00000000-0000-0000-0000-000000000001";

// A notary that answers from a list: each question about the upload takes the next status off it,
// and `silent` is a question it does not answer. Everything it is asked is written down.
const NOTARY = `#!/bin/bash
printf '%s\\n' "$*" >>"$ASKED"
case "$1 $2" in
  "notarytool submit")
    if [ -n "\${UPLOAD_FAILS:-}" ]; then exit 1; fi
    printf '{"id":"${SUBMISSION}","message":"Successfully uploaded file"}\\n'
    ;;
  "notarytool info")
    status=$(head -n 1 "$STATUSES")
    tail -n +2 "$STATUSES" >"$STATUSES.left" && mv "$STATUSES.left" "$STATUSES"
    if [ -z "$status" ] || [ "$status" = silent ]; then exit 1; fi
    printf '{"id":"${SUBMISSION}","status":"%s"}\\n' "$status"
    ;;
  *) exit 2 ;;
esac
`;

const made = [];
afterEach(() => {
  while (made.length) rmSync(made.pop(), { recursive: true, force: true });
});

/** Notarizes a file with a notary that answers `statuses` in turn, and says what came of it. */
function notarize(statuses, { unanswered = 10, uploadFails = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "notarize-"));
  made.push(root);
  mkdirSync(join(root, "bin"));
  writeFileSync(join(root, "bin/xcrun"), NOTARY);
  chmodSync(join(root, "bin/xcrun"), 0o755);
  writeFileSync(join(root, "statuses"), statuses.map((status) => `${status}\n`).join(""));
  writeFileSync(join(root, "asked"), "");

  const script = [
    "set -euo pipefail",
    `. "${PROGRESS}"`,
    `. "${NOTARIZE}"`,
    "notary_every=0",
    `notary_unanswered=${unanswered}`,
    'progress_begin demo Stage "The image"',
    'progress_step "The image"',
    'notarize "the image.dmg" --key "the key.p8" --key-id KEY --issuer ISSUER',
    'echo "notarized=[$notarized] submission=[$submission]"',
  ].join("\n");
  const answer = spawnSync("/bin/bash", ["-c", script], {
    cwd: root,
    encoding: "utf8",
    timeout: 20_000,
    env: {
      ...process.env,
      PATH: `${join(root, "bin")}:${process.env.PATH}`,
      ASKED: join(root, "asked"),
      STATUSES: join(root, "statuses"),
      UPLOAD_FAILS: uploadFails ? "1" : "",
    },
  });
  const asked = readFileSync(join(root, "asked"), "utf8").trim().split("\n").filter(Boolean);
  const note = readFileSync(join(root, "target/progress/demo.note"), "utf8");
  return { answer, asked, note, said: answer.stdout.trim().split("\n").pop() };
}

describe.runIf(process.platform === "darwin")("notarizing a file", () => {
  it("asks after the upload until the notary gives it a status, and takes that status", () => {
    const { answer, asked, note, said } = notarize(["In Progress", "In Progress", "Accepted"]);

    expect(answer.status).toBe(0);
    expect(said).toBe(`notarized=[Accepted] submission=[${SUBMISSION}]`);
    expect(asked.filter((line) => line.startsWith("notarytool info"))).toHaveLength(3);
    // Nothing is left on the bottom line once the notary has answered.
    expect(note).toBe("\n");
  });

  it("takes a refusal as the notary gave it", () => {
    const { said } = notarize(["In Progress", "Invalid"]);

    expect(said).toBe(`notarized=[Invalid] submission=[${SUBMISSION}]`);
  });

  it("uploads once, without waiting, and asks every question signed in the same way", () => {
    const { asked } = notarize(["Accepted"]);

    expect(asked).toEqual([
      "notarytool submit the image.dmg --key the key.p8 --key-id KEY --issuer ISSUER --output-format json",
      `notarytool info ${SUBMISSION} --key the key.p8 --key-id KEY --issuer ISSUER --output-format json`,
    ]);
  });

  it("asks nothing more of an upload that never went, and says there is no status and no submission", () => {
    const { answer, asked, said } = notarize(["Accepted"], { uploadFails: true });

    expect(answer.status).toBe(0);
    expect(said).toBe("notarized=[] submission=[]");
    expect(asked).toHaveLength(1);
  });

  it("stops asking once the notary has not answered so many times in a row", () => {
    const { asked, said } = notarize(["silent", "silent", "silent", "Accepted"], { unanswered: 3 });

    expect(said).toBe(`notarized=[] submission=[${SUBMISSION}]`);
    expect(asked.filter((line) => line.startsWith("notarytool info"))).toHaveLength(3);
  });

  it("counts only the questions in a row that went unanswered", () => {
    const { said } = notarize(["silent", "silent", "In Progress", "silent", "silent", "Accepted"], { unanswered: 3 });

    expect(said).toBe(`notarized=[Accepted] submission=[${SUBMISSION}]`);
  });
});
