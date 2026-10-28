import { beforeEach, describe, expect, it } from "vitest";

import { applyNativeFeel, markWindowControls } from "./native-feel";

function press(target: Element, type: "contextmenu" | "dragstart"): boolean {
  const event = new Event(type, { bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("native feel", () => {
  it("gives a control no browser menu of its own", () => {
    // Right-clicking a folder name should not offer Reload and Inspect.
    document.body.innerHTML = `<button id="folder">INBOX</button>`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("folder")!, "contextmenu")).toBe(true);
    stop();
  });

  it("leaves the menu alone on text a person may want to copy", () => {
    document.body.innerHTML = `<article data-selectable><p id="body">The message.</p></article>`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("body")!, "contextmenu")).toBe(false);
    stop();
  });

  it("leaves it alone in a field being typed into", () => {
    document.body.innerHTML = `<input id="host" />`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("host")!, "contextmenu")).toBe(false);
    stop();
  });

  it("does not let a control be dragged out of the window", () => {
    document.body.innerHTML = `<button id="folder">INBOX</button>`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("folder")!, "dragstart")).toBe(true);
    stop();
  });

  it("lets go of a row the window moves itself", () => {
    // The sidebar reorders mailboxes by dragging them. Cancelled here, one level above every
    // handler listening for it, that drag looked built and did nothing at all.
    document.body.innerHTML = `<div data-drags><button id="row">one@example.com</button></div>`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("row")!, "dragstart")).toBe(false);
    stop();
  });

  it("still gives that row no browser menu of its own", () => {
    // The mark is about carrying the row, and says nothing about Reload and Inspect.
    document.body.innerHTML = `<div data-drags><button id="row">one@example.com</button></div>`;
    const stop = applyNativeFeel();

    expect(press(document.getElementById("row")!, "contextmenu")).toBe(true);
    stop();
  });

  it("stops when it is told to", () => {
    document.body.innerHTML = `<button id="folder">INBOX</button>`;
    const stop = applyNativeFeel();
    stop();

    expect(press(document.getElementById("folder")!, "contextmenu")).toBe(
      false,
    );
  });
});

describe("markWindowControls", () => {
  /** A document that says it is running where the test says. */
  function on(platform: string): Document {
    const page = document.implementation.createHTMLDocument("");

    Object.defineProperty(page, "defaultView", {
      value: { navigator: { userAgent: platform } },
      configurable: true,
    });

    return page;
  }

  it("marks a window whose buttons float over its corner", () => {
    // macOS is where the title bar is taken away, so the content reaches the top of the window and
    // the buttons sit over it.
    const page = on("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");

    markWindowControls(page);

    expect(page.documentElement.dataset.titlebar).toBe("overlay");
  });

  it("leaves a window that has a bar of its own alone", () => {
    // On Windows and Linux the bar is how a window is moved, closed and told apart, and the corner
    // below it is ordinary space.
    for (const platform of [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
      "Mozilla/5.0 (X11; Linux x86_64)",
    ]) {
      const page = on(platform);

      markWindowControls(page);

      expect(page.documentElement.dataset.titlebar, platform).toBeUndefined();
    }
  });

  it("says nothing when there is nothing to ask", () => {
    const page = document.implementation.createHTMLDocument("");
    Object.defineProperty(page, "defaultView", {
      value: null,
      configurable: true,
    });

    markWindowControls(page);

    expect(page.documentElement.dataset.titlebar).toBeUndefined();
  });
});
