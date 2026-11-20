import { readFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { STRIP, actAsTitleBar } from "./title-bar";

/**
 * The core, as plain functions: a mock that keeps what each call answered takes every refusal
 * itself, and a refusal the strip left to nobody would go unseen.
 */
const core = vi.hoisted(() => ({
  pressed: 0,
  doubleClicked: 0,
  answer: (): Promise<void> => Promise.resolve(),
}));

vi.mock("@/shared/ipc/client", () => ({
  pressTitleBar: () => {
    core.pressed += 1;
    return core.answer();
  },
  doubleClickTitleBar: () => {
    core.doubleClicked += 1;
    return core.answer();
  },
}));

/** Inside the strip, which is 32 pixels at the window's 16-pixel rem. */
const UP_THERE = { clientX: 120, clientY: 10 };

function mouse(
  type: "mousedown" | "mouseup",
  target: Element,
  init: MouseEventInit = {},
): boolean {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    button: 0,
    detail: 1,
    ...UP_THERE,
    ...init,
  });
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

function element(id: string): Element {
  const found = document.getElementById(id);
  if (found === null) throw new Error(`no #${id}`);
  return found;
}

let stop: () => void = () => {};

beforeEach(() => {
  document.documentElement.dataset.titlebar = "overlay";
  document.body.innerHTML = `
    <aside id="sidebar"><button id="compose">Compose</button></aside>
    <div id="search-row"><input id="search" role="searchbox" /></div>
    <article data-selectable>
      <header id="header">
        <div id="subject-line"><h1 id="subject">Quarterly figures</h1>
          <button id="reply"><svg id="reply-icon"></svg></button></div>
      </header>
    </article>
    <section id="screen" tabindex="-1"><h1 id="heading">Settings</h1>
      <a id="link" href="#licences">Licences</a> <label id="label">Theme</label></section>
    <div id="edge" role="separator" tabindex="0"></div>
    <div id="row" role="option">A message</div>
    <div id="focusable" tabindex="0"></div>
    <div id="editable" contenteditable="true"></div>
    <div id="dragged" data-drags>Inbox</div>
  `;
  core.pressed = 0;
  core.doubleClicked = 0;
  core.answer = () => Promise.resolve();
  stop = actAsTitleBar();
});

afterEach(() => {
  stop();
  delete document.documentElement.dataset.titlebar;
});

describe("the top of the window as a title bar", () => {
  it("moves the window from the empty top of a column, and nothing else comes of the press", () => {
    expect(mouse("mousedown", element("sidebar"))).toBe(true);
    expect(core.pressed).toBe(1);
  });

  it("is as tall as the room the stylesheet leaves under the window's buttons", () => {
    const stylesheet = readFileSync(join(__dirname, "..", "index.css"), "utf8");
    const room =
      /\.under-window-controls\s*\{\s*padding-top:\s*([\d.]+)rem;/.exec(
        stylesheet,
      );

    expect(Number(room?.[1])).toBe(STRIP);
  });

  it("ends at the last row of pixels of the strip", () => {
    mouse("mousedown", element("sidebar"), { clientY: 31 });
    expect(core.pressed).toBe(1);

    expect(mouse("mousedown", element("sidebar"), { clientY: 32 })).toBe(false);
    expect(core.pressed).toBe(1);
  });

  it.each([
    ["a button", "compose"],
    ["the picture inside a button", "reply-icon"],
    ["the search field", "search"],
    ["a link", "link"],
    ["a label", "label"],
    ["the edge a column is resized by", "edge"],
    ["a row of a list", "row"],
    ["anything the keyboard reaches", "focusable"],
    ["text being written", "editable"],
    ["a row the window drags", "dragged"],
  ])("leaves %s to answer the press itself", (_, id) => {
    expect(mouse("mousedown", element(id))).toBe(false);
    expect(core.pressed).toBe(0);
  });

  it("selects text a person may take with them, pressed on the text itself", () => {
    expect(mouse("mousedown", element("subject"))).toBe(false);
    expect(core.pressed).toBe(0);
  });

  it("moves the window from the room around that text", () => {
    mouse("mousedown", element("header"));
    mouse("mousedown", element("subject-line"));

    expect(core.pressed).toBe(2);
  });

  it("moves the window from a heading nobody selects, and from a screen only a script can focus", () => {
    mouse("mousedown", element("heading"));
    mouse("mousedown", element("screen"));

    expect(core.pressed).toBe(2);
  });

  it("leaves the secondary click alone, with either button", () => {
    expect(mouse("mousedown", element("sidebar"), { button: 2 })).toBe(false);
    expect(mouse("mousedown", element("sidebar"), { ctrlKey: true })).toBe(
      false,
    );

    expect(core.pressed).toBe(0);
  });

  it("does what a double click on a title bar does as the second press comes up", () => {
    const sidebar = element("sidebar");
    mouse("mousedown", sidebar);
    mouse("mouseup", sidebar);
    expect(mouse("mousedown", sidebar, { detail: 2 })).toBe(true);

    // The second press holds the window still, and nothing happens until it comes up.
    expect(core.pressed).toBe(1);
    expect(core.doubleClicked).toBe(0);

    mouse("mouseup", sidebar, { detail: 2 });
    expect(core.doubleClicked).toBe(1);
  });

  it("is no double click once the pointer has moved in between", () => {
    const sidebar = element("sidebar");
    mouse("mousedown", sidebar, { detail: 2 });
    mouse("mouseup", sidebar, { detail: 2, clientX: 140 });

    expect(core.doubleClicked).toBe(0);
  });

  it("is no double click when the second press landed on a control", () => {
    mouse("mousedown", element("compose"), { detail: 2 });
    mouse("mouseup", element("sidebar"), { detail: 2 });

    expect(core.doubleClicked).toBe(0);
  });

  it("answers a double click once, however many times the button then comes up", () => {
    const sidebar = element("sidebar");
    mouse("mousedown", sidebar, { detail: 2 });
    mouse("mouseup", sidebar, { detail: 2 });
    mouse("mouseup", sidebar, { detail: 2 });

    expect(core.doubleClicked).toBe(1);
  });

  it("takes a third press in a row for neither", () => {
    const sidebar = element("sidebar");
    expect(mouse("mousedown", sidebar, { detail: 3 })).toBe(false);
    mouse("mouseup", sidebar, { detail: 3 });

    expect(core.pressed).toBe(0);
    expect(core.doubleClicked).toBe(0);
  });

  it("takes the core's refusal to move the window as an answer", async () => {
    core.answer = () => Promise.reject(new Error("refused"));
    const unanswered: unknown[] = [];
    const note = (reason: unknown) => unanswered.push(reason);
    process.on("unhandledRejection", note);
    const sidebar = element("sidebar");

    mouse("mousedown", sidebar);
    mouse("mousedown", sidebar, { detail: 2 });
    mouse("mouseup", sidebar, { detail: 2 });
    // A refusal nobody takes is reported once the work in hand is done.
    await new Promise((settled) => setTimeout(settled, 0));
    process.off("unhandledRejection", note);

    expect(core.pressed).toBe(1);
    expect(core.doubleClicked).toBe(1);
    expect(unanswered).toEqual([]);
  });

  it("stops when it is told to", () => {
    stop();
    mouse("mousedown", element("sidebar"));

    expect(core.pressed).toBe(0);
  });

  it("does nothing where the window keeps the system's own bar", () => {
    stop();
    delete document.documentElement.dataset.titlebar;
    stop = actAsTitleBar();

    expect(mouse("mousedown", element("sidebar"))).toBe(false);
    mouse("mousedown", element("sidebar"), { detail: 2 });
    mouse("mouseup", element("sidebar"), { detail: 2 });

    expect(core.pressed).toBe(0);
    expect(core.doubleClicked).toBe(0);
  });
});
