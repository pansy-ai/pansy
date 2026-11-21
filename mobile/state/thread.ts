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
  | { kind: "reasoning"; id: string; text: string; seconds?: number }
  | { kind: "tool"; id: string; tool: string; title: string; status: ToolStatus; input?: string; output?: string; exit?: number }
  | { kind: "subagent"; id: string; name: string; detail: string; status: ToolStatus }
  | { kind: "notice"; id: string; text: string; tone: "danger" | "muted" }
  | { kind: "compact"; id: string }
  /** A turn ended here, seen live: how long it took, whether the user stopped it. */
  | { kind: "end"; id: string; ms?: number; stopped?: boolean };

export type Approval = { request: string; kind: string; title: string; detail: string; reason?: string; questions?: Question[] };

const SUBAGENT_KINDS = new Set(["subagent", "task", "agent"]);

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
        return { kind: "reasoning", id, text: t.text };
      case "compact":
        return { kind: "compact", id };
      default:
        if (t.name && SUBAGENT_KINDS.has(t.name.toLowerCase())) return { kind: "subagent", id, name: t.text || t.name, detail: t.input ?? "", status: toolStatus(t.status) };
        return { kind: "tool", id, tool: t.name ?? "tool", title: t.text || t.name || "", status: toolStatus(t.status), input: t.input, output: t.output };
    }
  });
}

export type FeedState = { items: Item[]; approvals: Approval[]; running: boolean; startedAt: number | null };

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
      if (event.kind === "reasoning") return { ...state, items: [...items, { kind: "reasoning", id: event.item, text: "" }] };
      if (SUBAGENT_KINDS.has(event.kind)) {
        const [name, ...rest] = event.title.split(" · ");
        return { ...state, items: [...items, { kind: "subagent", id: event.item, name, detail: rest.join(" · "), status: "running" }] };
      }
      return { ...state, items: [...items, { kind: "tool", id: event.item, tool: event.kind, title: event.title, status: "running" }] };
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
              ? { ...i, status: toolStatus(event.status) }
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
      const done: Item[] = items.map((i) => (i.kind === "assistant" && i.streaming ? { ...i, streaming: false } : (i.kind === "tool" || i.kind === "subagent") && i.status === "running" ? { ...i, status: "done" as ToolStatus } : i));
      const failed = /fail|error/i.test(event.status);
      const stopped = /interrupt|cancel/i.test(event.status);
      const notice: Item[] = failed ? [{ kind: "notice", id: `n${now}`, text: event.error ?? event.status, tone: "danger" }] : [];
      const end: Item = { kind: "end", id: `e${now}`, ms: state.startedAt ? now - state.startedAt : undefined, stopped };
      return { ...state, items: [...done, ...notice, end], running: false, startedAt: null };
    }
    case "exited":
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
