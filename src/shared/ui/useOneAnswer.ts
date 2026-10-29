import { useRef, useState } from "react";

/**
 * One question to the core at a time, and what came of the last one.
 *
 * The guard is set by the press itself. Two presses of Enter land before the first answer does,
 * and a guard read from what was drawn would let both through: two groups of one name, or one
 * group removed twice.
 */
export function useOneAnswer(onDone: () => void) {
  const [busy, setBusy] = useState(false);
  const [refused, setRefused] = useState(false);
  const asking = useRef(false);

  const send = (ask: () => Promise<boolean>) => {
    if (asking.current) return;
    asking.current = true;
    setBusy(true);
    setRefused(false);
    void ask()
      .catch(() => false)
      .then((done) => {
        asking.current = false;
        if (done) {
          onDone();
          return;
        }
        // What was typed stays: refused, it is what the person tries again with.
        setBusy(false);
        setRefused(true);
      });
  };

  return { busy, refused, send };
}
