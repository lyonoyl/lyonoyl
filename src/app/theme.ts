import type { DarkSchemeDto } from "@/shared/ipc/generated/DarkSchemeDto";
import type { LightSchemeDto } from "@/shared/ipc/generated/LightSchemeDto";
import type { ThemeDto } from "@/shared/ipc/generated/ThemeDto";
import { isDark, SYSTEM_IS_DARK } from "@/shared/ui/darkness";

const DARK_CLASS = "dark";
const SCHEME_ATTRIBUTE = "data-scheme";
const LIGHT_SCHEME_ATTRIBUTE = "data-light-scheme";

/**
 * Applies the stored appearance to the document.
 *
 * Three settings, because they answer three questions. The theme decides *whether* it is dark —
 * `system` is not a third palette, it means "follow the OS", so it is resolved here against
 * `prefers-color-scheme` and re-resolved whenever the OS setting changes. The two schemes decide
 * what dark and what light look like, and each is named on the root element for the stylesheet to
 * match.
 *
 * Both schemes are set whatever the theme says: the side not worn ignores its own, and it is then
 * already right the moment the OS turns over, with no flash of the wrong colours in between.
 */
export function applyTheme(
  theme: ThemeDto,
  scheme: DarkSchemeDto = "graphite",
  lightScheme: LightSchemeDto = "paper",
): () => void {
  const media = window.matchMedia(SYSTEM_IS_DARK);

  document.documentElement.setAttribute(SCHEME_ATTRIBUTE, scheme);
  document.documentElement.setAttribute(LIGHT_SCHEME_ATTRIBUTE, lightScheme);

  const resolve = () => {
    document.documentElement.classList.toggle(
      DARK_CLASS,
      isDark(theme, media.matches),
    );
  };

  resolve();

  // Only the `system` setting depends on the OS; an explicit choice must not drift with it.
  if (theme !== "system") {
    return () => {};
  }
  media.addEventListener("change", resolve);
  return () => media.removeEventListener("change", resolve);
}
