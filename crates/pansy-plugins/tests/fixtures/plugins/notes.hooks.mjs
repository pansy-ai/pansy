// Request hooks for pansy's tests: a note added before routing, and the
// reply written where the options say once it is whole.
import fs from "node:fs"

export async function onRequest(body, ctx) {
  const messages = [...(body.messages ?? []), { role: "user", content: `note from ${ctx.origin}` }]
  return { ...body, messages }
}

export async function onResponse(reply, ctx) {
  if (ctx.options.replyFile) fs.writeFileSync(ctx.options.replyFile, JSON.stringify({ reply, success: ctx.success }))
}
