/**
 * Leaving a screen, the one way every route out of one takes.
 *
 * Its own module because two files have to agree on it: the cross the pane puts in its corner, and
 * the key the window answers. A second copy is how the two start costing different things — which
 * is the whole of what invariant 20 refuses.
 *
 * The blur is not decoration. A field that writes down what it holds when it is left — a group's
 * name — has to actually be left, and neither pressing a button nor pressing a key does that on
 * its own: on macOS a button takes no focus when it is clicked, and a key never takes it anywhere.
 *
 * What `close` does is the screen's own business, and it may be nothing at all: the composer
 * refuses while a message is going out, and the account form refuses while it is working. The
 * refusal belongs to the screen, so neither the cross nor the key has to know about it.
 */
export function leaveScreen(close: () => void): void {
  const filling = document.activeElement;
  if (filling instanceof HTMLElement) filling.blur();

  close();
}
