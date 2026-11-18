// What the Agents page keeps between its parts: which row is open, which
// picker, what is folded, and the page's arrangement (its order and the
// agents hidden by hand, kept in pansy's settings, never in an agent's
// files). Connecting and disconnecting go through here too.
import { signal } from "nsh";
import { Agents } from "pansy/native";
import { tr, trCore } from "./i18n";
import { act, agents, choiceName, choices, clis, loadAgents, loadVersions, reason, settings, toast, type Agent, type AgentField, type Cli, type FileChange, type SettingsView } from "./model";

/** The agent whose picker is open, and for which of its models: its main one, the one for quick tasks, its subagents', or a field's (a tier's). */
export const picking = signal<{ id: string; small: boolean; subagent?: boolean; field?: string } | null>(null);
/** The agent whose effort is being picked, and which of its efforts. */
export const efforting = signal<{ id: string; key: string } | null>(null);
/** The connected agent whose row is opened (Details). */
export const expanded = signal<string | null>(null);
/** The folded agents are shown. */
export const showAll = signal(false);
/** The hidden agents are shown, under the others. */
export const showHidden = signal(false);
/** Install another agent is open; `null`: as the list has it (open with no agents). */
export const installsOpen = signal<boolean | null>(null);
/** What pansy changed in each agent's config, as last read. */
export const changes = signal<Record<string, FileChange[]>>({});
/** The opened rows whose changes are shown, by agent. */
export const viewing = signal<Record<string, boolean>>({});
/** The agent asked about before it is disconnected. */
export const disconnecting = signal<Agent | null>(null);
/** Where it was asked: the main window, or the quick panel by the tray icon. */
export const disconnectIn = signal<"main" | "panel">("main");
/** Manage agents is open. */
export const managing = signal(false);

/** The agents on this computer, or connected: the rows of the list. */
export const here = () => agents().filter((a) => a.installed || a.connected);
export const hiddenIds = () => settings()?.agentsHidden ?? [];
export const isHidden = (a: Agent) => hiddenIds().includes(a.id);

/**
 * The rows in view, in order, and the folded rest: an agent not connected
 * is folded once one is (none connected, every one is in view), and one
 * hidden by hand always is.
 */
export function arrange() {
  const order = settings()?.agentOrder ?? [];
  const rank = (a: Agent) => {
    const i = order.indexOf(a.id);
    return i < 0 ? order.length : i;
  };
  const all = here()
    .map((a, i) => ({ a, i }))
    .sort((x, y) => rank(x.a) - rank(y.a) || x.i - y.i)
    .map((x) => x.a);
  const hidden = new Set(hiddenIds());
  const anyUsed = all.some((a) => !hidden.has(a.id) && a.connected);
  const inView = (a: Agent) => !hidden.has(a.id) && (!anyUsed || a.connected);
  return { all, shown: all.filter(inView), folded: all.filter((a) => !inView(a)) };
}

/** Saves the order and the hidden ones; the page shows them at once, and as they were if saving fails. */
export async function saveArrangement(order: string[], hidden: string[]): Promise<boolean> {
  const prev = settings();
  if (prev) settings.set({ ...prev, agentOrder: order, agentsHidden: hidden });
  try {
    settings.set(JSON.parse(await Agents.arrange(JSON.stringify(order), JSON.stringify(hidden))) as SettingsView);
    return true;
  } catch (error) {
    settings.set(prev);
    toast(reason(error), "error");
    return false;
  }
}

/** Puts the agent at `to` among the rows in view; the folded ones keep their places after them. */
export function moveAgent(id: string, to: number) {
  const { shown, folded } = arrange();
  const ids = shown.map((a) => a.id);
  const from = ids.indexOf(id);
  if (from < 0 || to < 0 || to >= ids.length || to === from) return;
  ids.splice(to, 0, ...ids.splice(from, 1));
  void saveArrangement([...ids, ...folded.map((a) => a.id)], hiddenIds());
}

export function setHidden(a: Agent, hide: boolean) {
  const { all } = arrange();
  const hidden = hiddenIds().filter((x) => x !== a.id);
  if (hide) hidden.push(a.id);
  void saveArrangement(
    all.map((x) => x.id),
    hidden,
  ).then((ok) => {
    if (!ok) return;
    const still = !hide && arrange().folded.some((x) => x.id === a.id);
    toast(hide ? tr("%s hidden · find it under Hidden at the bottom", a.name) : still ? tr("%s is no longer hidden · it stays under Not set up until it is connected", a.name) : tr("%s shown", a.name));
  });
}

export function loadChanges(id: string) {
  void Agents.changes(id)
    .then((json) => changes.update((c) => ({ ...c, [id]: JSON.parse(json) as FileChange[] })))
    .catch(() => changes.update((c) => ({ ...c, [id]: [] })));
}

export function connect(agent: Agent, model: string) {
  void act(`agent:${agent.id}`, () => Agents.connect(agent.id, model), tr("%s now uses %s", agent.name, choiceName(model))).then(() => {
    void loadAgents();
    if (expanded() === agent.id) loadChanges(agent.id);
  });
}

/** Switched on: back on what it was on when it was switched off, else its model is picked first. */
export function switchOn(agent: Agent) {
  const last = agent.recent.find((m) => choices().some((c) => c.id === m));
  if (last) connect(agent, last);
  else picking.set({ id: agent.id, small: false });
}

/** Asks first, with what goes back, in the window `where` the switch was. */
export function askDisconnect(agent: Agent, where: "main" | "panel" = "main") {
  disconnectIn.set(where);
  changes.update((c) => {
    const next = { ...c };
    delete next[agent.id];
    return next;
  });
  loadChanges(agent.id);
  disconnecting.set(agent);
}

export function disconnectNow(agent: Agent) {
  void act(`agent:${agent.id}`, () => Agents.disconnect(agent.id), tr("%s no longer goes through pansy; its own settings are back", agent.name)).then(() => {
    if (expanded() === agent.id) expanded.set(null);
    void loadAgents();
  });
}

export const fieldOf = (a: Agent, key: string): AgentField | undefined => a.fields.find((f) => f.key === key);

/** Claude Code's tiers, which `/model` switches between; each a field of its own but Haiku, the model for quick tasks. */
export const TIERS = ["opus", "sonnet", "haiku", "fable"] as const;
/** Whether the agent has tiers: its opened row then lays them out, each with its model and its effort. */
export const hasTiers = (a: Agent) => TIERS.some((t) => fieldOf(a, t));
/** The fields the tiers' rows show: the rest are a new session's own settings. */
const inTiers = (key: string) => TIERS.some((t) => key === t || key === `${t}_effort`) || key === "subagent_effort" || key === "auto_mode";
/** The agent's fields with no row of their own: its effort, and whatever a manifest of the user's adds. */
export const plainFields = (a: Agent) => a.fields.filter((f) => !(hasTiers(a) && inTiers(f.key)));

/** A field by the name the window gives it; one the window has no word for, by its key. */
export function fieldName(key: string): string {
  switch (key) {
    case "effort":
      return tr("effort");
    case "opus_effort":
      return tr("opus effort");
    case "sonnet_effort":
      return tr("sonnet effort");
    case "haiku_effort":
      return tr("haiku effort");
    case "fable_effort":
      return tr("fable effort");
    case "subagent_effort":
      return tr("subagent effort");
    case "auto_mode":
      return tr("auto mode");
    default:
      return key.replaceAll("_", " ");
  }
}

/** An effort as the window says it; `null`, the agent's own. */
export const effortName = (level: string | null) => (level ? trCore(level) : tr("default"));

/** Gives a field its value; empty takes it back. What pansy changed in the agent's files is read again. */
export function setField(agent: Agent, key: string, value: string) {
  const said = tr("%s %s → %s", agent.name, fieldName(key), !value ? tr("default") : fieldOf(agent, key)?.kind === "model" ? choiceName(value) : effortName(value));
  return act(`agent:${agent.id}`, () => Agents.setField(agent.id, key, value), said).then(() => {
    void loadAgents();
    if (expanded() === agent.id) loadChanges(agent.id);
  });
}

// ---- a row dragged by its logo ----

/** The row being dragged among the rows in view: where it was, where it would go, and how far it has come. */
export const dragging = signal<{ id: string; from: number; to: number; dy: number } | null>(null);
/** Each row's height as last laid out: an opened one is taller. */
const heights: Record<string, number> = {};
export const rowHeight = (id: string, height: number) => {
  heights[id] = height;
};
const heightOf = (id: string) => heights[id] ?? 56;

export function dragStart(id: string) {
  const from = arrange().shown.findIndex((a) => a.id === id);
  if (from >= 0) dragging.set({ id, from, to: from, dy: 0 });
}

/** The row has come `dy` from where it was: it takes the place of each row it is more than halfway over, and stays within the rows in view. */
export function dragMove(dy: number) {
  const d = dragging();
  if (!d) return;
  const ids = arrange().shown.map((a) => a.id);
  const below = ids.slice(d.from + 1).reduce((sum, id) => sum + heightOf(id), 0);
  const above = ids.slice(0, d.from).reduce((sum, id) => sum + heightOf(id), 0);
  const by = Math.max(-above, Math.min(below, dy));
  let to = d.from;
  let passed = 0;
  if (by > 0) {
    while (to < ids.length - 1 && by > passed + heightOf(ids[to + 1]) / 2) passed += heightOf(ids[++to]);
  } else {
    while (to > 0 && -by > passed + heightOf(ids[to - 1]) / 2) passed += heightOf(ids[--to]);
  }
  dragging.set({ ...d, to, dy: by });
}

/** Let go: dropped, the row takes its new place; else it goes back. */
export function dragEnd(drop: boolean) {
  const d = dragging();
  dragging.set(null);
  if (d && drop && d.to !== d.from) moveAgent(d.id, d.to);
}

/** How far a row is from its place while one is dragged: the dragged one follows the pointer, those it passed make room. */
export function shiftOf(id: string): number {
  const d = dragging();
  if (!d) return 0;
  if (id === d.id) return d.dy;
  const i = arrange().shown.findIndex((a) => a.id === id);
  if (i < 0) return 0;
  if (d.from < i && i <= d.to) return -heightOf(d.id);
  if (d.to <= i && i < d.from) return heightOf(d.id);
  return 0;
}

// ---- the agents' programs: their versions, and updating them ----

/** The agents whose programs are being updated now. */
export const updating = signal<string[]>([]);
/** Update all, once pressed: how far it is, which agent it is at, what changed and what failed. */
export const updatingAll = signal<{ n: number; done: number; now: string; changed: string[]; failed: { name: string; why: string }[] } | null>(null);

export const cliOf = (a: Agent): Cli | undefined => clis()[a.id];
/** The agents a newer version is out for, not being updated yet. */
export const updatable = () => here().filter((a) => cliOf(a)?.update && !updating().includes(a.id));
/** "Pi 1.0.9 → 1.1.0", or the name and the version now when the one before isn't known or is the same. */
const versionChange = (name: string, from?: string | null, to?: string | null) => (from && to && from !== to ? `${name} ${from} → ${to}` : [name, to].filter(Boolean).join(" "));

/** Updates one agent's program with the command its install tells; "" when it went well, else why not. `quiet` (Update all) leaves the saying to the caller. */
export async function updateCli(a: Agent, quiet = false): Promise<string> {
  if (updating().includes(a.id)) return tr("%s is already being updated", a.name);
  const from = cliOf(a)?.version;
  updating.update((u) => [...u, a.id]);
  try {
    const now = JSON.parse(await Agents.updateCli(a.id)) as Cli;
    clis.update((c) => ({ ...c, [a.id]: now }));
    const all = updatingAll();
    if (quiet && all) updatingAll.set({ ...all, changed: [...all.changed, versionChange(a.name, from, now.version)] });
    if (!quiet) toast(from && now.version && from !== now.version ? tr("%s updated: %s → %s", a.name, from, now.version) : tr("%s updated to %s", a.name, now.version ?? ""));
    return "";
  } catch (error) {
    if (!quiet) toast(reason(error), "error");
    void loadVersions(); // what it is now
    return reason(error);
  } finally {
    updating.update((u) => u.filter((id) => id !== a.id));
  }
}

/** Updates every agent a newer version is out for, one at a time: npm installs side by side collide. */
export async function updateAll() {
  const running = updatingAll();
  if (running && running.done < running.n) return;
  const ups = updatable();
  if (ups.length === 0) return;
  updatingAll.set({ n: ups.length, done: 0, now: ups[0].name, changed: [], failed: [] });
  for (const a of ups) {
    updatingAll.update((s) => s && { ...s, now: a.name });
    const why = await updateCli(a, true);
    updatingAll.update((s) => s && { ...s, done: s.done + 1, failed: why ? [...s.failed, { name: a.name, why }] : s.failed });
  }
  const end = updatingAll();
  if (end && end.failed.length > 0) toast(end.failed.map((f) => f.why).join(" · "), "error");
}

/** What a row can do: `gone`, its program not found; `empty`, no models to connect it to yet. */
export function kindOf(a: Agent): "gone" | "empty" | "ok" {
  if (a.cliMissing) return "gone";
  if (!a.connected && choices().length === 0) return "empty";
  return "ok";
}

/** The line under an agent's name. */
export function status(a: Agent): { tone: "on" | "off"; text: string } {
  if (kindOf(a) === "empty") return { tone: "off", text: tr("Add a key or a subscription first; then there are models to connect") };
  if (!a.connected && a.paused) return { tone: "off", text: tr("Restored for now · %s uses its own settings until it is connected again", a.name) };
  if (!a.connected) return { tone: "off", text: tr("Not connected · %s uses its own settings", a.name) };
  if (a.picksIn) return { tone: "on", text: tr("Connected · %d models in %s's %s", choices().length, a.name, a.picksIn) };
  if (hasTiers(a)) return { tone: "on", text: tr("Connected · switch the opus · sonnet · haiku tiers in /model") };
  if (a.id === "codex") return { tone: "on", text: tr("Connected · new sessions take it") };
  return { tone: "on", text: tr("Connected · starts on the model picked here") };
}
