/**
 * The direction of a field somebody types into.
 *
 * Empty, a field takes the window's direction, so its caret and its placeholder start where the
 * window's lines do. With a letter in it, the letters decide: a subject in English runs left to
 * right in an Arabic window, and one in Arabic the other way in an English one. `auto` on an empty
 * field would not do: with nothing to read it runs left to right whatever the window.
 */
export function typedDirection(value: string): "auto" | undefined {
  return /\p{L}/u.test(value) ? "auto" : undefined;
}
