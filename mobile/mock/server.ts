// The mock home pansy, on Bun: serves the phone's web build at `/` and
// mock/fake.ts's protocol at `/api/…`, `/.well-known/pansy/environment`
// and `/ws`, on one port as pansy-remote will (phone-protocol.md "连接").
//
//   nsh package --target web -o dist/web App.tsx
//   bun mock/server.ts [--dir dist/web/Pansy] [--port 7268] [--host 0.0.0.0] [--approval] [--slow 1]
//
// It prints a pairing link (any secret pairs, except one with "expired");
// open it on the phone, or type the address and any code in "Add machine".
// No proof or token is checked. Kept out of `nsh check` (tsconfig.json).
import { networkInterfaces } from "node:os";
import { join, normalize } from "node:path";
import { FakeHome, type Client } from "./fake";

const arg = (name: string, fallback: string) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const dir = arg("dir", join(import.meta.dir, "../dist/web/Pansy"));
const port = Number(arg("port", "7268"));
const host = arg("host", "0.0.0.0");

/** The machine's LAN address, for the link the phone opens. */
function lan(): string {
  for (const list of Object.values(networkInterfaces()))
    for (const a of list ?? []) if (a.family === "IPv4" && !a.internal && /^(10|172|192)\./.test(a.address)) return a.address;
  return "127.0.0.1";
}
const base = `http://${host === "0.0.0.0" ? lan() : host}:${port}`;
const home = new FakeHome({ base, scale: Number(arg("slow", "1")), requireApproval: process.argv.includes("--approval") });
home.onCall = (c) => console.log(`← ${c.method} ${JSON.stringify(c.params).slice(0, 160)}`);

const TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".bin": "application/octet-stream",
};

/** CORS for a page served elsewhere (`nsh run --target web`): its origin, with credentials. */
function cors(req: Request): Record<string, string> {
  const origin = req.headers.get("origin");
  if (!origin) return {};
  return {
    "access-control-allow-origin": origin,
    "access-control-allow-credentials": "true",
    "access-control-allow-headers": "authorization, dpop, content-type, accept",
    "access-control-allow-methods": "GET, POST, OPTIONS",
  };
}

type Data = { deviceId: string; client?: Client; stop?: () => void };

const server = Bun.serve<Data, never>({
  hostname: host,
  port,
  async fetch(req, srv) {
    const url = new URL(req.url);
    if (req.method === "OPTIONS") return new Response(null, { status: 204, headers: cors(req) });
    if (url.pathname === "/ws") {
      const deviceId = home.takeTicket(url.searchParams.get("ticket") ?? "");
      if (!deviceId) return new Response("bad ticket", { status: 401 });
      return srv.upgrade(req, { data: { deviceId } }) ? undefined : new Response("upgrade failed", { status: 400 });
    }
    if (url.pathname.startsWith("/api/") || url.pathname.startsWith("/.well-known/")) {
      const text = req.method === "POST" ? await req.text() : "";
      const headers: Record<string, string> = {};
      req.headers.forEach((v, k) => (headers[k] = v));
      const r = home.http(req.method, url.pathname, text ? JSON.parse(text) : undefined, headers);
      console.log(`${req.method} ${url.pathname} → ${r.status}`);
      return Response.json(r.body ?? {}, { status: r.status, headers: { ...cors(req), ...(r.headers ?? {}) } });
    }
    // the web app
    const path = normalize(decodeURIComponent(url.pathname)).replace(/^(\.\.[/\\])+/, "");
    let file = Bun.file(join(dir, path === "/" ? "index.html" : path));
    if (!(await file.exists())) file = Bun.file(join(dir, "index.html"));
    if (!(await file.exists())) return new Response(`No web build in ${dir}: nsh package --target web -o dist/web App.tsx (the site is dist/web/Pansy)`, { status: 404 });
    const ext = file.name?.match(/\.[a-z0-9]+$/i)?.[0] ?? "";
    return new Response(file, { headers: { "content-type": TYPES[ext] ?? "application/octet-stream", "cache-control": "no-cache" } });
  },
  websocket: {
    open(ws) {
      const client: Client = { shell: false, threads: new Set(), deviceId: ws.data.deviceId, send: (m) => ws.send(JSON.stringify(m)) };
      ws.data.client = client;
      ws.data.stop = home.connect(client);
      console.log(`ws open (${ws.data.deviceId})`);
    },
    message(ws, data) {
      if (ws.data.client) void home.receive(ws.data.client, String(data));
    },
    close(ws) {
      ws.data.stop?.();
      console.log(`ws closed (${ws.data.deviceId})`);
    },
  },
});

// the server's ping every 20 seconds (the phone reconnects after 60 of silence)
setInterval(() => {
  for (const c of home.clients) c.send({ event: "ping", data: {} });
}, 20_000);

const secret = crypto.randomUUID().replaceAll("-", "");
console.log(`mock pansy-remote on http://${server.hostname}:${server.port} (serving ${dir})`);
console.log(`pair: ${base}/#pair=${secret}`);
console.log(`  or: address ${base.replace(/^http:\/\//, "")}, any code`);
