// The app with mock/fake.ts's home behind its transport, for App.test.tsx:
// requests and the socket go to the fake (no network), and every RPC the
// app makes is written to Storage's `test.calls` for the tests to read.
import { mount, Storage } from "nsh";
import { FakeHome } from "../mock/fake";
import { setTransport } from "../net/transport";
import { Root } from "../ui/Root";

export const BASE = "http://home.test:7268";

const home = new FakeHome({ base: BASE, scale: 0.02 });
const calls: { method: string; params: Record<string, unknown> }[] = [];
home.onCall = (c) => {
  calls.push(c);
  void Storage.setItem("test.calls", JSON.stringify(calls));
};

setTransport({
  fetch: async (url, init) => {
    const u = new URL(url);
    if (u.origin !== BASE) throw new Error(`no route to ${u.origin}`);
    const r = home.http(init.method, u.pathname, init.body ? JSON.parse(init.body) : undefined, init.headers);
    return new Response(r.body === undefined ? "" : JSON.stringify(r.body), { status: r.status, headers: { "content-type": "application/json", ...(r.headers ?? {}) } });
  },
  socket: (url) => home.socket(url),
});

mount(Root);
