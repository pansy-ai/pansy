// pansy's host B: magpie's gateway middleware (magpie.middleware) and agent
// plugins (magpie.agent) on Bun, answering pansy over stdin/stdout, one JSON
// message a line, as host.js does: {id, method, params} answered {id,
// result} or {id, error}.
//
// The hooks, their ctx and what they may return are magpie's
// (docs/subsystems/gateway-middleware.md in magpie, internal/middleware):
//
//   onRequest(body, ctx)   the agent's request, before it is routed: an
//                          object is the body the next hook and the gateway
//                          get, undefined keeps it, ctx.reject(status,
//                          message) turns it away
//   onAccount(account, ctx) before each try on a subscription's or plugin's
//                          account: {skip: true} passes it over
//   onProvider(body, ctx)  each try's body, ctx.provider saying where it goes
//   onEvent(event, ctx)    each streamed event (export const events names
//                          the ones wanted): an object replaces it, null
//                          drops it
//   onResponse(body, ctx)  a whole JSON reply, ctx.status its status
//   strategies             {name: (pool, seats, options) => order}
//   onUsage(row, options)  each usage row, off the request's path
//
// A hook that throws, returns what it may not, or runs past its time (250ms,
// 50ms an event, 20ms a strategy) leaves things as they were and counts as a
// failure. pansy holds the hooks to their time too, and starts this host
// again when one never answers.

import readline from "node:readline"
import { pathToFileURL } from "node:url"

const write = process.stdout.write.bind(process.stdout)
const out = (msg) => write(JSON.stringify(msg) + "\n")
// What a plugin prints goes to stderr, which pansy logs: stdout is answers.
const toErr = (...xs) => process.stderr.write(xs.map((x) => (typeof x === "string" ? x : Bun.inspect(x))).join(" ") + "\n")
console.log = console.info = console.debug = console.warn = console.error = toErr
process.stdout.write = (chunk, enc, cb) => process.stderr.write(chunk, enc, cb)

const LIMIT = { request: 250, event: 50, load: 1000, strategy: 20, usage: 250 }
const HOOKS = ["onRequest", "onAccount", "onProvider", "onEvent", "onResponse"]

const mws = []
const agents = []
const runs = new Map()

// Failures logged, at most 60 lines a minute.
let logged = 0
setInterval(() => (logged = 0), 60_000).unref()
function fail(rec, hook, e) {
  rec.stats.failures++
  rec.stats.lastError = `${hook}: ${e?.message ?? e}`
  if (logged++ < 60) toErr(`[${rec.name}] ${rec.stats.lastError}`)
}

function late(ms, what) {
  return new Promise((_, no) => setTimeout(() => no(new Error(`${what} took more than ${ms}ms`)), ms).unref?.())
}

// call runs one hook within its time: {ok, value}. A hook that kept the
// thread past its time is a failure too, its answer not taken.
async function call(rec, hook, limit, fn) {
  const started = performance.now()
  rec.stats.calls++
  try {
    const value = await Promise.race([Promise.resolve().then(fn), late(limit, hook)])
    const ms = performance.now() - started
    rec.stats.totalMs += ms
    if (ms > limit) throw new Error(`${hook} took ${Math.round(ms)}ms, past its ${limit}ms`)
    return { ok: true, value }
  } catch (e) {
    rec.stats.totalMs += performance.now() - started
    fail(rec, hook, e)
    return { ok: false }
  }
}

const isObject = (v) => v !== null && typeof v === "object"

async function load(item) {
  const mod = await Promise.race([import(pathToFileURL(item.file).href), late(LIMIT.load, "loading")])
  return mod
}

async function init({ middleware = [], agents: ags = [] }) {
  for (const m of middleware) {
    const rec = { name: m.name, spec: m.spec, options: m.options ?? {}, hooks: [], events: null, strategies: [], usage: false, error: null, stats: { calls: 0, failures: 0, totalMs: 0, lastError: "" } }
    try {
      const mod = await load(m)
      rec.mod = mod
      rec.hooks = HOOKS.filter((h) => typeof mod[h] === "function")
      if (Array.isArray(mod.events)) rec.events = new Set(mod.events.map(String))
      if (isObject(mod.strategies)) rec.strategies = Object.keys(mod.strategies).filter((k) => typeof mod.strategies[k] === "function" || typeof mod.strategies[k]?.order === "function")
      rec.usage = typeof mod.onUsage === "function"
      if (!rec.hooks.length && !rec.strategies.length && !rec.usage) rec.error = "exports no gateway hook (onRequest, onAccount, onProvider, onEvent, onResponse), strategies or onUsage"
    } catch (e) {
      rec.error = String(e?.message ?? e)
    }
    mws.push(rec)
  }
  for (const a of ags) {
    const rec = { spec: a.spec, options: a.options ?? {}, agent: null, connect: null, error: null }
    try {
      const mod = await load(a)
      const spec = mod.agent ?? mod.default?.agent
      if (!isObject(spec) || typeof spec.id !== "string" || !spec.id) throw new Error("exports no agent with an id")
      rec.agent = JSON.parse(JSON.stringify(spec))
      rec.connect = typeof mod.connect === "function" ? mod.connect : typeof mod.default?.connect === "function" ? mod.default.connect : null
      if (!rec.connect && !spec.model) throw new Error("agent.model names no key, and there is no connect to write the gateway: say one or both")
    } catch (e) {
      rec.error = String(e?.message ?? e)
    }
    agents.push(rec)
  }
  return { middleware: mws.map(describe), agents: agents.map((a) => ({ spec: a.spec, agent: a.agent, connect: !!a.connect, error: a.error })) }
}

function describe(rec) {
  return {
    name: rec.name,
    spec: rec.spec,
    hooks: rec.hooks,
    events: rec.events ? [...rec.events] : null,
    strategies: rec.strategies,
    usage: rec.usage,
    error: rec.error,
  }
}

function stats() {
  return Object.fromEntries(mws.map((m) => [m.name, { ...describe(m), calls: m.stats.calls, failures: m.stats.failures, avgMs: m.stats.calls ? m.stats.totalMs / m.stats.calls : 0, lastError: m.stats.lastError }]))
}

// ctxOf is middleware i's ctx in a run: the request's info, its options and
// a state of its own, kept for all its hooks of the request.
function ctxOf(run, i) {
  if (!run.ctxs[i]) {
    const rec = mws[i]
    run.ctxs[i] = {
      ...run.info,
      options: structuredClone(rec.options),
      state: {},
      reject(status, message) {
        if (run.rejected) return
        run.rejected = {
          status: Number.isInteger(status) && status >= 400 && status <= 599 ? status : 403,
          message: typeof message === "string" ? message : "turned away by a gateway middleware",
        }
      },
    }
  }
  return run.ctxs[i]
}

function runOf(id) {
  const run = runs.get(id)
  if (!run) throw new Error("no such run")
  return run
}

const handlers = {
  init,
  stats,
  ping: () => "pong",
  begin({ info }) {
    const id = crypto.randomUUID()
    runs.set(id, { info: info ?? {}, ctxs: [], rejected: null })
    return { run: id }
  },
  end({ run }) {
    runs.delete(run)
    return null
  },
  async request({ run: id, body }) {
    const run = runOf(id)
    let changed = false
    for (const [i, rec] of mws.entries()) {
      if (!rec.hooks.includes("onRequest")) continue
      const ctx = ctxOf(run, i)
      const r = await call(rec, "onRequest", LIMIT.request, () => rec.mod.onRequest(structuredClone(body), ctx))
      if (run.rejected) return { reject: { ...run.rejected, by: rec.name } }
      if (!r.ok || r.value === undefined) continue
      if (!isObject(r.value)) {
        fail(rec, "onRequest", "returned neither an object nor undefined")
        continue
      }
      body = r.value
      changed = true
    }
    return changed ? { body } : {}
  },
  async account({ run: id, account, refs }) {
    const run = runOf(id)
    for (const [i, rec] of mws.entries()) {
      if (!rec.hooks.includes("onAccount")) continue
      const ctx = ctxOf(run, i)
      if (!account) {
        delete ctx.account
        continue
      }
      const a = { ...account, ref: refs?.[rec.name] ?? "" }
      ctx.account = a
      const r = await call(rec, "onAccount", LIMIT.request, () => rec.mod.onAccount(structuredClone(a), ctx))
      if (!r.ok || r.value === undefined) continue
      if (r.value === null) {
        fail(rec, "onAccount", "returned null; return {skip: true} to pass the account over")
        continue
      }
      if (isObject(r.value) && r.value.skip === true) return { skip: true, by: rec.name }
    }
    return { skip: false }
  },
  async provider({ run: id, body, provider }) {
    const run = runOf(id)
    let changed = false
    for (const [i, rec] of mws.entries()) {
      if (!rec.hooks.includes("onProvider")) continue
      const ctx = ctxOf(run, i)
      ctx.provider = provider
      const r = await call(rec, "onProvider", LIMIT.request, () => rec.mod.onProvider(structuredClone(body), ctx))
      if (!r.ok || r.value === undefined) continue
      if (!isObject(r.value)) {
        fail(rec, "onProvider", r.value === null ? "returned null; a request can't be dropped" : "returned neither an object nor undefined")
        continue
      }
      body = r.value
      changed = true
    }
    return changed ? { body } : {}
  },
  async event({ run: id, name, data }) {
    const run = runOf(id)
    let changed = false
    for (const [i, rec] of mws.entries()) {
      if (!rec.hooks.includes("onEvent")) continue
      if (rec.events && !rec.events.has(name ?? "")) continue
      const ctx = ctxOf(run, i)
      const r = await call(rec, "onEvent", LIMIT.event, () => rec.mod.onEvent(structuredClone(data), ctx))
      if (!r.ok || r.value === undefined) continue
      if (r.value === null) return { drop: true }
      if (!isObject(r.value)) {
        fail(rec, "onEvent", "returned neither an object, null nor undefined")
        continue
      }
      data = r.value
      changed = true
    }
    return changed ? { data } : {}
  },
  async response({ run: id, body, status }) {
    const run = runOf(id)
    let changed = false
    for (const [i, rec] of mws.entries()) {
      if (!rec.hooks.includes("onResponse")) continue
      const ctx = ctxOf(run, i)
      ctx.status = status
      const r = await call(rec, "onResponse", LIMIT.request, () => rec.mod.onResponse(structuredClone(body), ctx))
      if (!r.ok || r.value === undefined) continue
      if (!isObject(r.value)) {
        fail(rec, "onResponse", r.value === null ? "returned null; a whole reply can't be dropped" : "returned neither an object nor undefined")
        continue
      }
      body = r.value
      changed = true
    }
    return changed ? { body } : {}
  },
  // strategy is the order a routing of a middleware's gives a pool's seats:
  // a permutation of their indexes, or null for their listed order.
  async strategy({ name, strategy, pool, seats }) {
    const rec = mws.find((m) => m.name === name)
    if (!rec || !rec.strategies.includes(strategy)) return { order: null }
    const s = rec.mod.strategies[strategy]
    const fn = typeof s === "function" ? s : s.order.bind(s)
    const r = await call(rec, `strategies.${strategy}`, LIMIT.strategy, () => fn(pool, structuredClone(seats), structuredClone(rec.options)))
    if (!r.ok) return { order: null }
    const order = r.value
    const n = seats.length
    if (!Array.isArray(order) || order.length !== n || new Set(order).size !== n || !order.every((i) => Number.isInteger(i) && i >= 0 && i < n)) {
      fail(rec, `strategies.${strategy}`, "returned what isn't an order of the seats")
      return { order: null }
    }
    return { order }
  },
  async usage({ row }) {
    for (const rec of mws) {
      if (!rec.usage) continue
      await call(rec, "onUsage", LIMIT.usage, () => rec.mod.onUsage(structuredClone(row), structuredClone(rec.options)))
    }
    return null
  },
  async connect({ spec, id, args }) {
    const rec = agents.find((a) => (spec && a.spec === spec) || a.agent?.id === id)
    if (!rec || rec.error) throw new Error(rec?.error ?? `no agent plugin ${id ?? spec}`)
    if (!rec.connect) return {}
    const keys = await Promise.race([Promise.resolve().then(() => rec.connect(args ?? {})), late(5000, "connect")])
    if (keys !== undefined && !isObject(keys)) throw new Error("connect returned what isn't an object of keys")
    return JSON.parse(JSON.stringify(keys ?? {}))
  },
}

const rl = readline.createInterface({ input: process.stdin, terminal: false })
rl.on("line", (line) => {
  if (!line.trim()) return
  let msg
  try {
    msg = JSON.parse(line)
  } catch {
    return
  }
  if (msg.id === undefined) return
  const fn = handlers[msg.method]
  if (!fn) return out({ id: msg.id, error: { message: `no method ${msg.method}` } })
  Promise.resolve()
    .then(() => fn(msg.params ?? {}))
    .then(
      (result) => out({ id: msg.id, result: result ?? null }),
      (e) => out({ id: msg.id, error: { message: String(e?.message ?? e) } }),
    )
})
rl.on("close", () => process.exit(0))
