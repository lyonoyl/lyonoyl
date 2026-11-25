import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { FileName, kindOf } from "./FileName";

/** The name as it is drawn: the span that runs left to right whatever the name. */
function drawn(name: string): HTMLElement {
  const { container } = render(<FileName name={name} />);
  const shown = container.firstElementChild;
  if (!(shown instanceof HTMLElement)) throw new Error("nothing was drawn");
  return shown;
}

describe("kindOf", () => {
  it("names the kind a name ends in, in capitals", () => {
    expect(kindOf("invoice.pdf")).toBe("PDF");
    expect(kindOf("photo.jpeg")).toBe("JPEG");
  });

  it("names the kind its name ends in, never the one the message declared", () => {
    // A message can call an application a PDF, in the type it declares, which never reaches the
    // window, and earlier in the name, which does. The end of the name is what the system opens it
    // by, so the word is what the end says.
    expect(kindOf("Statement.pdf.app")).toBe("APP");
  });

  it("says nothing where the name ends in no kind anybody reads", () => {
    // No extension, and one that is not a word: nothing stands in for either.
    expect(kindOf("thing")).toBe("");
    expect(kindOf("Minutes v2. final draft")).toBe("");
    expect(kindOf(".hidden")).toBe("");
  });
});

describe("FileName", () => {
  it("keeps how a long name ends on screen, cutting it in the middle", () => {
    // Cut at its end, a long name loses its extension off screen, and the extension is what decides
    // what the file does when it is opened. The start gives way; the ending does not.
    const shown = drawn("Statement-for-your-account-ending-0000-and-more-words.pdf.app");

    const [start, ending] = Array.from(shown.children);
    expect(ending?.textContent).toBe(".app");
    expect(ending?.className).toContain("shrink-0");
    expect(ending?.className).not.toMatch(/truncate|ellipsis|overflow/);
    expect(start?.textContent).toBe(
      "Statement-for-your-account-ending-0000-and-more-words.pdf",
    );
    expect(start?.className).toContain("text-ellipsis");
  });

  it("keeps only the last characters of an ending longer than any extension", () => {
    // Kept whole, an ending of any length would take the whole room, and how the name starts would
    // be cut down to nothing.
    const shown = drawn(`notes.${"x".repeat(40)}`);

    const [start, ending] = Array.from(shown.children);
    expect(ending?.textContent).toBe("x".repeat(20));
    expect(start?.textContent).toBe(`notes.${"x".repeat(20)}`);
  });

  it("keeps an ending too wide for the room inside it", () => {
    // A dot and nineteen ideographs are within the characters an ending keeps whole, and wider
    // than the room a name has. Unclipped, they are drawn over the size beside them. The room
    // clips what does not fit it; the ending itself is still never cut short.
    const wide = "報".repeat(19);
    const shown = drawn(`notes.${wide}`);

    expect(shown.lastElementChild?.textContent).toBe(`.${wide}`);
    expect(shown.className).toMatch(/\bmax-w-56\b/);
    expect(shown.className).toMatch(/\boverflow-hidden\b/);
  });

  it("keeps the ending at the right of a name that starts right to left", () => {
    // Laid out in the name's own direction, one Hebrew letter at the front turns the row around:
    // the ending goes to the far left, drawn `app.`, and the last thing read is the `.pdf` before
    // it. The row runs left to right whatever the name; only the start keeps its own direction.
    const start = "אStatement-for-your-account-ending-0000.pdf";
    const shown = drawn(`${start}.app`);

    expect(shown.textContent).toBe(`${start}.app`);
    expect(shown.getAttribute("dir")).toBe("ltr");
    expect(shown.lastElementChild?.textContent).toBe(".app");
    expect(shown.lastElementChild?.closest("[dir]")?.getAttribute("dir")).toBe(
      "ltr",
    );
    expect(shown.firstElementChild?.textContent).toBe(start);
    expect(shown.firstElementChild?.getAttribute("dir")).toBe("auto");
  });
});
