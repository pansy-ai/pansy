// What the Agents page keeps between its parts: which row is open, which
// picker, what is folded, and the page's arrangement (its order and the
// agents hidden by hand, kept in pansy's settings, never in an agent's
// files). Connecting and disconnecting go through here too.
import { signal } from "nsh";
import { Agents } from "pansy/native";
import { tr, trCore } from "./i18n";
import { act, agents, choiceName, choices, clis, loadAgents, loadVersions, reason, settings, toast, type Agent, type AgentField, type AgentModels, type Choice, type Cli, type FileChange, type SettingsView } from "./model";

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
/** The agent whose list of models is being picked. */
export const listing = signal<Agent | null>(null);

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
/** Whether a field is a setting of the agent's own: a size, a share, a switch. */
const isSetting = (f: AgentField) => f.kind === "tokens" || f.kind === "percent" || f.kind === "switch";
/** The agent's fields with no row of their own: its effort, and whatever model or effort a manifest of the user's adds. */
export const plainFields = (a: Agent) => a.fields.filter((f) => !isSetting(f) && !(hasTiers(a) && inTiers(f.key)));
/** The share that goes with a switch, beside it in its row: `background_compaction_at` with `background_compaction`. */
export const shareOf = (a: Agent, f: AgentField) => (f.kind === "switch" ? a.fields.find((x) => x.kind === "percent" && x.key === `${f.key}_at`) : undefined);
/** The rows of the agent's own settings sheet: each setting but a share drawn beside its switch. */
export const settingFields = (a: Agent) => a.fields.filter((f) => isSetting(f) && !a.fields.some((s) => shareOf(a, s) === f));

/** A size as the window says it: 272K, 1M, 128.5K. */
export function sizeName(tokens: number): string {
  const short = (n: number) => String(Math.round(n * 10) / 10);
  return tokens >= 1_000_000 ? `${short(tokens / 1_000_000)}M` : tokens >= 1_000 ? `${short(tokens / 1_000)}K` : String(tokens);
}

/** A size as it is typed (`272k`, `1.5m`, `272000`); null when it is none. */
export function parseSize(text: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*([km]?)$/i.exec(text.trim().replaceAll(",", ""));
  if (!m) return null;
  const scale = m[2].toLowerCase() === "m" ? 1_000_000 : m[2].toLowerCase() === "k" ? 1_000 : 1;
  const n = Number(m[1]) * scale;
  return Number.isInteger(Math.round(n)) && (scale > 1 || Number.isInteger(Number(m[1]))) ? Math.round(n) : null;
}

/** The size a tokens setting starts from when it is first given one. */
export const startSize = (f: AgentField) => Math.max(f.min ?? 0, f.key === "context_window" ? 1_000_000 : 272_000);

/** A setting of the sheet by its name; one the window has no word for, by its key. */
export function settingName(key: string): string {
  switch (key) {
    case "compact_at":
      return tr("Long conversations");
    case "context_window":
      return tr("Context window");
    case "background_compaction":
      return tr("Background compaction");
    default:
      return fieldName(key);
  }
}

/** What a setting of the sheet does, under its name. */
export function settingSaid(a: Agent, key: string): string {
  switch (key) {
    case "compact_at":
      return a.id === "codex"
        ? tr("Codex summarises a conversation once it is this long; its own is 90% of the window it takes the model to have. The whole window is slower and costlier.")
        : tr("%s summarises a conversation once it is this long; its own is close to the model's whole window, which is slower and costlier.", a.name);
    case "context_window":
      return tr("The window %s takes the model to have, for a model it doesn't know or has wrong. It compacts at 90% of it at most.", a.name);
    case "background_compaction":
      return tr("Codex 0.163 and later summarise while a turn goes on, past the share of the window typed (65% when empty). A Codex feature still under development.");
    default:
      return "";
  }
}

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
    case "compact_at":
      return tr("compact at");
    case "context_window":
      return tr("context window");
    case "background_compaction":
      return tr("background compaction");
    case "background_compaction_at":
      return tr("background compaction at");
    default:
      return key.replaceAll("_", " ");
  }
}

/** An effort as the window says it; `null`, the agent's own. */
export const effortName = (level: string | null) => (level ? trCore(level) : tr("default"));

/** A field's value as the window says it. */
function valueName(f: AgentField | undefined, value: string): string {
  if (!value) return tr("default");
  switch (f?.kind) {
    case "model":
      return choiceName(value);
    case "tokens":
      return sizeName(parseSize(value) ?? 0);
    case "percent":
      return `${value.replace(/%$/, "")}%`;
    case "switch":
      return tr("On");
    default:
      return effortName(value);
  }
}

/** Gives a field its value; empty takes it back. What pansy changed in the agent's files is read again. */
export function setField(agent: Agent, key: string, value: string) {
  const said = tr("%s %s → %s", agent.name, fieldName(key), valueName(fieldOf(agent, key), value));
  return act(`agent:${agent.id}`, () => Agents.setField(agent.id, key, value), said).then(() => {
    void loadAgents();
    if (expanded() === agent.id) loadChanges(agent.id);
  });
}

// ---- the models an agent lists, and the names it lists them by ----

/** What the user picked for the agent's list; nothing picked, every model. */
export const listedOf = (a: Agent): AgentModels => settings()?.agentModels?.[a.id] ?? {};
/** Whether `models` leaves `id` in a list. */
export const lists = (models: AgentModels, id: string) => (models.onlyPicked ? (models.picked ?? []).includes(id) : !(models.hidden ?? []).includes(id));
/** The models the agent is set to: they stay in its list whatever is picked. */
export const heldBy = (a: Agent): string[] => [a.model, a.smallModel, a.subagentModel, ...a.fields.filter((f) => f.kind === "model").map((f) => f.value)].filter((m): m is string => !!m);
/** The models in the agent's list: those left in it, and those it is set to. */
export function shownTo(a: Agent): Choice[] {
  if (!a.listsModels) return choices();
  const models = listedOf(a);
  const held = heldBy(a);
  return choices().filter((c) => lists(models, c.id) || held.includes(c.id));
}

/** Saves which models the agent lists; a connected one's list is written again. */
export function setListed(a: Agent, models: AgentModels) {
  const prev = settings();
  if (prev) settings.set({ ...prev, agentModels: { ...prev.agentModels, [a.id]: models } });
  return act(`agent:${a.id}`, async () => {
    try {
      settings.set(JSON.parse(await Agents.setListed(a.id, JSON.stringify(models))) as SettingsView);
    } catch (error) {
      settings.set(prev);
      throw error;
    }
  }).then(() => {
    void loadAgents();
    if (expanded() === a.id) loadChanges(a.id);
  });
}

/** Whether the agent's list names a model's provider: its own way, or `null`, as Settings has it. */
export const namesOf = (a: Agent): boolean | null => settings()?.agentNames?.[a.id] ?? null;

/** Saves it: `on`, `off`, or "" to follow Settings again. */
export function setNames(a: Agent, mode: "" | "on" | "off") {
  return act(`agent:${a.id}`, async () => {
    settings.set(JSON.parse(await Agents.setNames(a.id, mode)) as SettingsView);
  }).then(() => {
    if (expanded() === a.id) loadChanges(a.id);
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
  if (!a.connected && a.signedIn) return { tone: "off", text: a.id === "claude-code" ? tr("Not connected · now on its Claude subscription") : a.id === "codex" ? tr("Not connected · now signed in with ChatGPT") : tr("Not connected · now on its own sign-in") };
  if (!a.connected) return { tone: "off", text: tr("Not connected · %s uses its own settings", a.name) };
  if (a.picksIn) return { tone: "on", text: tr("Connected · %d models in %s's %s", shownTo(a).length, a.name, a.picksIn) };
  if (hasTiers(a)) return { tone: "on", text: tr("Connected · switch the opus · sonnet · haiku tiers in /model") };
  if (a.id === "codex") return { tone: "on", text: tr("Connected · new sessions take it") };
  return { tone: "on", text: tr("Connected · starts on the model picked here") };
}
