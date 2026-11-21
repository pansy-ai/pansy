// The phone's own settings: its appearance (the language is ui/i18n.ts's).
import { setColorScheme, signal, Storage } from "nsh";

export type Appearance = "system" | "light" | "dark";

const KEY = "pansy.appearance";

export const appearance = signal<Appearance>("system");

export async function loadPrefs() {
  const saved = await Storage.getItem(KEY);
  if (saved === "light" || saved === "dark" || saved === "system") setAppearance(saved);
}

export function setAppearance(a: Appearance) {
  appearance.set(a);
  setColorScheme(a);
  void Storage.setItem(KEY, a);
}
