// A middleware for pansy's tests: every hook, doing something a test can see.
export const events = ["content_block_delta", ""]

export function onRequest(body, ctx) {
  ctx.state.seen = (ctx.state.seen ?? 0) + 1
  const text = JSON.stringify(body)
  if (text.includes("forbidden")) return ctx.reject(451, "guard: no forbidden words")
  if (text.includes("throw-please")) throw new Error("asked to throw")
  if (ctx.options.slow) {
    const end = Date.now() + ctx.options.slow
    while (Date.now() < end) {}
  }
  if (!text.includes("secret")) return undefined
  return JSON.parse(text.replaceAll("secret", "[guarded]"))
}

export function onAccount(account, ctx) {
  if (account.model === "skip-me") return { skip: true }
}

export function onProvider(body, ctx) {
  return { ...body, metadata: { to: ctx.provider.id, seen: ctx.state.seen } }
}

export function onEvent(event, ctx) {
  if (event.type === "ping") return null
  if (event.delta?.text) return { ...event, delta: { ...event.delta, text: event.delta.text.toUpperCase() } }
  const c = event.choices?.[0]?.delta?.content
  if (typeof c === "string") return { ...event, choices: [{ ...event.choices[0], delta: { ...event.choices[0].delta, content: c.toUpperCase() } }] }
}

export function onResponse(body, ctx) {
  return { ...body, guarded: ctx.status }
}

export const strategies = {
  reverse: (pool, seats) => seats.map((_, i) => seats.length - 1 - i),
  broken: () => [0, 0],
}

export function onUsage(row, options) {
  if (options.usageFile) require("node:fs").appendFileSync(options.usageFile, JSON.stringify(row) + "\n")
}
