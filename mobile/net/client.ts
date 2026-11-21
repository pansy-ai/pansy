// One machine's connection: the first of its routes that answers, its
// credentials (a DPoP token refreshed before it runs out, or the page's
// cookie), a WebSocket of JSON requests and events, and reconnecting with
// jittered backoff when it drops (phone-protocol.md "连接", "WebSocket").
import { signal } from "nsh";
import { proof, type DeviceKey } from "./dpop";
import { http, openSocket, type Socket } from "./transport";
import { RpcError, type Environment, type Route } from "./types";

export type Auth = { mode: "dpop"; key: DeviceKey; token: string; expiresAt: number } | { mode: "cookie" };

/** A paired machine, as the phone keeps it. */
export type Machine = {
  id: string;
  name: string;
  version?: string;
  os?: string;
  capabilities: string[];
  /** In the order they are tried; `ok` and `ms` are how the last try went. */
  routes: (Route & { ok?: boolean; ms?: number })[];
  deviceId: string;
  scopes: string[];
  approval: "approved" | "pending";
  code?: string;
  auth: Auth;
};

export type ConnState = "connecting" | "online" | "offline" | "pending" | "revoked";

const ENV_PATH = "/.well-known/pansy/environment";
const PING_GAP = 60_000;
const CALL_TIMEOUT = 30_000;

/** JSON over HTTP to `base`: with the DPoP token and a proof when `auth` is one. */
export async function request<T>(base: string, method: string, path: string, options: { body?: unknown; auth?: Auth; timeout?: number; dpopKey?: DeviceKey } = {}): Promise<T> {
  const url = base + path;
  const headers: Record<string, string> = { accept: "application/json" };
  if (options.body !== undefined) headers["content-type"] = "application/json";
  const auth = options.auth;
  if (auth?.mode === "dpop") {
    headers.authorization = `DPoP ${auth.token}`;
    headers.dpop = await proof(auth.key, method, url, auth.token);
  } else if (options.dpopKey) {
    headers.dpop = await proof(options.dpopKey, method, url);
  }
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), options.timeout ?? 10_000);
  let res: Response;
  try {
    res = await http(url, { method, headers, body: options.body === undefined ? undefined : JSON.stringify(options.body), signal: abort.signal });
  } catch (e) {
    throw new RpcError("network", String((e as Error)?.message ?? e));
  } finally {
    clearTimeout(timer);
  }
  const text = await res.text();
  let json: unknown = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch {
    json = null;
  }
  if (!res.ok) {
    const err = (json as { error?: { code?: string; message?: string } } | null)?.error;
    throw new RpcError(err?.code ?? (res.status === 401 ? "unauthorized" : res.status === 403 ? "forbidden" : `http_${res.status}`), err?.message ?? `HTTP ${res.status}`);
  }
  return json as T;
}

/** The machine's public description at `base`. */
export const environment = (base: string, timeout = 5000) => request<Environment>(base, "GET", ENV_PATH, { timeout });

const wsUrl = (base: string) => base.replace(/^http/, "ws") + "/ws";

type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void; timer: ReturnType<typeof setTimeout> };

export class Connection {
  readonly state = signal<ConnState>("connecting");
  /** The route in use, while online. */
  readonly route = signal<string | null>(null);
  readonly error = signal<string | null>(null);

  private ws: Socket | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private listeners = new Map<string, Set<(data: unknown) => void>>();
  private openListeners = new Set<() => void>();
  private attempt = 0;
  private retry: ReturnType<typeof setTimeout> | undefined;
  private watchdog: ReturnType<typeof setTimeout> | undefined;
  private refresher: ReturnType<typeof setTimeout> | undefined;
  private stopped = false;
  private connecting = false;

  constructor(
    private machine: () => Machine,
    private save: (m: Machine) => void,
  ) {}

  start() {
    this.stopped = false;
    void this.connect();
  }

  stop() {
    this.stopped = true;
    clearTimeout(this.retry);
    clearTimeout(this.watchdog);
    clearTimeout(this.refresher);
    this.ws?.close();
    this.ws = null;
    this.fail("offline");
  }

  /** Tries again now: the app came back, the user pulled to refresh. */
  wake() {
    if (this.stopped || this.connecting || this.state() === "online") return;
    clearTimeout(this.retry);
    this.attempt = 0;
    void this.connect();
  }

  /** A request over the socket; rejects with the home's error, or `offline`. */
  call<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    const ws = this.ws;
    if (!ws || this.state() !== "online") return Promise.reject(new RpcError("offline", "offline"));
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new RpcError("timeout", method));
      }, CALL_TIMEOUT);
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
      ws.send(JSON.stringify({ id, method, params }));
    });
  }

  /** Listens to a pushed event; returns what stops it. */
  on(event: string, fn: (data: never) => void): () => void {
    let set = this.listeners.get(event);
    if (!set) this.listeners.set(event, (set = new Set()));
    set.add(fn as (data: unknown) => void);
    return () => set!.delete(fn as (data: unknown) => void);
  }

  /** Runs `fn` each time the socket opens: subscribe again there. */
  onOpen(fn: () => void): () => void {
    this.openListeners.add(fn);
    if (this.state() === "online") fn();
    return () => this.openListeners.delete(fn);
  }

  /** HTTP to the route in use, with the device's credentials. */
  http<T>(method: string, path: string, body?: unknown): Promise<T> {
    const base = this.route();
    if (!base) return Promise.reject(new RpcError("offline", "offline"));
    return request<T>(base, method, path, { body, auth: this.machine().auth });
  }

  private fail(state: ConnState, error: string | null = null) {
    this.route.set(null);
    this.state.set(state);
    this.error.set(error);
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(new RpcError("offline", "offline"));
    }
    this.pending.clear();
  }

  private schedule(ms?: number) {
    if (this.stopped) return;
    clearTimeout(this.retry);
    const delay = ms ?? Math.min(30_000, 1000 * 2 ** this.attempt) * (0.5 + Math.random());
    this.attempt = Math.min(this.attempt + 1, 6);
    this.retry = setTimeout(() => void this.connect(), delay);
  }

  /** The first route that answers as this machine, each try's time kept. */
  private async findRoute(): Promise<string | null> {
    const m = this.machine();
    const routes = m.routes.map((r) => ({ ...r }));
    let found: string | null = null;
    let env: Environment | null = null;
    for (const r of routes) {
      const t0 = Date.now();
      try {
        const e = await environment(r.url);
        // another machine answers at this address: no credential goes to it
        r.ok = e.id === m.id;
        r.ms = Date.now() - t0;
        if (r.ok && !found) {
          found = r.url;
          env = e;
          break;
        }
      } catch {
        r.ok = false;
        r.ms = undefined;
      }
    }
    const known = new Set(routes.map((r) => r.url));
    const extra = (env?.routes ?? []).filter((r) => !known.has(r.url));
    this.save({
      ...m,
      name: env?.name ?? m.name,
      version: env?.version ?? m.version,
      os: env?.os ?? m.os,
      capabilities: env?.capabilities ?? m.capabilities,
      routes: [...routes, ...extra],
    });
    return found;
  }

  /** A new token before this one runs out (DPoP; an expired one still refreshes). */
  private async refresh(base: string) {
    const m = this.machine();
    if (m.auth.mode !== "dpop") return;
    const r = await request<{ accessToken: string; expiresIn: number }>(base, "POST", "/api/auth/refresh", { auth: m.auth, body: {} });
    const auth = { ...m.auth, token: r.accessToken, expiresAt: Date.now() + r.expiresIn * 1000 };
    this.save({ ...this.machine(), auth });
  }

  private planRefresh() {
    clearTimeout(this.refresher);
    const auth = this.machine().auth;
    if (auth.mode !== "dpop") return;
    const wait = Math.max(5_000, auth.expiresAt - Date.now() - 60_000);
    this.refresher = setTimeout(() => {
      const base = this.route();
      if (base) void this.refresh(base).then(() => this.planRefresh(), () => this.schedule(5_000));
    }, wait);
  }

  private async connect() {
    if (this.stopped || this.connecting) return;
    this.connecting = true;
    this.state.set(this.state() === "pending" ? "pending" : "connecting");
    try {
      const base = await this.findRoute();
      if (!base) {
        this.fail("offline", "unreachable");
        this.schedule();
        return;
      }
      const m = this.machine();
      if (m.auth.mode === "dpop" && m.auth.expiresAt - Date.now() < 60_000) await this.refresh(base);
      if (this.machine().approval === "pending") {
        const me = await request<{ approval: "approved" | "pending"; scopes: string[] }>(base, "GET", "/api/auth/me", { auth: this.machine().auth });
        if (me.approval !== "approved") {
          this.fail("pending");
          this.schedule(5_000);
          return;
        }
        this.save({ ...this.machine(), approval: "approved", scopes: me.scopes ?? this.machine().scopes });
      }
      const { ticket } = await request<{ ticket: string }>(base, "POST", "/api/auth/ws-ticket", { auth: this.machine().auth, body: {} });
      let url = `${wsUrl(base)}?ticket=${encodeURIComponent(ticket)}`;
      const auth = this.machine().auth;
      if (auth.mode === "dpop") url += `&dpop=${encodeURIComponent(await proof(auth.key, "GET", wsUrl(base), auth.token))}`;
      this.open(url, base);
    } catch (e) {
      const code = (e as RpcError)?.code;
      if (code === "unauthorized" || code === "forbidden") {
        // revoked on the machine, or its session ran out: pair again
        this.fail("revoked", code);
        this.schedule(60_000);
      } else {
        this.fail("offline", String((e as Error)?.message ?? e));
        this.schedule();
      }
    } finally {
      this.connecting = false;
    }
  }

  private open(url: string, base: string) {
    this.ws?.close();
    const ws = openSocket(url);
    this.ws = ws;
    const alive = () => {
      clearTimeout(this.watchdog);
      this.watchdog = setTimeout(() => ws.close(), PING_GAP);
    };
    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.attempt = 0;
      this.route.set(base);
      this.error.set(null);
      this.state.set("online");
      alive();
      this.planRefresh();
      for (const fn of [...this.openListeners]) fn();
    };
    ws.onmessage = (data) => {
      if (this.ws !== ws) return;
      alive();
      let msg: { id?: number; result?: unknown; error?: { code: string; message: string }; event?: string; data?: unknown };
      try {
        msg = JSON.parse(data);
      } catch {
        return;
      }
      if (typeof msg.id === "number") {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        clearTimeout(p.timer);
        if (msg.error) p.reject(new RpcError(msg.error.code, msg.error.message));
        else p.resolve(msg.result);
      } else if (msg.event) {
        for (const fn of [...(this.listeners.get(msg.event) ?? [])]) fn(msg.data);
      }
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      clearTimeout(this.watchdog);
      this.fail("offline", "closed");
      this.schedule();
    };
  }
}
