// What can be done to a thread from its row's menu or its page's: archive,
// snooze, pin, rename, mark unread, copy its link, delete its record
// (thread.update / thread.delete; the agent's session files stay).
import { Clipboard, effect, signal, View } from "nsh";
import { Haptics } from "nsh/mobile";
import type { ThreadSummary } from "../net/types";
import { conn, putSummary, shells } from "../state/store";
import { tr } from "./i18n";
import { BottomSheet, Btn, Field, toast } from "./parts";

async function update(machine: string, threadId: string, patch: Record<string, unknown>): Promise<boolean> {
  const c = conn(machine);
  if (!c) return false;
  try {
    const r = await c.call<{ thread: ThreadSummary }>("thread.update", { threadId, ...patch });
    if (r?.thread) putSummary(machine, r.thread);
    return true;
  } catch (e) {
    toast(tr("Couldn't change it: %s", String((e as Error).message ?? e)), { tone: "error" });
    return false;
  }
}

export async function archive(machine: string, t: ThreadSummary, on = true) {
  if (!(await update(machine, t.id, { archived: on }))) return;
  Haptics.impact("medium");
  if (on) toast(tr("Archived “%s”", t.title), { action: { label: tr("Undo"), onPress: () => void update(machine, t.id, { archived: false }) } });
}

/** Snoozes `t` for `hours`: it waits on the Later shelf till then. */
export async function snooze(machine: string, t: ThreadSummary, hours: number | null) {
  const until = hours === null ? null : new Date(Date.now() + hours * 3_600_000).toISOString();
  if (await update(machine, t.id, { snoozedUntil: until })) if (until) toast(tr("Snoozed “%s”", t.title));
}

/** Hours from now till tomorrow at nine. */
export function tillTomorrow(): number {
  const d = new Date();
  const t = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1, 9);
  return (t.getTime() - d.getTime()) / 3_600_000;
}

export const pin = (machine: string, t: ThreadSummary) => update(machine, t.id, { pinned: !t.pinned });
export const markUnread = (machine: string, t: ThreadSummary) => update(machine, t.id, { unread: true });

export async function copyLink(machine: string, t: ThreadSummary) {
  try {
    await Clipboard.writeText(`pansy://thread/${encodeURIComponent(machine)}/${encodeURIComponent(t.id)}`);
    Haptics.selection();
    toast(tr("Link copied"));
  } catch {
    toast(tr("Couldn't copy"), { tone: "error" });
  }
}

export async function remove(machine: string, t: ThreadSummary) {
  const c = conn(machine);
  if (!c) return;
  try {
    await c.call("thread.delete", { threadId: t.id });
    shells.set({ ...shells(), [machine]: (shells()[machine] ?? []).filter((x) => x.id !== t.id) });
    toast(tr("Deleted the thread's record"));
  } catch (e) {
    toast(tr("Couldn't change it: %s", String((e as Error).message ?? e)), { tone: "error" });
  }
}

/** The thread being renamed. */
export const renaming = signal<{ machine: string; thread: ThreadSummary } | null>(null);

export function RenameSheet() {
  const title = signal("");
  const open = () => renaming() !== null;
  effect(() => {
    const r = renaming();
    if (r) title.set(r.thread.title);
  });
  const save = async () => {
    const r = renaming();
    if (!r || !title().trim()) return;
    if (await update(r.machine, r.thread.id, { title: title().trim() })) renaming.set(null);
  };
  return (
    <BottomSheet open={open()} onClose={() => renaming.set(null)} title={tr("Rename")} start={0.4} testID="rename-sheet">
      <View class="gap-3 px-5 pb-8 pt-2">
        <Field value={title()} onChange={title.set} placeholder={tr("Title")} testID="rename-field" autoFocus onSubmit={() => void save()} />
        <Btn label={tr("Save")} onPress={() => void save()} testID="rename-save" />
      </View>
    </BottomSheet>
  );
}
