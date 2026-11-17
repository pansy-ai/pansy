// What the Agents page keeps between its parts: which row is open, which
// picker, what is folded, and the page's arrangement (its order and the
// agents hidden by hand, kept in pansy's settings, never in an agent's
// files). Connecting and disconnecting go through here too.
import { signal } from "nsh";
import { Agents } from "pansy/native";
import { tr } from "./i18n";
import { act, agents, choiceName, choices, loadAgents, reason, settings, toast, type Agent, type FileChange, type SettingsView } from "./model";

/** The agent whose picker is open, and for which of its models. */
export const picking = signal<{ id: string; small: boolean } | null>(null);
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
  if (a.id === "claude-code" || a.id === "codex") return { tone: "on", text: tr("Connected · new sessions take it") };
  return { tone: "on", text: tr("Connected · starts on the model picked here") };
}
