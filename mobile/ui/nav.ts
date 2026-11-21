// Where the user is: the tab, each tab's stack of pages, and the sheets
// that open over everything (adding a machine, a new task).
import { signal } from "nsh";
import { createNavStack } from "nsh/base";

export type Tab = "chats" | "settings";
export type ChatRoute = { name: "list" } | { name: "thread"; machine: string; thread: string };
export type SettingsRoute = { name: "root" } | { name: "machine"; id: string } | { name: "about" };

export const tab = signal<Tab>("chats");
export const chats = createNavStack<ChatRoute>([{ name: "list" }]);
export const settingsNav = createNavStack<SettingsRoute>([{ name: "root" }]);

/** The pairing sheet, and the link it was opened with (the page's `#pair=`). */
export const pairOpen = signal(false);
export const pairLink = signal<{ base: string; secret: string } | null>(null);

/** The new task screen, and the machine it starts on. */
export const newTask = signal<{ machine?: string } | null>(null);

export function openThread(machine: string, thread: string) {
  tab.set("chats");
  chats.setRoutes([{ name: "list" }, { name: "thread", machine, thread }]);
}
