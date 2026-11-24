// Pairing (phone-protocol.md "配对"): a link `http(s)://<address>/#pair=<secret>`
// from the computer's QR code, or an address and the code typed by hand;
// the machine's description first, then `POST /api/auth/pair`, with a DPoP
// key where WebCrypto is, else the page's cookie.
import { platform } from "nsh";
import { request, environment, type Auth, type Machine } from "./client";
import { hasWebCrypto, newKey } from "./dpop";
import type { Paired } from "./types";

export const DEFAULT_PORT = 7268;

/** A pairing link's address and secret, or null when it is none. */
export function parsePairUrl(url: string): { base: string; secret: string } | null {
  const m = /^(https?:\/\/[^/?#\s]+)[^#\s]*#(?:[^#]*&)?pair=([A-Za-z0-9_\-]+)/.exec(url.trim());
  return m ? { base: m[1], secret: m[2] } : null;
}

/** An address as typed (`192.168.1.20`, `mac.ts.net:7268`, a URL) as the base URL. */
export function normalizeAddress(address: string): string {
  let a = address.trim().replace(/\/+$/, "");
  if (!/^https?:\/\//.test(a)) a = `http://${a}`;
  const host = a.replace(/^https?:\/\//, "");
  if (!/:\d+$/.test(host) && !/^\[.*\]$/.test(host) && !/\/./.test(host)) a += `:${DEFAULT_PORT}`;
  return a;
}

/** Digits in a typed pairing code. */
export const CODE_DIGITS = 6;

/** A pairing code as typed, kept to its six digits and shown `123-456`. */
export function formatCode(typed: string): string {
  const d = typed.replace(/\D/g, "").slice(0, CODE_DIGITS);
  return d.length > 3 ? `${d.slice(0, 3)}-${d.slice(3)}` : d;
}

/** The page's own pairing link, when the web app was opened from the QR code. */
export function pageUrl(): string | null {
  try {
    const top = (globalThis as { top?: { location?: { href?: string } } }).top;
    return top?.location?.href ?? null;
  } catch {
    return null;
  }
}

/** Takes the secret out of the page's address once it is used: it works once. */
export function clearPageSecret() {
  try {
    const top = (globalThis as { top?: { location: { pathname: string; search: string }; history?: { replaceState(a: unknown, b: string, c: string): void } } }).top;
    top?.history?.replaceState(null, "", top.location.pathname + top.location.search);
  } catch {
    // not a page, or one of another origin
  }
}

/** What the device calls itself to the computer. */
export function deviceName(): string {
  const os = platform.os;
  if (os === "ios") return "iPhone";
  if (os === "android") return "Android";
  if (os === "web") {
    const ua = String((globalThis as { navigator?: { userAgent?: string } }).navigator?.userAgent ?? "");
    const browser = /Edg\//.test(ua) ? "Edge" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : /Firefox\//.test(ua) ? "Firefox" : "Browser";
    const device = /iPhone/.test(ua) ? "iPhone" : /iPad/.test(ua) ? "iPad" : /Android/.test(ua) ? "Android" : /Mac/.test(ua) ? "Mac" : /Windows/.test(ua) ? "Windows" : "";
    return device ? `${browser} · ${device}` : browser;
  }
  return os;
}

const platformName = (): "ios" | "android" | "web" => (platform.os === "ios" ? "ios" : platform.os === "android" ? "android" : "web");

/** Pairs with the machine at `base` with `secret`: the machine to keep. */
export async function pair(base: string, secret: string): Promise<Machine> {
  const env = await environment(base);
  let key = null;
  if (hasWebCrypto()) {
    try {
      key = await newKey();
    } catch {
      key = null;
    }
  }
  const body = { secret, deviceName: deviceName(), platform: platformName(), cookie: !key };
  const r = await request<Paired>(base, "POST", "/api/auth/pair", { body, dpopKey: key ?? undefined });
  const auth: Auth = key && r.accessToken ? { mode: "dpop", key, token: r.accessToken, expiresAt: Date.now() + (r.expiresIn ?? 3600) * 1000 } : { mode: "cookie" };
  const routes = [{ kind: "paired", url: base }, ...env.routes.filter((x) => x.url !== base)];
  return {
    id: env.id,
    name: env.name,
    version: env.version,
    os: env.os,
    capabilities: env.capabilities,
    routes,
    deviceId: r.deviceId,
    scopes: r.scopes,
    approval: r.approval,
    code: r.code,
    auth,
  };
}
