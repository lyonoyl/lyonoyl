// @vitest-environment node
//
// This file starts a real development server and asks it for a real page; nothing in it touches a
// document. Left in the browser-shaped environment it went through that environment's own `fetch`,
// which under the load of the whole suite gave up on a request to a server on this machine — the
// file failed in a full run and never once in twelve runs of its own.
import { createServer } from "vite";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

/**
 * The development server sends the application's own policy, and must be able to live under it.
 *
 * This is not a hypothetical. The policy refuses inline scripts, as the shipped one does — and Vite
 * injects one inline module of its own, the preamble React Fast Refresh needs. Sent without a way
 * to allow that one line, the browser refuses it, every component module then throws on a global
 * the preamble was supposed to define, and the window opens white with nothing in the log to say
 * why: the failure is inside the webview, and no Rust gate can see it.
 *
 * The existing checks compare the policy's *text*. Text was not the problem. So this one starts the
 * real server, asks for the real page, and holds what came back against what the browser would do
 * with it.
 */

let html = "";
let policy = "";

beforeAll(async () => {
  // A port of its own: the development server may well be running while this test does.
  const server = await createServer({
    configFile: "vite.config.ts",
    logLevel: "silent",
    server: { port: 0, strictPort: false },
  });

  await server.listen();

  try {
    const address = server.resolvedUrls?.local[0];
    if (!address) throw new Error("the development server reported no address");

    const answer = await fetch(address);
    html = await answer.text();
    policy = answer.headers.get("content-security-policy") ?? "";
  } finally {
    await server.close();
  }
}, 60_000);

afterAll(() => {
  html = "";
  policy = "";
});

/** The sources one directive allows, e.g. `script-src 'self' 'nonce-abc'`. */
function sourcesFor(directive: string): string[] {
  const found = policy
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith(`${directive} `));

  return found ? found.split(/\s+/).slice(1) : [];
}

describe("the page the development server sends", () => {
  it("comes with a policy at all", () => {
    expect(policy).not.toBe("");
    expect(sourcesFor("script-src")).not.toHaveLength(0);
  });

  it("does not buy itself room by allowing every inline script", () => {
    // The moment development accepts `'unsafe-inline'`, it stops rehearsing what ships: an inline
    // script written by mistake works here and fails in front of a reader.
    expect(sourcesFor("script-src")).not.toContain("'unsafe-inline'");
  });

  it("allows every script it sends, and each one by name", () => {
    const allowed = sourcesFor("script-src");
    const inline = [
      ...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g),
    ].filter((tag) => !/\bsrc=/.test(tag[1]) && tag[2].trim() !== "");

    // If Vite ever stops injecting its preamble this test has nothing to prove, and saying so is
    // better than passing quietly.
    expect(inline.length).toBeGreaterThan(0);

    for (const [, attributes] of inline) {
      const nonce = /\bnonce="([^"]+)"/.exec(attributes)?.[1];

      expect(
        nonce,
        `an inline script carries no nonce: ${attributes}`,
      ).toBeTruthy();
      expect(allowed).toContain(`'nonce-${nonce}'`);
    }
  });
});
