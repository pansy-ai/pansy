// The phone's own settings: its appearance, the tool calls a thread shows
// (T3's four), the language being ui/i18n.ts's.
import { setColorScheme, signal, Storage } from "nsh";

export type Appearance = "system" | "light" | "dark";

/** T3's `toolCallVisibility`: every call, the failed alone, none, or only the answers ("Zen"). */
export type ToolCalls = "showAll" | "hideSuccessful" | "hideAll" | "zen";

const KEY = "pansy.appearance";
const TOOLS_KEY = "pansy.toolCalls";

export const appearance = signal<Appearance>("system");
export const toolCalls = signal<ToolCalls>("showAll");

export async function loadPrefs() {
  const saved = await Storage.getItem(KEY);
  if (saved === "light" || saved === "dark" || saved === "system") setAppearance(saved);
  const tools = await Storage.getItem(TOOLS_KEY);
  if (tools === "showAll" || tools === "hideSuccessful" || tools === "hideAll" || tools === "zen") toolCalls.set(tools);
}

export function setToolCalls(t: ToolCalls) {
  toolCalls.set(t);
  void Storage.setItem(TOOLS_KEY, t);
}

export function setAppearance(a: Appearance) {
  appearance.set(a);
  setColorScheme(a);
  void Storage.setItem(KEY, a);
}
