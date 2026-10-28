// React flushes its passive effects through `setImmediate`; Testing Library waits on
// `setTimeout(0)`. Which of the two lands first is nothing React promises, and on a busy machine
// it is decided by the machine. Delaying the first by one turn makes React lose that race every
// time, so an ordering that shows up in front of a person about once in twenty runs shows up here
// every run.
//
// This is a gate rather than a debugging aid, and it earned that: four separate defects were found
// under it in one pass, all the same shape — a value the next action depends on being set in an
// effect, which is to say after the browser has drawn. The window between the drawing and the
// effect is where a person's keystroke lands, and what they get there is the previous answer.
//
// Nothing here is unusual for React. What it removes is the luck.
const immediately = globalThis.setImmediate;

globalThis.setImmediate = ((run: (...given: unknown[]) => void, ...given: unknown[]) =>
  setTimeout(() => setTimeout(() => run(...given), 0), 0)) as unknown as typeof immediately;

export {};
