// The machines the phone is paired with (kept in Storage), a connection to
// each, and every machine's thread list as its shell subscription keeps it.
import { effect, lifecycle, signal, Storage } from "nsh";
import { Connection, type ConnState, type Machine } from "../net/client";
import type { ThreadSummary } from "../net/types";

const KEY = "pansy.machines";

export const machines = signal<Machine[]>([]);
/** Machines are read from Storage. */
export const ready = signal(false);
/** Each machine's threads, by machine id; absent until its shell answers. */
export const shells = signal<Record<string, ThreadSummary[]>>({});

const conns = new Map<string, Connection>();

export const machine = (id: string) => machines().find((m) => m.id === id);
export const conn = (id: string) => conns.get(id);
export const connState = (id: string): ConnState => conns.get(id)?.state() ?? "offline";
export const online = (id: string) => connState(id) === "online";

function persist() {
  void Storage.setItem(KEY, JSON.stringify(machines()));
}

function update(m: Machine) {
  machines.set(machines().map((x) => (x.id === m.id ? m : x)));
  persist();
}

function setShell(id: string, threads: ThreadSummary[] | undefined) {
  const next = { ...shells() };
  if (threads) next[id] = threads;
  else delete next[id];
  shells.set(next);
}

function upsert(id: string, t: ThreadSummary) {
  const list = shells()[id] ?? [];
  const at = list.findIndex((x) => x.id === t.id);
  setShell(id, at < 0 ? [t, ...list] : list.map((x) => (x.id === t.id ? t : x)));
}

function connect(m: Machine) {
  const id = m.id;
  const c = new Connection(() => machine(id) ?? m, update);
  conns.set(id, c);
  c.on("shell.thread", (t: ThreadSummary) => upsert(id, t));
  c.on("shell.removed", (d: { threadId: string }) => setShell(id, (shells()[id] ?? []).filter((t) => t.id !== d.threadId)));
  c.onOpen(() => {
    void c.call<{ machine?: { name?: string }; threads: ThreadSummary[] }>("shell.subscribe").then(
      (r) => setShell(id, r.threads),
      () => {},
    );
  });
  c.start();
}

let started = false;

/** Reads the machines and connects to each; once. */
export async function start() {
  if (started) return;
  started = true;
  try {
    const saved = JSON.parse((await Storage.getItem(KEY)) ?? "[]") as Machine[];
    machines.set(Array.isArray(saved) ? saved : []);
  } catch {
    machines.set([]);
  }
  ready.set(true);
  for (const m of machines()) connect(m);
  // back in front: whatever dropped tries again at once
  effect(() => {
    if (lifecycle() === "active") for (const c of conns.values()) c.wake();
  });
}

/** Keeps `m` (a new pairing, or one again) and connects to it. */
export function addMachine(m: Machine) {
  conns.get(m.id)?.stop();
  machines.set([...machines().filter((x) => x.id !== m.id), m]);
  persist();
  connect(m);
}

export function removeMachine(id: string) {
  conns.get(id)?.stop();
  conns.delete(id);
  machines.set(machines().filter((m) => m.id !== id));
  setShell(id, undefined);
  persist();
}

/** The routes of a machine in a new order, or with one added or removed. */
export function setRoutes(id: string, routes: Machine["routes"]) {
  const m = machine(id);
  if (!m) return;
  update({ ...m, routes });
}

/** Every thread of every machine, with the machine it is on. */
export type Entry = { machine: Machine; thread: ThreadSummary };

export function entries(): Entry[] {
  const out: Entry[] = [];
  for (const m of machines()) for (const t of shells()[m.id] ?? []) out.push({ machine: m, thread: t });
  return out;
}

/** A thread's summary as its machine's shell has it now. */
export const summary = (machineId: string, threadId: string) => (shells()[machineId] ?? []).find((t) => t.id === threadId);

/** Replaces a thread's summary where an answer gave it (thread.update). */
export const putSummary = upsert;

/** Wakes every connection: pull to refresh. */
export function wakeAll() {
  for (const c of conns.values()) c.wake();
}
