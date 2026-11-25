// Where requests and sockets go: the platform's fetch and WebSocket, or a
// stand-in (test/harness.tsx puts mock/fake.ts's home here, so the app's
// tests run its whole client against a scripted pansy).

/** A text WebSocket, as the client uses one. */
export interface Socket {
  send(data: string): void;
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((data: string) => void) | null;
  onclose: ((reason: string) => void) | null;
}

export type Transport = {
  fetch: (url: string, init: { method: string; headers: Record<string, string>; body?: string; signal?: AbortSignal }) => Promise<Response>;
  socket: (url: string) => Socket;
};

function platformSocket(url: string): Socket {
  const ws = new WebSocket(url);
  const s: Socket = {
    send: (data) => ws.send(data),
    close: () => ws.close(),
    onopen: null,
    onmessage: null,
    onclose: null,
  };
  let closed = false;
  const close = (reason: string) => {
    if (closed) return;
    closed = true;
    s.onclose?.(reason);
  };
  ws.onopen = () => s.onopen?.();
  ws.onmessage = (e: { data: unknown }) => s.onmessage?.(typeof e.data === "string" ? e.data : String(e.data));
  ws.onerror = () => close("error");
  ws.onclose = (e: { code?: number }) => close(`closed ${e?.code ?? ""}`);
  return s;
}

let transport: Transport = {
  // cookie sessions (a page on plain HTTP) need the browser's cookie sent along
  fetch: (url, init) => fetch(url, { ...init, credentials: "include" } as RequestInit),
  socket: platformSocket,
};

export const http = (url: string, init: Parameters<Transport["fetch"]>[1]) => transport.fetch(url, init);
export const openSocket = (url: string) => transport.socket(url);

/** Puts a stand-in in the platform's place. */
export function setTransport(t: Transport) {
  transport = t;
}
