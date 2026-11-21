// DPoP (RFC 9449) with WebCrypto: the device's P-256 key and the proofs it
// signs (ES256), as phone-protocol.md "鉴权" has them. The key is made
// extractable so its JWK can be kept in Storage, which holds only strings:
// phase 2 moves it to the system keychain, as a key that cannot leave it.

export type Jwk = { kty?: string; crv?: string; x?: string; y?: string; d?: string; [k: string]: unknown };

export type DeviceKey = { privateJwk: Jwk; publicJwk: { kty: "EC"; crv: "P-256"; x: string; y: string } };

const ALG = { name: "ECDSA", namedCurve: "P-256" } as const;

/** Whether this place has WebCrypto: not a browser page on plain HTTP away from localhost. */
export function hasWebCrypto(): boolean {
  const c = (globalThis as { crypto?: { subtle?: { generateKey?: unknown } } }).crypto;
  return typeof c?.subtle?.generateKey === "function";
}

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/** Bytes as base64url, unpadded. */
export function b64url(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    out += B64[(n >> 18) & 63] + B64[(n >> 12) & 63];
    if (i + 1 < bytes.length) out += B64[(n >> 6) & 63];
    if (i + 2 < bytes.length) out += B64[n & 63];
  }
  return out;
}

const utf8 = (s: string) => new TextEncoder().encode(s);

/** A new device key, its JWKs to keep. */
export async function newKey(): Promise<DeviceKey> {
  const pair = (await crypto.subtle.generateKey(ALG as never, true, ["sign", "verify"])) as unknown as { privateKey: unknown };
  const jwk = (await crypto.subtle.exportKey("jwk", pair.privateKey as never)) as Jwk;
  return { privateJwk: jwk, publicJwk: { kty: "EC", crv: "P-256", x: jwk.x!, y: jwk.y! } };
}

const imported = new Map<string, Promise<unknown>>();

function signingKey(key: DeviceKey): Promise<unknown> {
  const id = key.publicJwk.x;
  let k = imported.get(id);
  if (!k) {
    k = crypto.subtle.importKey("jwk", key.privateJwk as never, ALG as never, false, ["sign"]);
    imported.set(id, k);
  }
  return k;
}

/** SHA-256 of `text`, base64url: a token's `ath`. */
export async function sha256(text: string): Promise<string> {
  return b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", utf8(text))));
}

/** A proof for `method` on `url` (its query and fragment left out), bound to `token` when there is one. */
export async function proof(key: DeviceKey, method: string, url: string, token?: string): Promise<string> {
  const htu = url.replace(/[?#].*$/, "");
  const header = { typ: "dpop+jwt", alg: "ES256", jwk: key.publicJwk };
  const payload: Record<string, unknown> = { jti: crypto.randomUUID(), htm: method, htu, iat: Math.floor(Date.now() / 1000) };
  if (token) payload.ath = await sha256(token);
  const input = `${b64url(utf8(JSON.stringify(header)))}.${b64url(utf8(JSON.stringify(payload)))}`;
  const sig = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" } as never, (await signingKey(key)) as never, utf8(input));
  return `${input}.${b64url(new Uint8Array(sig))}`;
}
