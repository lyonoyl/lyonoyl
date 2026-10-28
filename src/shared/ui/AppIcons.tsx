import type { AppIconDto } from "@/shared/ipc/generated/AppIconDto";

import ink from "./app-icons/ink.png";
import indigo from "./app-icons/indigo.png";
import dusk from "./app-icons/dusk.png";
import plum from "./app-icons/plum.png";
import graphite from "./app-icons/graphite.png";
import sky from "./app-icons/sky.png";
import paper from "./app-icons/paper.png";
import ember from "./app-icons/ember.png";
import rust from "./app-icons/rust.png";

/** Every picture the application can show itself as, in the order offered. */
const PICTURES: ReadonlyArray<{ value: AppIconDto; src: string }> = [
  { value: "ink", src: ink },
  { value: "indigo", src: indigo },
  { value: "dusk", src: dusk },
  { value: "plum", src: plum },
  { value: "graphite", src: graphite },
  { value: "sky", src: sky },
  { value: "paper", src: paper },
  { value: "ember", src: ember },
  { value: "rust", src: rust },
];

/** The picture for one choice: the same whether the window is light or dark. */
export function appIconPicture(value: AppIconDto): string {
  return (PICTURES.find((one) => one.value === value) ?? PICTURES[0]).src;
}

/**
 * The pictures the application can show itself as, offered as the pictures and nothing else.
 *
 * What is chosen here is what the Dock shows while the application runs, in a light window and a dark
 * one alike. There is no word under any of them: a picture is chosen by how it looks. The group is named by the
 * heading it stands under.
 *
 * All nine stand in one row, as the Dock holds them, and the row narrows with the room it is given
 * rather than wrapping: a row that wraps leaves one picture alone under eight.
 *
 * **The picture is what is square, and the button only wraps it.** WebKit, which draws the window on
 * macOS, lays out a square button's padding outside the square, so a picture filling a square button
 * is drawn taller than it is wide.
 */
export function AppIcons({
  chosen,
  labelledBy,
  disabled = false,
  onChoose,
}: {
  chosen: AppIconDto;
  /** The heading that names the group. */
  labelledBy: string;
  disabled?: boolean;
  onChoose: (value: AppIconDto) => void;
}) {
  return (
    <div
      role="group"
      aria-labelledby={labelledBy}
      className="grid w-full max-w-[36rem] grid-cols-9 gap-1"
    >
      {PICTURES.map((picture) => (
        <button
          key={picture.value}
          type="button"
          aria-pressed={chosen === picture.value}
          data-app-icon={picture.value}
          disabled={disabled}
          onClick={() => onChoose(picture.value)}
          className={`rounded-xl p-0.5 transition-colors ${
            chosen === picture.value
              ? "bg-muted ring-2 ring-ring"
              : "hover:bg-muted/60"
          }`}
        >
          <img
            src={picture.src}
            alt=""
            draggable={false}
            className="block aspect-square h-auto w-full"
          />
        </button>
      ))}
    </div>
  );
}
