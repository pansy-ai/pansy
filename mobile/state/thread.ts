// One open thread: its history (thread.subscribe's `history`) and its live
// events folded into the feed's items, the approvals waiting, whether a
// turn runs; subscribed again with `after` (the last seq) on reconnect.
import { signal } from "nsh";
import type { Connection } from "../net/client";
import type { Event, Question, Seq, Subscribed, ThreadSummary, Turn } from "../net/types";
import { conn, putSummary } from "./store";

export type ToolStatus = "running" | "done" | "failed" | "declined" | "waiting";

export type Item =
  | { kind: "user"; id: string; text: string; at?: string; queued?: boolean }
  | { kind: "assistant"; id: string; text: string; at?: string; streaming?: boolean }
  | { kind: "reasoning"; id: string; text: string; at?: string }
  | { kind: "tool"; id: string; tool: string; title: string; status: ToolStatus; input?: string; output?: string; exit?: number; at?: string }
  /** A subagent the agent started: what it was told, its type or agent and model, what it reported back. */
  | { kind: "subagent"; id: string; name: string; detail: string; status: ToolStatus; output?: string }
  /** What went wrong: the agent's words, or (`exited`) that the agent quit while it worked. */
  | { kind: "notice"; id: string; text: string; tone: "danger" | "muted"; exited?: { code?: number } }
  | { kind: "compact"; id: string }
  /** A turn ended here, live or as the agent's file says: how long it took (where known), whether the user stopped it. */
  | { kind: "end"; id: string; ms?: number; stopped?: boolean; at?: string };

export type Approval = { request: string; kind: string; title: string; detail: string; reason?: string; questions?: Question[] };

/** A subagent's call: pansy's own word live, the agents' own in their files (Claude Code's Task and Agent, Codex's spawn_agent). */
const SUBAGENT_KINDS = new Set(["subagent", "task", "agent", "spawn_agent", "spawnagent"]);
const isSubagent = (kind: string | undefined) => !!kind && SUBAGENT_KINDS.has(kind.toLowerCase());

/** A subagent's title, `name · detail`, as its row shows it. */
function subagentOf(id: string, title: string, status: ToolStatus): Extract<Item, { kind: "subagent" }> {
  const [name, ...rest] = title.split(" · ");
  return { kind: "subagent", id, name, detail: rest.join(" · "), status };
}

/** A tool's status as the protocol says it, as the feed shows it. */
function toolStatus(s: string | undefined): ToolStatus {
  if (!s) return "done";
  if (/fail|error/i.test(s)) return "failed";
  if (/declin|denied|reject/i.test(s)) return "declined";
  if (/run|progress|pending/i.test(s)) return "running";
  return "done";
}

/** The history's turns as the feed's items. */
export function fromHistory(turns: Turn[]): Item[] {
  return turns.map((t, i): Item => {
    const id = `h${i}`;
    switch (t.role) {
      case "user":
        return { kind: "user", id, text: t.text, at: t.at };
      case "assistant":
        return { kind: "assistant", id, text: t.text, at: t.at };
      case "reasoning":
        return { kind: "reasoning", id, text: t.text, at: t.at };
      case "compact":
        return { kind: "compact", id };
      case "end":
        return { kind: "end", id, ms: t.ms, stopped: t.status === "stopped", at: t.at };
      default:
        if (isSubagent(t.name)) return { kind: "subagent", id, name: t.text || t.name!, detail: t.input ?? "", status: toolStatus(t.status), output: t.output };
        return { kind: "tool", id, tool: t.name ?? "tool", title: t.text || t.name || "", status: toolStatus(t.status), input: t.input, output: t.output, at: t.at };
    }
  });
}

export type FeedState = { items: Item[]; approvals: Approval[]; running: boolean; startedAt: number | null };

/** The turn going on, ended: the reply done streaming, the calls still running as `running`, then `notice` and the end. */
function close(state: FeedState, running: ToolStatus, notice: Item[], stopped: boolean, now: number): FeedState {
  const done: Item[] = state.items.map((i) => (i.kind === "assistant" && i.streaming ? { ...i, streaming: false } : (i.kind === "tool" || i.kind === "subagent") && i.status === "running" ? { ...i, status: running } : i));
  const end: Item = { kind: "end", id: `e${now}`, ms: state.startedAt ? now - state.startedAt : undefined, stopped, at: new Date(now).toISOString() };
  return { ...state, items: [...done, ...notice, end], running: false, startedAt: null, approvals: [] };
}

/** `state` after `event`: pure, so the tests and the view fold the same way. */
export function apply(state: FeedState, event: Event, now = Date.now()): FeedState {
  const items = state.items;
  const replace = (id: string, f: (i: Item) => Item) => items.map((i) => (i.id === id ? f(i) : i));
  switch (event.type) {
    case "turnStarted":
      return { ...state, running: true, startedAt: state.running && state.startedAt ? state.startedAt : now };
    case "delta": {
      const at = items.find((i) => i.id === event.item);
      if (at && at.kind === "assistant") return { ...state, items: replace(event.item, (i) => ({ ...(i as Extract<Item, { kind: "assistant" }>), text: (i as { text: string }).text + event.text, streaming: true })) };
      return { ...state, items: [...items, { kind: "assistant", id: event.item, text: event.text, streaming: true, at: new Date(now).toISOString() }] };
    }
    case "message": {
      const at = items.find((i) => i.id === event.item);
      if (at) return { ...state, items: replace(event.item, () => ({ kind: "assistant", id: event.item, text: event.text, at: (at as { at?: string }).at })) };
      return { ...state, items: [...items, { kind: "assistant", id: event.item, text: event.text, at: new Date(now).toISOString() }] };
    }
    case "user": {
      // the line this phone sent (shown already) or another device's
      const mine = items.findIndex((i) => i.kind === "user" && i.id.startsWith("local") && i.text === event.text);
      if (mine >= 0) return { ...state, items: items.map((i, n) => (n === mine ? { ...(i as Extract<Item, { kind: "user" }>), id: `u${now}${n}`, queued: false } : i)) };
      return { ...state, items: [...items, { kind: "user", id: `u${now}${items.length}`, text: event.text, at: new Date(now).toISOString() }] };
    }
    case "toolStarted": {
      // Codex's reasoning comes as an item of its own
      if (event.kind === "reasoning") return { ...state, items: [...items, { kind: "reasoning", id: event.item, text: "", at: new Date(now).toISOString() }] };
      if (isSubagent(event.kind)) return { ...state, items: [...items, subagentOf(event.item, event.title, "running")] };
      return { ...state, items: [...items, { kind: "tool", id: event.item, tool: event.kind, title: event.title, status: "running", at: new Date(now).toISOString() }] };
    }
    case "toolFinished": {
      // finished before this phone saw it start (it subscribed in between)
      if (!items.some((i) => i.id === event.item)) return apply(apply(state, { type: "toolStarted", item: event.item, kind: event.kind, title: event.title }, now), event, now);
      return {
        ...state,
        items: replace(event.item, (i) =>
          i.kind === "reasoning"
            ? { ...i, text: event.output || i.text }
            : i.kind === "subagent"
              ? { ...subagentOf(i.id, event.title || `${i.name} · ${i.detail}`, toolStatus(event.status)), output: event.output || i.output }
              : { ...(i as Extract<Item, { kind: "tool" }>), title: event.title || (i as { title: string }).title, status: toolStatus(event.status), output: event.output, exit: event.exit }),
      };
    }
    case "approval":
      if (state.approvals.some((a) => a.request === event.request)) return state;
      return { ...state, approvals: [...state.approvals, { request: event.request, kind: event.kind, title: event.title, detail: event.detail, reason: event.reason }] };
    case "question":
      if (state.approvals.some((a) => a.request === event.request)) return state;
      return { ...state, approvals: [...state.approvals, { request: event.request, kind: "question", title: event.questions[0]?.header ?? "", detail: "", questions: event.questions }] };
    case "resolved":
      return { ...state, approvals: state.approvals.filter((a) => a.request !== event.request) };
    case "turnFinished": {
      const failed = /fail|error/i.test(event.status);
      const stopped = /interrupt|cancel/i.test(event.status);
      const notice: Item[] = failed ? [{ kind: "notice", id: `n${now}`, text: event.error ?? event.status, tone: "danger" }] : [];
      return { ...close(state, "done", notice, stopped, now), approvals: state.approvals };
    }
    case "exited":
      // quit while it worked: what was running never finishes, so the turn ends here, saying so
      if (state.running) return close(state, "failed", [{ kind: "notice", id: `n${now}`, text: "", tone: "danger", exited: { code: event.code } }], false, now);
      return { ...state, running: false, startedAt: null, approvals: [] };
    default:
      return state;
  }
}

export class ThreadView {
  readonly local = signal<ThreadSummary | null>(null);
  readonly items = signal<Item[]>([]);
  readonly approvals = signal<Approval[]>([]);
  readonly running = signal(false);
  readonly startedAt = signal<number | null>(null);
  readonly loading = signal(true);
  readonly error = signal<string | null>(null);
  private lastSeq = 0;
  private subscribed = false;
  private stops: (() => void)[] = [];

  constructor(
    readonly machineId: string,
    readonly threadId: string,
  ) {}

  get conn(): Connection | undefined {
    return conn(this.machineId);
  }

  private state(): FeedState {
    return { items: this.items(), approvals: this.approvals(), running: this.running(), startedAt: this.startedAt() };
  }

  private set(s: FeedState) {
    this.items.set(s.items);
    this.approvals.set(s.approvals);
    this.running.set(s.running);
    this.startedAt.set(s.startedAt);
  }

  private take(e: Seq<Event>) {
    if (e.seq <= this.lastSeq) return;
    this.lastSeq = e.seq;
    this.set(apply(this.state(), e.event));
  }

  open() {
    const c = this.conn;
    if (!c) {
      this.loading.set(false);
      this.error.set("offline");
      return;
    }
    this.stops.push(
      c.on("thread.event", (d: { threadId: string; seq: number; event: Event }) => {
        if (d.threadId === this.threadId) this.take({ seq: d.seq, event: d.event });
      }),
    );
    this.stops.push(c.onOpen(() => void this.subscribe()));
  }

  private async subscribe() {
    const c = this.conn;
    if (!c) return;
    const again = this.subscribed;
    try {
      const r = await c.call<Subscribed>("thread.subscribe", again ? { threadId: this.threadId, after: this.lastSeq } : { threadId: this.threadId });
      this.local.set(r.thread);
      if (!again) {
        const t = r.thread;
        this.set({ items: fromHistory(r.history), approvals: [], running: t.status === "working" || t.status === "approval", startedAt: t.startedAt ? Date.parse(t.startedAt) : null });
      }
      for (const e of r.live) this.take(e);
      this.lastSeq = Math.max(this.lastSeq, r.seq);
      this.subscribed = true;
      this.error.set(null);
    } catch (e) {
      this.error.set(String((e as Error)?.message ?? e));
    } finally {
      this.loading.set(false);
    }
  }

  close() {
    for (const s of this.stops) s();
    this.stops = [];
    void this.conn?.call("thread.unsubscribe", { threadId: this.threadId }).catch(() => {});
  }

  /** Shows a line the user sent at once; the `user` event confirms it. */
  echo(text: string, queued: boolean) {
    this.items.set([...this.items(), { kind: "user", id: `local${Date.now()}`, text, at: new Date().toISOString(), queued }]);
  }

  /** Takes a summary an answer gave (thread.update). */
  put(t: ThreadSummary) {
    this.local.set(t);
    putSummary(this.machineId, t);
  }
}
