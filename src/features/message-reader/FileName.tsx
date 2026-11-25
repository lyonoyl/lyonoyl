/**
 * The extension a file's name ends in, without its dot, or nothing where the name has none.
 *
 * What the system reads to decide what a saved file is and what opens it. A dot at the very start
 * is not one — it is what hides a file — and the window does not rely on the names reaching it
 * already without such a dot.
 */
function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");

  return dot > 0 ? name.slice(dot + 1) : "";
}

/**
 * The word for a kind of file: the extension its name ends in, in capitals.
 *
 * Taken from the name and never from the type the message declared for the part. The extension is
 * what the system goes by once the file is saved; the declared type is only the sender's word for
 * it, and a message can call an application a PDF — a row reading PDF beside it would be this
 * window repeating the claim. Not a catalogue key and not translated: an extension is part of the
 * name the message gave the file.
 *
 * Shown only when it is short and plain. An extension is whatever follows the last dot, and
 * `final draft` is not a word for a kind of anything. Where there is no word, the row is a name and
 * a size.
 */
export function kindOf(name: string): string {
  const extension = extensionOf(name);

  return /^[a-z0-9]{1,8}$/i.test(extension) ? extension.toUpperCase() : "";
}

/** The most of a name's end that is kept whole: any extension a system reads, and not much more. */
const LONGEST_ENDING = 20;

/**
 * A file's name as two parts: the start, which may be cut short, and the ending, which never is.
 *
 * The ending is the extension, dot and all. A name cut at its end loses exactly that, and a sender
 * who knows it writes `Statement-for-your-account-ending-0000.pdf.app`: cut there, it reads as a
 * document and opens as an application. So the cut goes in the middle, and the ending stays on
 * screen. Past the length of any extension a system reads, only the last characters are kept
 * whole, so a name whose last dot is followed by a sentence rather than an extension still shows
 * how it starts.
 */
function startAndEnding(name: string): [string, string] {
  const extension = extensionOf(name);
  if (extension === "") return [name, ""];

  const characters = Array.from(name);
  const kept = Math.min(Array.from(extension).length + 1, LONGEST_ENDING);

  return [
    characters.slice(0, characters.length - kept).join(""),
    characters.slice(characters.length - kept).join(""),
  ];
}

/**
 * A file's name, cut in the middle where it does not fit, so that how it ends is always on screen.
 *
 * The two parts run left to right whatever script the name is in, and the ending, read left to
 * right as well, is always at the right edge. Laid out in the name's own direction, one
 * right-to-left letter at the front would turn the row around: the ending at the far left, drawn
 * `app.`, and the `.pdf` before it the last thing read. The start keeps the name's own direction
 * and gives way; the ending does not.
 *
 * The name never takes more than its room. An ending wider than the room is clipped at the room's
 * edge rather than drawn over the size beside it — no extension anything opens a file by is that
 * wide.
 *
 * Spaces are kept as written: a space just before the last dot is part of the name, and drawn as
 * two parts it would otherwise be dropped where they meet.
 */
export function FileName({ name }: { name: string }) {
  const [start, ending] = startAndEnding(name);

  return (
    <span dir="ltr" className="flex min-w-0 max-w-56 overflow-hidden">
      <span dir="auto" className="overflow-hidden text-ellipsis whitespace-pre">
        {start}
      </span>
      {ending && <span className="shrink-0 whitespace-pre">{ending}</span>}
    </span>
  );
}
