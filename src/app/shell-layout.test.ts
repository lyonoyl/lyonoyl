import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

/**
 * The message list scrolls inside the window. That only holds while the document itself cannot
 * grow: a list of a hundred thousand rows is over five million pixels tall, and if the page takes
 * that height it scrolls in place of the list, which then renders one screen of rows and stops.
 *
 * No unit test can see layout, so these check the two rules the behaviour depends on.
 */

const styles = readFileSync(join(__dirname, "..", "index.css"), "utf8");
const shell = readFileSync(join(__dirname, "..", "App.tsx"), "utf8");

describe("application shell", () => {
  it("gives the document a fixed height", () => {
    const rule = styles.match(/body,\s*#root\s*\{[^}]*\}/);
    expect(rule?.[0]).toBeDefined();
    expect(rule?.[0]).toContain("height: 100%");
    expect(rule?.[0]).toContain("overflow: hidden");
  });

  it("hides a scroller's bar in every engine, and leaves it a scroller", () => {
    // The message list and the mailboxes wear this. The standard property alone leaves the bar on
    // the older WebKit builds, which obey only the prefixed rule.
    const rule = styles.match(/\.no-gutter\s*\{[^}]*\}/);
    const prefixed = styles.match(/\.no-gutter::-webkit-scrollbar\s*\{[^}]*\}/);

    expect(rule?.[0]).toContain("scrollbar-width: none");
    expect(prefixed?.[0]).toContain("display: none");
    // What scrolls is the element's own business; a rule that clipped it would take the wheel,
    // the trackpad and the focus with the bar.
    expect(rule?.[0]).not.toContain("overflow");
  });

  it("keeps text from the mail at the window's side while it runs its own way", () => {
    // `dir="auto"` aligns a box by its own direction too: a Latin subject in a window written right
    // to left would stand at the far end of its row. In the base layer, so a utility can still say
    // otherwise, and not on a field, where a line starts where its language starts.
    const base = styles.slice(styles.indexOf("@layer base"));
    const rule = base.match(
      /:where\(\[dir="auto"\]:not\(input, textarea\)\)\s*\{[^}]*\}/,
    );
    expect(rule?.[0]).toContain("text-align: match-parent");
  });

  it("keeps the panes beside the sidebar clear of the window's buttons when it is mirrored", () => {
    // The buttons stay in the left corner, which a mirrored window gives to the panes rather than
    // to the sidebar.
    const rule = styles.match(
      /:root\[data-titlebar="overlay"\]\[dir="rtl"\] \.window-panes > :not\(:first-child\)\s*\{[^}]*\}/,
    );
    expect(rule?.[0]).toContain("padding-top: 2rem");
    expect(shell).toContain("window-panes");
  });

  it("does not let the shell grow with its content", () => {
    // `min-h-screen` is "at least the window" and grows past it; the shell needs "exactly".
    expect(shell).not.toContain("min-h-screen");
    expect(shell).toContain("h-full overflow-hidden");
  });
});
