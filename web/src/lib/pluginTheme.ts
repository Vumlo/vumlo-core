/**
 * The design tokens a plugin page may draw with (ADR-0066). A page in a sandboxed frame cannot share
 * the components of this app, and should not; what it can share is the look. The colours are read
 * from the page as they are now, so none is written here and the plugin follows the theme.
 */
const PLUGIN_TOKENS = [
  "--bg", "--surface", "--sunken", "--hover", "--border", "--text", "--muted",
  "--accent", "--accent-text", "--accent-soft", "--ok-soft", "--ok-fg", "--danger", "--danger-soft", "--input-bg",
] as const;

/** The computed value of each token on `<html>`, for the current theme. */
export function readPluginTokens(): Record<string, string> {
  const style = getComputedStyle(document.documentElement);
  return Object.fromEntries(PLUGIN_TOKENS.map((name) => [name, style.getPropertyValue(name).trim()]));
}
