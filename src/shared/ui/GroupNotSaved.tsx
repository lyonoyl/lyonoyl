import type { ReactNode } from "react";

/** The line a panel shows when the core did not do what it was asked, in the panel's own words. */
export function GroupNotSaved({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="text-sm text-destructive">
      {children}
    </p>
  );
}
