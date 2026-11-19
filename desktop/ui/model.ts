// The window's state: what pansy/native reports, and the actions that
// change it. Every action ends by reloading what it changed, so the window
// shows what is on disk.
import { batch, Notifications, signal } from "nsh";
import { Agents, Gateway, Groups, Masking, Plugins, Prefs, Profiles, Providers, Sessions, Settings, Subscriptions, Usage } from "pansy/native";
import { tr } from "./i18n";

export type Protocol = "anthropic" | "chat" | "responses" | "gemini";

/** A subscription pansy serves: ChatGPT's (as Codex uses it), GitHub Copilot's, Gemini Code Assist's (as Gemini CLI uses it). */
export type SubscriptionKind = "codex" | "copilot" | "gemini-cli";

/** A subscription's account: who is signed in, on which plan, kept by the agent or by pansy. */
export interface SignIn {
  kind: SubscriptionKind;
  user: string;
  plan?: string;
  source: "agent" | "pansy";
  /** The Google Cloud project it bills, when one was named. */
  project?: string;
}

/** One window of a subscription's allowance. */
export interface AllowanceWindow {
  /** `5 hours`, `7 days`, `Premium requests`, or a model's id. */
  name: string;
  /** The share used, 0 to 100. */
  used: number;
  resetsAt?: string;
  spanSecs?: number;
  /** A count, when the vendor counts it (`120 / 300`). */
  display?: string;
  unlimited?: boolean;
}

/** What one subscription account has used of its allowance (`Subscriptions.allowances`). */
export interface Allowance {
  provider: string;
  account: string;
  kind: SubscriptionKind;
  user: string;
  plan?: string;
  enabled: boolean;
  windows: AllowanceWindow[];
  /** Held by its vendor until then: the gateway sends the next account. */
  spentUntil?: string;
  note?: string;
  error?: string;
  readAt: string;
}

/** What is left on one API key, as its vendor's or its relay's balance query says (`Subscriptions.balances`). */
export interface Balance {
  provider: string;
  account: string;
  /** The provider's name. */
  name: string;
  /** The key's name, when the provider has several keys on. */
  user?: string;
  /** `$12.50`, `¥110.00 · $3.00`. */
  amount?: string;
  /** A relay key's own spending windows. */
  windows?: AllowanceWindow[];
  error?: string;
  readAt: string;
}

/** Where a sign-in begun from the window stands (`Subscriptions.signIn`). */
export interface SignInState {
  id: string;
  kind: SubscriptionKind;
  how: "browser" | "code";
  url: string;
  code: string;
  state: "starting" | "waiting" | "done" | "failed" | "canceled";
  user: string;
  plan: string;
  account: string;
  error: string;
}

export interface Account {
  id: string;
  label: string;
  display: string;
  masked: string;
  enabled: boolean;
  /** The key's own proxy, over the provider's; empty for none. */
  proxy: string;
  /** Set on a subscription's account, which has no key. */
  signIn: SignIn | null;
}

export interface ModelInfo {
  id: string;
  name: string;
  context: number | null;
  hidden: boolean;
  manual: boolean;
  /** The APIs it is served on, when its vendor says; empty for any. */
  apis: Protocol[];
}

export type Endpoints = { protocol: Protocol; baseUrl: string }[];
export type PresetKind = "vendor" | "relay" | "local";

export interface Provider {
  id: string;
  name: string;
  /** The preset it was made from (by its id now), or null for one of the user's own. */
  preset: string | null;
  /** Its logo in ui/logos, `generic` for one of the user's own. */
  icon: string;
  kind: PresetKind | null;
  note: string;
  enabled: boolean;
  keyless: boolean;
  /** Where its keys are made, and its vendor's site, at its plan or region. */
  keyUrl: string;
  website: string;
  /** The plan or region of its preset it is at, by its endpoints. */
  region: string | null;
  /** Where its endpoints are (`http://localhost:11434`). */
  address: string;
  /** The header its key goes in on the Anthropic API that the user picked or pansy learned. */
  authHeader: "x-api-key" | "bearer" | null;
  /** The one it goes in now: the vendor's own (Anthropic's, Bedrock's) or the one above; null for both. */
  anthropicAuth: "x-api-key" | "bearer" | null;
  headers: Record<string, string>;
  endpoints: Endpoints;
  accounts: Account[];
  models: ModelInfo[];
  proxy: string;
  /** The subscription it serves, its accounts sign-ins; null for keys. */
  subscription: SubscriptionKind | null;
}

/** One of a preset's plans or regions, each at endpoints of its own. */
export interface Region {
  id: string;
  name: string;
  endpoints: Endpoints;
  keyUrl: string;
  website: string;
  /** Its models are asked for after it is picked. */
  lists: boolean;
}

export interface Preset {
  id: string;
  name: string;
  /** The add sheet's name for it, when `name` is long. */
  short: string;
  icon: string;
  kind: PresetKind;
  note: string;
  host: string;
  endpoints: Endpoints;
  website: string;
  keyUrl: string;
  keyHint: string;
  keyless: boolean;
  models: string[];
  regions: Region[];
  /** What the regions choose between: Region or Plan. */
  regionLabel: string;
  headerHints: string[];
  /** For a vendor at the user's own resource (Azure): an example of its address. */
  endpoint: string;
  endpointHint: string;
}

/** One more thing an agent may be given (`pansy_agents::FieldView`): a model of pansy's or how hard a model thinks. */
export interface AgentField {
  /** How its manifest names it: `effort`, `opus`, `opus_effort`. */
  key: string;
  /** A model, an effort, or a setting of the agent's own: a size in tokens, a percent, a switch (`on`). */
  kind: "model" | "effort" | "tokens" | "percent" | "switch";
  /** An effort's levels, the lowest first. */
  levels: string[];
  /** With none given, the main model's. */
  follows: boolean;
  /** The least and the most a size may be. */
  min: number | null;
  max: number | null;
  /** What the user gave it, while the agent is connected. */
  value: string | null;
}

/** An agent's program (`pansy_agents::cli::Cli`): its version, what installed it and the version that is out. */
export interface Cli {
  version: string | null;
  /** `npm`, `bun`, `pnpm`, `mise`, or `own` for its own updater; null when its install doesn't tell. */
  via: string | null;
  /** The command that updates it. */
  command: string | null;
  latest: string | null;
  /** A newer version is out. */
  update: boolean;
}

/** One agent as `pansy_agents::AgentView` serializes it. */
export interface Agent {
  id: string;
  name: string;
  protocol: Protocol;
  installed: boolean;
  /** Its settings are here, its program isn't. */
  cliMissing: boolean;
  /** How to install it: `via` is script, powershell, brew or npm. */
  install: { via: string; command: string }[];
  /** Where it picks among the gateway's models, for one that lists them all. */
  picksIn: string | null;
  connected: boolean;
  model: string | null;
  smallModel: string | null;
  hasSmallModel: boolean;
  /** The model its subagents start on when they name none, as pansy set it. */
  subagentModel: string | null;
  /** It has a setting for that model. */
  hasSubagentModel: boolean;
  /** Every subagent of it goes on that model, whatever one its lead names: the gateway sees to it. */
  subagentEvery: boolean;
  /** What else may be set for it: a tier's model, an effort, a setting of its own. */
  fields: AgentField[];
  /** It lists pansy's models itself: which ones is the user's pick. */
  listsModels: boolean;
  /** Not connected, it is signed in to its vendor itself (its Claude subscription, ChatGPT). */
  signedIn: boolean;
  configFiles: string[];
  recent: string[];
  note: string | null;
  /** On its own config for now (Settings › Diagnostics): the model to connect it again with. */
  paused: string | null;
}

/** What pansy changed in one file of an agent's config (keys masked). */
export interface FileChange {
  path: string;
  /** pansy made the file: disconnecting removes it. */
  created: boolean;
  keys: { key: string; before: string | null; after: string | null }[];
}

export interface Choice {
  id: string;
  provider: string;
  name: string;
  group: boolean;
}

export type Routing = "order" | "rotate" | "usage";

export interface Group {
  id: string;
  name: string;
  members: string[];
  routing: Routing;
  /** A middleware's routing that orders the members (`<plugin>/<name>`), over `routing`. */
  strategy?: string | null;
}

// --- Plugins (`Plugins` of pansy/native) ---------------------------------------------

/** A way a plugin's provider is signed in to. */
export interface PluginMethod {
  type: "oauth" | "api";
  label: string;
  labels?: Record<string, string>;
  keyless?: boolean;
  keyAsked?: boolean;
  placeholder?: string;
}

/** A provider a plugin signs in to, with its accounts. */
export interface PluginProvider {
  id: string;
  name: string;
  methods: PluginMethod[];
  models: number;
  listError: string;
  accounts: { key: string; kind: string; user: string }[];
}

/** A plugin added, as the Plugins page shows it. */
export interface PluginRow {
  spec: string;
  package: string;
  name: string;
  version: string;
  description: string;
  /** package.json's `magpie.icon`: a LobeHub icon's name, an https picture or a data URI. */
  icon: string;
  off: boolean;
  /** `provider`, `pi`, `middleware`, `hooks`, `agent`. */
  kinds: string[];
  /** Loaded from a path, not installed. */
  path: boolean;
  options: Record<string, unknown>;
  /** package.json's `magpie.options`: an example of its options. */
  example: unknown;
  error: string | null;
  /** The name its middleware goes by. */
  middleware: string | null;
  providers: PluginProvider[];
  agents: { id: string; name: string }[];
}

/** The market's shelves, in the order it shows them: OpenCode plugins and pi packages first. */
export type Shelf = "opencode" | "pi" | "middleware" | "agent" | "tools" | "page";

/** A community plugin, as magpie's market lists it. */
export interface MarketEntry {
  package: string;
  name: string;
  /** `magpie` (the hub's curated list, begun from magpie-community's), `opencode` or `pi` (another author's, on npm). */
  source: string;
  /** Written for magpie by its community. */
  community: boolean;
  publisher: string;
  version: string;
  /** npm's downloads last week; 0 when not known. */
  weekly: number;
  kind: string;
  shelf: Shelf;
  summary: Record<string, string>;
  providers: string[];
  icon: string;
  /** pansy runs its kind. */
  runs: boolean;
}

/** Where a sign-in to a plugin's provider stands (`Plugins.signIn`). */
export interface PluginFlow {
  id: string;
  provider: string;
  method: number;
  state: "asking" | "key" | "waiting" | "code" | "done" | "failed" | "canceled";
  prompt: { type: string; key: string; message: string; placeholder?: string; options?: { label: string; value: string; hint?: string }[] } | null;
  invalid: string;
  url: string;
  instructions: string;
  opened: boolean;
  account: string;
  error: string;
}

/** A middleware's calls, as it ran. */
export interface MiddlewareStats {
  hooks: string[];
  calls: number;
  failures: number;
  avgMs: number;
  lastError: string;
  error: string | null;
}

export const plugins = signal<PluginRow[]>([]);
export const pluginStats = signal<Record<string, MiddlewareStats>>({});
/** The community plugins, once read. */
export const market = signal<MarketEntry[] | null>(null);
/** The Plugins page's tab: the market, or the plugins installed. */
export const pluginTab = signal<"discover" | "installed">("discover");
/** The routings middleware give, `<plugin>/<name>`. */
export const pluginRoutings = signal<string[]>([]);
/** What magpie has to bring over. */
export const magpieFound = signal<{ dir: string; plugins: string[]; accounts: number } | null>(null);

export async function loadPlugins() {
  try {
    plugins.set(JSON.parse(await Plugins.list()) as PluginRow[]);
    if (plugins().some((p) => p.middleware && !p.off)) pluginStats.set(JSON.parse(await Plugins.stats()) as Record<string, MiddlewareStats>);
    magpieFound.set(JSON.parse(await Plugins.magpie()));
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadMarket(fresh = false) {
  try {
    market.set(JSON.parse(await Plugins.discover(fresh)) as MarketEntry[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadRoutings() {
  try {
    pluginRoutings.set(JSON.parse(await Plugins.routings()) as string[]);
  } catch {
    pluginRoutings.set([]);
  }
}

export interface Profile {
  name: string;
  agents: Record<string, { model: string; smallModel?: string | null; subagentModel?: string | null; fields?: Record<string, string> }>;
  active: boolean;
}

export interface Totals {
  requests: number;
  errors: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  reasoning: number;
}

export interface UsageRecord {
  at: string;
  agent: string;
  requested: string;
  provider: string;
  account?: string;
  model: string;
  client: Protocol;
  upstream: Protocol;
  status: number;
  input: number;
  output: number;
  cacheRead: number;
  ms: number;
  failovers: number;
  error?: string;
}

export interface UsageSummary {
  days: number;
  total: Totals;
  byDay: Record<string, Totals>;
  byModel: Record<string, Totals>;
  byAgent: Record<string, Totals>;
  byProvider: Record<string, Totals>;
  /** By the folder `pansy run` started the agent in. */
  byProject: Record<string, Totals>;
  latest: UsageRecord[];
}

/** What masking does with a kind of value: masks it, sends it as written and says so, or holds the request back. */
export type MaskAction = "mask" | "warn" | "block";

/** Settings › Privacy's masking (`pansy_mask::Options`). */
export interface MaskOptions {
  secrets: boolean;
  personal: boolean;
  kinds: Record<string, boolean>;
  words: string[];
  wordsAnyCase: boolean;
  wordsWhole: boolean;
  rules: { kind: "prefix" | "regex"; value: string; name?: string }[];
  actions: Record<string, MaskAction>;
  allow: string[];
  shape: boolean;
}

export interface SettingsView {
  port: number;
  lan: boolean;
  gatewayKey: string;
  theme: "system" | "light" | "dark";
  language: "system" | "en" | "zh-Hans";
  /** "" goes through the system's proxy, "direct" through none, else this one. */
  proxy: string;
  closeToTray: boolean;
  /** What a click on the tray icon opens: the quick panel by it, or the main window. */
  trayClick: "panel" | "window";
  cooldownSecs: number;
  maxAttempts: number;
  firstByteTimeoutSecs: number;
  /** The sidebar shows; hidden, the pages are tabs in the title bar. */
  sidebar: boolean;
  hiddenPages: string[];
  bareModelIds: boolean;
  origins: string[];
  masking: MaskOptions;
  maskAlert: boolean;
  /** The Agents page's order, by agent id. */
  agentOrder: string[];
  /** The agents hidden on the Agents page. */
  agentsHidden: string[];
  /** Serve the subscriptions Codex and Copilot are signed in to, read where they keep them. */
  agentSignIns: boolean;
  /** Where API keys are kept: `file` or `system`. */
  keyStore: string;
  /** The Plugins page's 「国内镜像」: the plugin list, Bun and plugins from mirrors in China first. */
  chinaMirror: boolean;
  /** A model is listed with its provider's name before its own. */
  providerInNames: boolean;
  /** The agents that have it their own way, by agent id. */
  agentNames: Record<string, boolean>;
  /** Which models each agent lists, by agent id; one not here lists all. */
  agentModels: Record<string, AgentModels>;
}

/** Which of pansy's models one agent lists: all but `hidden`, or with `onlyPicked` the `picked` alone. */
export interface AgentModels {
  hidden?: string[];
  onlyPicked?: boolean;
  picked?: string[];
}

/** One value a request had masked, as often as it had it: never the value itself. */
export interface MaskValue {
  p: string;
  k: string;
  b: string;
  h?: string;
  l: number;
  n: number;
  a?: "warn" | "block";
  /** Where in the request: `messages[0].content`. */
  w?: string[];
}

/** One request's masking (`pansy_core::masking::Record`). */
export interface MaskRecord {
  key: number;
  t: string;
  agent?: string;
  provider?: string;
  model?: string;
  n: number;
  /** The request's size in bytes. */
  size?: number;
  values: MaskValue[];
  blocked?: boolean;
  took?: number;
  /** In memory only: by placeholder, the places the answer had it put back. */
  back?: Record<string, number>;
}

/** The days' masking: what the Masking page draws. */
export interface MaskSummary {
  days: number;
  values: number;
  distinct: number;
  requests: number;
  byDay: Record<string, number>;
  byAgent: Record<string, number>;
  byProvider: Record<string, number>;
  byKind: Record<string, number>;
  records: MaskRecord[];
}

/** A text masking changed in a request, before and after: in memory only, for the latest requests. */
export interface MaskChange {
  path: string;
  before: string;
  after: string;
}

/** An account routing lined up for a request (`pansy_gateway::trace::Slot`). */
export interface RouteSlot {
  provider: string;
  /** The key's label or end (`…a1b2`); none for a provider that takes no key. */
  account?: string;
  model: string;
  /** Seconds it had left to rest when the request came. */
  resting: number;
  /** Requests its model had answered. */
  used: number;
  /** Past the most tries a request makes: lined up, never tried. */
  spare: boolean;
  /** Its model's context window in tokens; 0 when not known. */
  window?: number;
}

/** A part of a request's prompt (`pansy_gateway::prompt::PromptPart`). */
export type PromptKind = "system" | "tools" | "skills" | "memory" | "files" | "results" | "chat";

/** What a part holds, by name: a tool, a file, a turn (`tag` `turn`, its number), `n` earlier turns (`turns`) or the `n` smallest left (`more`). */
export interface PromptItem {
  name: string;
  tag?: string;
  tokens: number;
  /** How many it stands for: a tool's results, reminders, images. */
  n?: number;
}

/** What a request's prompt held, part by part, estimated: names and sizes, never the text (`pansy_gateway::prompt::Prompt`). */
export interface Prompt {
  tokens: number;
  parts: { kind: PromptKind; tokens: number; items: PromptItem[] }[];
  /** The user's prompts in the conversation. */
  turns?: number;
  /** The conversation it is of, by a hash of its first prompt. */
  conv?: string;
}

/** One try of a request: an account asked, and what it answered (µs). */
export interface RouteTry {
  slot: number;
  atUs: number;
  /** 0 while it waits. */
  status: number;
  error?: string;
  headUs: number;
  us: number;
  /** Seconds it rests after failing. */
  rest: number;
  /** Its connection broke the first time, and it was asked once more. */
  again?: boolean;
}

/** One request as the Routing page plays it (`pansy_gateway::trace::Route`). */
export interface Route {
  id: number;
  at: string;
  agent: string;
  requested: string;
  group?: { id: string; name: string; routing: Routing; members: string[] };
  /** A subagent's request, as its agent says. */
  subagent?: boolean;
  /** The model the subagent asked for, when the gateway put it on the one set for the agent's subagents (`requested`). */
  asked?: string;
  /** Why it stayed on the model it asked for, though one is set for the agent's subagents. */
  kept?: string;
  lineup: RouteSlot[];
  tries: RouteTry[];
  maskingUs: number;
  done: boolean;
  status: number;
  error?: string;
  us: number;
  firstUs: number;
  input: number;
  output: number;
  cacheRead: number;
  /** What its prompt held; none until it was read, and for a day's requests. */
  prompt?: Prompt;
}

/** How an account or model fared today: Routing's Accounts and keys. */
export interface Tally {
  provider: string;
  account?: string;
  model: string;
  tried: number;
  answered: number;
  failed: number;
  last?: string;
}

export interface GatewayStatus {
  running: boolean;
  own: boolean;
  url: string;
  port: number;
  error: string | null;
  resting: { key: string; secs: number }[];
}

/** What a session spent (`pansy_agents::sessions::Tokens`). */
export interface SessionTokens {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  /** Inside output. */
  reasoning: number;
}

/** A conversation a session's agent started inside it: a Claude Code subagent, a thread Codex spawned. */
export interface Subagent {
  id: string;
  title: string;
  /** The kind its agent named it (Claude Code's `Explore`), if any. */
  kind: string;
  last: string | null;
  messages: number;
  tokens: SessionTokens;
}

/** One session of an agent (`pansy_agents::sessions::Session`). */
export interface Session {
  agent: string;
  id: string;
  /** The folder it ran in; empty when its agent kept none. */
  cwd: string;
  title: string;
  start: string | null;
  last: string | null;
  models: string[];
  /** The prompts and the replies. */
  messages: number;
  /** What it and its subagents spent. */
  tokens: SessionTokens;
  /** Its files' bytes. */
  size: number;
  path: string;
  /** The shell line that picks it up again in its folder. */
  resume: string;
  /** Written to in the last minute: its agent may be running it, and it is not deleted. */
  active: boolean;
  /** Delete can move it into the trash: its files are its own (OpenCode's are rows of a database it keeps open). */
  deletable: boolean;
  subagents: Subagent[];
}

/** One turn of a conversation: a prompt, a reply, what the model thought, a tool the agent called (its name, what it was given in a line, and whole with what it gave back), or where it was compacted. */
export interface Turn {
  role: "user" | "assistant" | "reasoning" | "tool" | "compact";
  name?: string;
  text: string;
  /** A tool's: what it was given, whole (JSON laid out, or a patch). */
  input?: string;
  /** A tool's: what it gave back. */
  output?: string;
  /** A tool's: how it ended; none when no result was kept. */
  status?: "done" | "error";
  at: string | null;
}

/** A session in pansy's trash (`pansy_agents::sessions::trash::Trashed`). */
export interface Trashed {
  /** Its folder in the trash: `<agent>/<name>`. */
  key: string;
  agent: string;
  id: string;
  title: string;
  cwd: string;
  last: string | null;
  deleted: string;
  size: number;
}

/** What the Sessions page lists: the user's home (folders under it are shown from ~), the sessions, and how many are in the trash. */
export interface SessionList {
  home: string;
  sessions: Session[];
  trash: number;
}

export type Page = "agents" | "sessions" | "providers" | "routing" | "groups" | "gateway" | "plugins" | "usage" | "masking" | "settings";

/** The pages the sidebar lists, which Settings › General can hide. */
export const SIDEBAR_PAGES: Page[] = ["agents", "sessions", "providers", "routing", "groups", "gateway", "plugins", "usage", "masking"];

/** Settings' own pages, under Settings in the sidebar while it is open. */
export type SettingsTab = "general" | "network" | "privacy" | "diagnostics" | "about";
export const settingsTab = signal<SettingsTab>("general");
/** What "Find a setting" holds: while it holds anything, every tab's matching settings show. */
export const settingsQuery = signal("");

export const page = signal<Page>("agents");
/** Whether the sidebar shows (the title bar's button): hidden, the pages are tabs in the title bar. */
export const sideShown = signal(Prefs.sidebar());
export const providers = signal<Provider[]>([]);
export const presets = signal<Preset[]>(JSON.parse(Providers.presets()) as Preset[]);
export const agents = signal<Agent[]>([]);
/** The agents have been looked for once: until then their page shows where rows will be. */
export const agentsRead = signal(false);
/** Each agent's program, asked after its rows: its version can take a while, the one that is out longer. */
export const clis = signal<Record<string, Cli>>({});
export const choices = signal<Choice[]>([]);
export const groups = signal<Group[]>([]);
export const profiles = signal<Profile[]>([]);
export const usage = signal<UsageSummary | null>(null);
export const usageDays = signal(7);
export const settings = signal<SettingsView | null>(null);
export const gateway = signal<GatewayStatus | null>(null);
export const masking = signal<MaskSummary | null>(null);
/** The requests the gateway routed lately, oldest first: Routing's Live. */
export const routes = signal<Route[]>([]);
/** The last week's days that had requests, newest first. */
export const routeDays = signal<{ day: string; requests: number }[]>([]);
/** The day Routing shows (`2026-11-12`), or null for Live; and its requests. */
export const routeDay = signal<string | null>(null);
export const dayRoutes = signal<Route[]>([]);
/** The request picked in Routing's list, or null for the latest. */
export const routePicked = signal<number | null>(null);
/** Today's tries by account and model. */
export const tallies = signal<Tally[]>([]);
/** Routing shows accounts by their place (key 1) instead of their names. */
export const hideAccounts = signal(false);
/** Routing's context window folded to its head. */
export const contextShut = signal(false);
/** How it draws the window: the map of tiles, or magpie's grid of cells. */
export const contextLook = signal<"map" | "grid">("map");
export const maskDays = signal(7);
/** The request the Masking page shows on its own, by its record's key. */
export const maskOpen = signal<number | null>(null);
/** The agents' sessions; null until they have been read once. */
export const sessions = signal<SessionList | null>(null);
/** The conversation the Sessions page shows on its own (a session's, or its subagent `sub`'s), or null for the list. */
export const sessionOpen = signal<{ agent: string; id: string; sub: string; title: string } | null>(null);
/** What is being done, by a key of the caller's (`connect:codex`). */
export const busy = signal<Record<string, boolean>>({});

export interface Toast {
  id: number;
  message: string;
  tone: "default" | "error";
}
export const toasts = signal<Toast[]>([]);
let nextToast = 1;

/** Shows `message` for four seconds. */
export function toast(message: string, tone: Toast["tone"] = "default") {
  const id = nextToast++;
  toasts.update((all) => [...all.slice(-2), { id, message, tone }]);
  setTimeout(() => toasts.update((all) => all.filter((t) => t.id !== id)), 4000);
}

/** A rejection's message: native calls reject with an Error or a string. */
export function reason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A question the user answers before something is done. */
export interface Question {
  title: string;
  description: string;
  confirm: string;
  danger?: boolean;
  answer: (yes: boolean) => void;
}
export const question = signal<Question | null>(null);

export function ask(q: Omit<Question, "answer">): Promise<boolean> {
  return new Promise((resolve) => {
    question.set({
      ...q,
      answer: (yes) => {
        question.set(null);
        resolve(yes);
      },
    });
  });
}

/** Runs `work` with `key` marked busy; a failure is a toast. True when it went through. */
export async function act(key: string, work: () => Promise<unknown>, done?: string): Promise<boolean> {
  busy.update((b) => ({ ...b, [key]: true }));
  try {
    await work();
    if (done) toast(done);
    return true;
  } catch (error) {
    toast(reason(error), "error");
    return false;
  } finally {
    busy.update((b) => {
      const next = { ...b };
      delete next[key];
      return next;
    });
  }
}

export const isBusy = (key: string) => busy()[key] === true;

/** The subscriptions' allowances, as last read; null before the first reading. */
export const allowances = signal<Allowance[] | null>(null);

/** Reads the allowances: kept a minute, or asked again (`fresh`). */
export async function loadAllowances(fresh = false) {
  try {
    allowances.set(JSON.parse(await Subscriptions.allowances(fresh)) as Allowance[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** The API keys' balances, as last read; null before the first reading. */
export const balances = signal<Balance[] | null>(null);

/** Reads the keys' balances: kept a minute, or asked again (`fresh`). */
export async function loadBalances(fresh = false) {
  try {
    balances.set(JSON.parse(await Subscriptions.balances(fresh)) as Balance[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** A provider another app has set up, as the import shows it (`pansy-agents`' `import`). */
export interface ImportItem {
  ref: string;
  /** Where in the app: Claude Code, Codex, settings.json… */
  from: string;
  name: string;
  icon: string;
  host: string;
  /** The key, masked; how many there are. */
  key: string;
  keys: number;
  models: number;
  /** New, already here (`same`), or at an id in use here (`taken`). */
  status: "new" | "same" | "taken";
  existing: string;
  /** A provider here at its address, which it can join as another key. */
  keyOf: string;
  reserved: boolean;
  /** Why it isn't picked at first; why it can't be imported. */
  off: string;
  skip: string;
  picked: boolean;
  fingerprint: string;
}

/** One app pansy imports providers from. */
export interface ImportSource {
  id: string;
  name: string;
  path: string;
  found: boolean;
  error: string | null;
  items: ImportItem[];
}

/** The other apps' providers the Providers page offers to import, unless ignored. */
export const importFound = signal<{ fingerprint: string; source: string }[]>([]);

export async function loadImportFound() {
  try {
    importFound.set(JSON.parse(await Providers.importFound()));
  } catch {
    importFound.set([]); // a look of its own: never in the way of adding by hand
  }
}

export async function loadProviders() {
  try {
    const [list, picks] = await Promise.all([Providers.list(), Providers.choices()]);
    batch(() => {
      providers.set(JSON.parse(list) as Provider[]);
      choices.set(JSON.parse(picks) as Choice[]);
    });
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadAgents() {
  try {
    agents.set(JSON.parse(await Agents.list()) as Agent[]);
  } catch (error) {
    toast(reason(error), "error");
  }
  agentsRead.set(true);
  void loadVersions(); // after the rows, never holding them up
}

let askingVersions = false;
/** The agents' programs; some still being asked are asked again in a moment, five times at most. */
export async function loadVersions(again = 0) {
  if (askingVersions) return;
  askingVersions = true;
  try {
    const said = JSON.parse(await Agents.versions()) as { clis: Record<string, Cli>; pending: boolean };
    clis.set(said.clis);
    if (said.pending && again < 5) setTimeout(() => void loadVersions(again + 1), 3000);
  } catch {
    // a version just isn't shown
  } finally {
    askingVersions = false;
  }
}

export async function loadGroups() {
  try {
    const [list, picks] = await Promise.all([Groups.list(), Providers.choices()]);
    batch(() => {
      groups.set(JSON.parse(list) as Group[]);
      choices.set(JSON.parse(picks) as Choice[]);
    });
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadProfiles() {
  try {
    profiles.set(JSON.parse(await Profiles.list()) as Profile[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadUsage() {
  try {
    usage.set(JSON.parse(await Usage.summary(usageDays())) as UsageSummary);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** Today's totals, as the tray icon's tooltip and the quick panel's Routing line say them; null before the first read. */
export const usageToday = signal<Totals | null>(null);

export async function loadUsageToday() {
  try {
    usageToday.set((JSON.parse(await Usage.summary(1)) as UsageSummary).total);
  } catch {
    // said again at the next read
  }
}

export async function loadSettings() {
  try {
    settings.set(JSON.parse(await Settings.get()) as SettingsView);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/**
 * Saves one setting (a list or an object as JSON) and reads them all again;
 * with `restart`, restarts the gateway for it (the port, sharing).
 */
export function setSetting<K extends keyof SettingsView>(key: K, value: SettingsView[K], restart = false): Promise<boolean> {
  const text = typeof value === "object" ? JSON.stringify(value) : String(value);
  return act(`setting:${key}`, async () => {
    await Settings.set(key, text);
    if (restart) {
      await Gateway.restart();
      toast(tr("The gateway restarted"));
    }
  }).then(async (ok) => {
    await loadSettings();
    if (restart) await loadGateway();
    return ok;
  });
}

/** Shows or hides the sidebar, and keeps it so for the next launch. */
export function setSideShown(on: boolean) {
  sideShown.set(on);
  try {
    Prefs.setSidebar(on);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** Saves masking with `change` made to it. */
export function setMasking(change: Partial<MaskOptions>): Promise<boolean> {
  const current = settings()?.masking;
  if (!current) return Promise.resolve(false);
  return setSetting("masking", { ...current, ...change });
}

/** Whether anything is masked: secrets, personal data or the user's words. */
export function maskingOn(): boolean {
  const m = settings()?.masking;
  return !!m && (m.secrets || m.personal || m.words.some((w) => w.trim().length >= 2));
}

export async function loadMasking() {
  try {
    masking.set(JSON.parse(await Masking.summary(maskDays())) as MaskSummary);
  } catch (error) {
    toast(reason(error), "error");
  }
}

export async function loadSessions() {
  try {
    sessions.set(JSON.parse(await Sessions.list()) as SessionList);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** The trace's version Routing has: 0 before it has any. */
let traceVersion = 0;

/** Reads the trace when it changed; says whether it did. Routing and the quick panel's Routing tab poll it. */
export function loadTrace(): boolean {
  const got = JSON.parse(Gateway.trace(traceVersion)) as { version: number; routes?: Route[] };
  if (!got.routes) return false;
  traceVersion = got.version;
  routes.set(got.routes);
  return true;
}

async function loadTally() {
  try {
    const [days, tally] = await Promise.all([Usage.days(), Usage.tally()]);
    batch(() => {
      routeDays.set(JSON.parse(days) as { day: string; requests: number }[]);
      tallies.set(JSON.parse(tally) as Tally[]);
    });
  } catch (error) {
    toast(reason(error), "error");
  }
}

async function readDay(day: string) {
  try {
    const got = JSON.parse(await Usage.day(day)) as Route[];
    if (routeDay() === day) dayRoutes.set(got);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** Shows `day`'s requests in Routing (`2026-11-12`), or Live's with null. */
export function showDay(day: string | null) {
  batch(() => {
    if (day !== routeDay()) dayRoutes.set([]);
    routeDay.set(day);
    routePicked.set(null);
  });
  if (day !== null) void readDay(day);
}

export function loadRouting() {
  loadTrace();
  const day = routeDay();
  return Promise.all([loadTally(), day !== null ? readDay(day) : undefined]);
}

export async function loadGateway() {
  try {
    gateway.set(JSON.parse(await Gateway.status()) as GatewayStatus);
  } catch {
    gateway.set(null);
  }
}

/** Reloads what `p` shows; settles once all of it has (the title bar's refresh turns until then). */
export function loadPage(p: Page): Promise<unknown> {
  switch (p) {
    case "agents":
      return Promise.all([loadAgents(), loadProviders(), loadGroups(), loadProfiles(), loadSettings()]);
    case "sessions":
      return Promise.all([loadSessions(), loadAgents()]);
    case "providers":
      return Promise.all([loadProviders(), loadAgents(), loadImportFound()]);
    case "routing":
      return Promise.all([loadRouting(), loadGroups(), loadProviders(), loadAgents(), loadGateway()]);
    case "groups":
      return Promise.all([loadGroups(), loadProviders(), loadSettings(), loadRoutings()]);
    case "gateway":
      return Promise.all([loadGateway(), loadSettings(), loadGroups(), loadProviders(), loadAgents()]);
    case "plugins":
      return Promise.all([loadPlugins(), loadProviders(), loadSettings(), market() ? null : loadMarket()]);
    case "usage":
      return Promise.all([loadUsage(), loadAgents(), loadProviders()]);
    case "masking":
      return Promise.all([loadSettings(), loadMasking()]);
    case "settings":
      return Promise.all([loadSettings(), loadGateway()]);
  }
}

export function go(p: Page) {
  page.set(p);
  void loadPage(p);
}

/**
 * Settings › Privacy's masking alert: a kind of value masked for the first
 * time, its name and how many places, never the value.
 */
function alertFirsts() {
  const firsts = JSON.parse(Masking.firsts()) as [string, number][];
  if (firsts.length === 0 || settings()?.maskAlert === false) return;
  const body = firsts.map(([kind, n]) => tr("%s: %d", kindLabel(kind), n)).join(" · ");
  toast(tr("Masked for the first time: %s", body));
  void Notifications.show({ id: "masking", title: tr("Masked for the first time"), body: tr("%s. The vendor got placeholders, never the values.", body), silent: true }).catch(() => {});
}

/** Starts the window: the first page, the settings (the theme), and the gateway's status every few seconds. */
export function start() {
  void loadPage(page());
  void loadProviders();
  void loadSettings();
  void loadGateway();
  // the tray's tooltip, read again every three minutes as magpie's is
  void loadUsageToday();
  setInterval(() => void loadUsageToday(), 180_000);
  setInterval(() => {
    void loadGateway();
    alertFirsts();
    // The Masking page is live: a request masked shows in a moment.
    if (page() === "masking") void loadMasking();
  }, 5000);
  // Routing plays the requests as the gateway routes them.
  setInterval(() => {
    if (page() === "routing" && loadTrace()) void loadTally();
  }, 700);
}

/** A choice's label: "DeepSeek · deepseek-chat", a group's name. */
export function choiceLabel(id: string | null): string {
  if (!id) return tr("Not set");
  const choice = choices().find((c) => c.id === id);
  if (!choice) return id;
  return choice.group ? tr("Group: %s", choice.name) : `${choice.provider} · ${choice.name}`;
}

/** A choice's own name: "deepseek-chat", a group's name; the id when it is unknown. */
export function choiceName(id: string | null): string {
  if (!id) return "";
  return choices().find((c) => c.id === id)?.name ?? id;
}

/** "128K" for a model's context window, when its provider said. */
export function contextOf(id: string): string | null {
  const p = providerOf(id);
  const model = p?.models.find((m) => `${p.id}/${m.id}` === id);
  const n = model?.context;
  if (!n) return null;
  return n >= 1e6 ? `${(n / 1048576).toFixed(1)}M` : `${Math.round(n / 1024)}K`;
}

/** The provider a model id is served by (`deepseek/deepseek-chat` → DeepSeek's). */
export function providerOf(id: string | null): Provider | undefined {
  if (!id) return undefined;
  const slash = id.indexOf("/");
  const pid = slash < 0 ? id : id.slice(0, slash);
  return providers().find((p) => p.id === pid);
}

/** A group's members' providers, each once. */
export function groupProviders(group: Group): Provider[] {
  const seen: Provider[] = [];
  for (const m of group.members) {
    const p = providerOf(m);
    if (p && !seen.includes(p)) seen.push(p);
  }
  return seen;
}

/** A provider's host: the first endpoint's (`api.deepseek.com`). */
export function hostOf(url: string | undefined): string {
  if (!url) return "";
  const rest = url.includes("://") ? url.slice(url.indexOf("://") + 3) : url;
  return rest.split("/")[0] ?? rest;
}

/** 12_345 → "12K", 4_612_000 → "4.6M", as magpie counts. */
/** A count as magpie's tray and quick panel say it: 1.33B, 355M, 1.2M, 456K, 8.2K, 950. */
export function fmtN(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (n >= 1e7) return `${Math.round(n / 1e6)}M`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e5) return `${Math.round(n / 1e3)}K`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(n);
}

export function compact(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e4) return `${Math.round(n / 1e3)}K`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(n);
}

/** What a kind of masked value is called: `API_KEY` → "API keys". A kind of the user's own rules is its name. */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "API_KEY":
      return tr("API keys");
    case "PRIVATE_KEY":
      return tr("Private keys");
    case "TOKEN":
      return tr("Tokens");
    case "PASSWORD":
      return tr("Passwords");
    case "SECRET":
      return tr("Secrets");
    case "SEED_PHRASE":
      return tr("Seed phrases");
    case "EMAIL":
      return tr("Emails");
    case "PHONE":
      return tr("Phone numbers");
    case "ID_CARD":
      return tr("ID numbers");
    case "BANK_CARD":
      return tr("Bank card numbers");
    case "SSN":
      return tr("Social security numbers");
    case "PASSPORT":
      return tr("Passport numbers");
    case "IBAN":
      return tr("Bank account numbers (IBAN)");
    case "BIRTHDAY":
      return tr("Birthdays");
    case "MAC":
      return tr("MAC addresses");
    case "SERIAL":
      return tr("Serial numbers");
    case "USERNAME":
      return tr("User names");
    case "HOSTNAME":
      return tr("Computer names");
    case "IP":
      return tr("IP addresses");
    case "BUCKET":
      return tr("Storage buckets");
    case "TERM":
      return tr("Your words");
    case "CUSTOM":
      return tr("Your rules");
  }
  return kind.endsWith("_KEY") || kind.endsWith("_KEYPAIR") ? tr("Wallet keys") : kind;
}

/** The kinds of secrets the rules find, as Settings › Privacy lists them. */
export const SECRET_KINDS = ["API_KEY", "PRIVATE_KEY", "TOKEN", "PASSWORD", "SECRET"];
/** The personal data always masked with Mask personal data. */
export const PERSONAL_KINDS = ["EMAIL", "PHONE", "ID_CARD", "BANK_CARD"];
/** The categories of personal data turned on or off on their own (`pansy_mask::CATEGORIES`), and whether each is on until the user chooses. */
export const MASK_CATEGORIES: { id: string; kinds: string[]; on: boolean }[] = [
  { id: "ssn", kinds: ["SSN"], on: true },
  { id: "passport", kinds: ["PASSPORT"], on: true },
  { id: "iban", kinds: ["IBAN"], on: true },
  { id: "birthday", kinds: ["BIRTHDAY"], on: true },
  { id: "mac", kinds: ["MAC"], on: true },
  { id: "serial", kinds: ["SERIAL"], on: true },
  { id: "home", kinds: ["USERNAME"], on: false },
  { id: "userhost", kinds: ["USERNAME", "HOSTNAME"], on: false },
  { id: "ip", kinds: ["IP"], on: false },
  { id: "bucket", kinds: ["BUCKET"], on: false },
];

/** A category of personal data's name and what it masks. */
export function categoryLabel(id: string): { title: string; description: string } {
  switch (id) {
    case "ssn":
      return { title: tr("Social security numbers"), description: tr("US social security numbers") };
    case "passport":
      return { title: tr("Passport numbers"), description: tr("A passport number beside the word passport") };
    case "iban":
      return { title: tr("Bank account numbers (IBAN)"), description: tr("International bank account numbers whose check digits add up") };
    case "birthday":
      return { title: tr("Birthdays"), description: tr("A date beside born, birthday or date of birth") };
    case "mac":
      return { title: tr("MAC addresses"), description: tr("A network card's hardware address") };
    case "serial":
      return { title: tr("Serial numbers"), description: tr("A device's serial number beside the words serial number") };
    case "home":
      return { title: tr("Your user name in paths"), description: tr("/Users/you and /home/you: in nearly every request, as agents send the folder they work in") };
    case "userhost":
      return { title: tr("user@host"), description: tr("A user name and a computer's name, as a shell prompt has them") };
    case "ip":
      return { title: tr("IP addresses"), description: tr("Public IPv4 and IPv6 addresses; this computer's own and private ones go as written") };
    case "bucket":
      return { title: tr("Storage buckets"), description: tr("S3, Google Cloud Storage and Azure buckets in URLs") };
  }
  return { title: id, description: "" };
}
