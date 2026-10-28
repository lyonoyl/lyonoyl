/**
 * Whether the window is dark, decided in one place.
 *
 * In a file of its own because two things have to give the same answer: the shell, which puts the
 * window into the dark, and the settings screen, which offers the dark colours only while they
 * are what the window wears. Two copies would be a screen offering colours nobody can see, or
 * hiding the ones somebody is looking at.
 */
import { useSyncExternalStore } from "react";

import type { ThemeDto } from "@/shared/ipc/generated/ThemeDto";

/** What the system says when it is dark. */
export const SYSTEM_IS_DARK = "(prefers-color-scheme: dark)";

/**
 * Whether the window is dark: a dark theme, or the system's while the system is dark. `system` is
 * not a third palette; it is following the system.
 */
export function isDark(theme: ThemeDto, systemIsDark: boolean): boolean {
  return theme === "dark" || (theme === "system" && systemIsDark);
}

/** Whether the system is dark now, read again whenever it changes. */
export function useSystemIsDark(): boolean {
  return useSyncExternalStore(listen, systemIsDarkNow, () => false);
}

function systemIsDarkNow(): boolean {
  return window.matchMedia(SYSTEM_IS_DARK).matches;
}

function listen(changed: () => void): () => void {
  const media = window.matchMedia(SYSTEM_IS_DARK);
  media.addEventListener("change", changed);
  return () => media.removeEventListener("change", changed);
}
