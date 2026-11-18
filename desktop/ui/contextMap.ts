// What Routing's context window draws, as magpie's context map: a request's
// prompt against its model's window, counted where the vendor said how many
// tokens it read, the session's requests one after another, and the map —
// each part a tile sized by its tokens, its items tiles inside it, the free
// space one of its share. Numbers only; ui/Context.tsx draws them.
import type { Prompt, PromptItem, PromptKind, Route } from "./model";

/** The parts, in the order they are drawn. */
export const PARTS: PromptKind[] = ["system", "tools", "skills", "memory", "files", "results", "chat"];

/** 950, 12.3K, 1.25M. */
export function fmtK(n: number): string {
  n = Math.max(0, Math.round(n || 0));
  const short = (x: number, d: number) => x.toFixed(d).replace(/\.0+$/, "");
  if (n >= 1e6) return `${short(n / 1e6, n >= 1e7 ? 1 : 2)}M`;
  if (n >= 1e3) return `${short(n / 1e3, n >= 1e5 ? 0 : 1)}K`;
  return String(n);
}

export const pct = (x: number) => `${Math.round(x * 100)}%`;

/** How full a window is: under 75% healthy, under 90% filling up, else near full. */
export const health = (fill: number): "ok" | "warn" | "bad" => (fill < 0.75 ? "ok" : fill < 0.9 ? "warn" : "bad");

/** The window of the model the request went to (the last one asked), else of the first lined up; 0 when not known. */
export function windowOf(r: Route): number {
  const last = r.tries[r.tries.length - 1];
  return (last ? r.lineup[last.slot]?.window : undefined) ?? r.lineup[0]?.window ?? 0;
}

/** The prompt as drawn: as many tokens as the vendor counted once it answered, its parts scaled to them; else as estimated. */
export interface Drawn extends Prompt {
  counted: boolean;
  window: number;
}

export function drawnOf(r: Route): Drawn | null {
  const p = r.prompt;
  if (!p || p.tokens <= 0) return null;
  const window = windowOf(r);
  if (!r.done || r.input <= 0) return { ...p, counted: false, window };
  const k = r.input / p.tokens;
  const scale = (n: number) => Math.round(n * k);
  return {
    ...p,
    tokens: r.input,
    parts: p.parts.map((x) => ({ ...x, tokens: scale(x.tokens), items: x.items.map((it) => ({ ...it, tokens: scale(it.tokens) })) })),
    counted: true,
    window,
  };
}

/** What the vendor read from its cache, once it answered. */
export function cacheOf(r: Route): { read: number; total: number } | null {
  return r.done && r.input > 0 ? { read: Math.min(r.cacheRead, r.input), total: r.input } : null;
}

/** A request's place on its session's line. */
export interface Point {
  id: number;
  tokens: number;
}

/** The requests of `r`'s conversation (its agent's, its first prompt's, a subagent's apart), oldest first. */
export function seriesOf(r: Route, all: Route[]): Point[] {
  const conv = r.prompt?.conv;
  if (!conv) return [];
  return all
    .filter((x) => x.prompt?.conv === conv && x.agent === r.agent && !!x.subagent === !!r.subagent)
    .sort((a, b) => a.id - b.id)
    .map((x) => ({ id: x.id, tokens: drawnOf(x)?.tokens ?? 0 }))
    .filter((x) => x.tokens > 0);
}

const median = (xs: number[]) => {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  return s[s.length >> 1]!;
};

/** How many more requests fit in `free`, at the pace the session's prompts grew; null with no growth to go by. */
export function headroom(series: Point[], free: number): number | null {
  const steps: number[] = [];
  for (let i = 1; i < series.length; i++) {
    const d = series[i]!.tokens - series[i - 1]!.tokens;
    // A prompt that shrank (compacted) is no step.
    if (d > 0 && series[i]!.tokens >= series[i - 1]!.tokens * 0.6) steps.push(d);
  }
  const g = median(steps);
  return g > 0 ? Math.floor(free / g) : null;
}

/** A value to lay out, and what it stands for. */
interface Val<T> {
  v: number;
  a: T;
}
export interface Rect<T> {
  a: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Lays `vals` in the rect as rects whose sides are as near alike as it can (Bruls, Huizing, van Wijk), in their order. */
export function squarify<T>(vals: Val<T>[], x: number, y: number, w: number, h: number): Rect<T>[] {
  const out: Rect<T>[] = [];
  vals = vals.filter((a) => a.v > 0);
  const total = vals.reduce((s, a) => s + a.v, 0);
  if (!total || w <= 0 || h <= 0) return out;
  const k = (w * h) / total;
  const worst = (row: Val<T>[], side: number) => {
    let sum = 0;
    let hi = 0;
    let lo = Infinity;
    for (const a of row) {
      const s = a.v * k;
      sum += s;
      hi = Math.max(hi, s);
      lo = Math.min(lo, s);
    }
    return Math.max((side * side * hi) / (sum * sum), (sum * sum) / (side * side * lo));
  };
  const lay = (row: Val<T>[]) => {
    const sum = row.reduce((s, a) => s + a.v * k, 0);
    if (w >= h) {
      const cw = sum / h;
      let cy = y;
      for (const a of row) {
        const ch = (a.v * k) / cw;
        out.push({ a: a.a, x, y: cy, w: cw, h: ch });
        cy += ch;
      }
      x += cw;
      w -= cw;
    } else {
      const rh = sum / w;
      let cx = x;
      for (const a of row) {
        const cw = (a.v * k) / rh;
        out.push({ a: a.a, x: cx, y, w: cw, h: rh });
        cx += cw;
      }
      y += rh;
      h -= rh;
    }
  };
  let row: Val<T>[] = [];
  for (const a of vals) {
    const side = Math.min(w, h);
    if (!row.length || worst([...row, a], side) <= worst(row, side)) {
      row.push(a);
      continue;
    }
    lay(row);
    row = [a];
  }
  if (row.length) lay(row);
  return out;
}

/** A tile of the map: a part, one of its items, its small ones together, or the free space. */
export interface Tile {
  /** Stays the same across draws. */
  key: string;
  kind: PromptKind | "free";
  /** The item it is; none for a part's tile, its small ones' or the free space. */
  it?: PromptItem;
  /** The items it stands for. */
  its: PromptItem[];
  tokens: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** An item's tile is at least this big (px²); smaller ones go together. */
const TILE_MIN = 1500;
/** The most items a part shows as tiles of their own. */
const TILE_MAX = 12;
/** No tile is laid smaller than this (px²): a sliver can't be pointed at. */
const TILE_FLOOR = 700;
/** Nor thinner than this across. */
const TILE_SIDE = 16;

/**
 * Lays a prompt out on a map `W`×`H`: the used share of the window and the
 * free space side by side along its long side (the used share eased, so a
 * window 8% full still has room to read its tiles), the parts squarified in
 * it, a part's items in the part. An item too small to read goes in one
 * tile with the part's other small ones; a part too small for its items is
 * one tile.
 */
export function tilesOf(p: Prompt, window: number, W: number, H: number): Tile[] {
  const scale = Math.max(window, p.tokens, 1);
  const across = W >= H;
  const long = across ? W : H;
  let usedLen = long;
  if (p.tokens < scale) usedLen = Math.min(long, Math.max(long * Math.pow(p.tokens / scale, 0.35), Math.min(long, 36)));
  const tiles: Tile[] = [];
  const parts = PARTS.map((k) => p.parts.find((x) => x.kind === k)).filter((x): x is Prompt["parts"][number] => !!x && x.tokens > 0);
  const ur: [number, number, number, number] = across ? [0, 0, usedLen, H] : [0, 0, W, usedLen];
  const floored = <T>(vals: Val<T>[], area: number): Val<T>[] => {
    const sum = vals.reduce((s, a) => s + a.v, 0);
    const lo = (sum * TILE_FLOOR) / Math.max(1, area);
    return vals.map((a) => ({ ...a, v: Math.max(a.v, lo) }));
  };
  // A part laid as a strip beside large ones (49 tokens beside 70K) is
  // raised until it is TILE_SIDE across both ways.
  const partVals = floored(parts.map((x) => ({ v: x.tokens, a: x })), ur[2] * ur[3]);
  let partRects = squarify(partVals, ...ur);
  for (let n = 0; n < 8; n++) {
    const thin = partRects.filter((r) => Math.min(r.w, r.h) < TILE_SIDE);
    if (!thin.length) break;
    for (const r of thin) {
      const v = partVals.find((a) => a.a === r.a)!;
      v.v *= (TILE_SIDE / Math.max(1, Math.min(r.w, r.h))) * 1.05;
    }
    partRects = squarify(partVals, ...ur);
  }
  for (const r of partRects) {
    const x = r.a;
    const items = x.items.filter((it) => it.tokens > 0).sort((a, b) => b.tokens - a.tokens);
    const isum = items.reduce((s, it) => s + it.tokens, 0);
    const area = r.w * r.h;
    const whole = () => tiles.push({ key: x.kind, kind: x.kind, its: items, tokens: x.tokens, x: r.x, y: r.y, w: r.w, h: r.h });
    if (!items.length || area < TILE_MIN * 2) {
      whole();
      continue;
    }
    const kept: PromptItem[] = [];
    const rest: PromptItem[] = [];
    for (const it of items) {
      if (kept.length < TILE_MAX && (it.tokens / isum) * area >= TILE_MIN) kept.push(it);
      else rest.push(it);
    }
    const vals: Val<{ it?: PromptItem; rest?: PromptItem[] }>[] = kept.map((it) => ({ v: it.tokens, a: { it } }));
    if (rest.length === 1 && !kept.length) vals.push({ v: rest[0]!.tokens, a: { it: rest[0] } });
    else if (rest.length) vals.push({ v: rest.reduce((s, it) => s + it.tokens, 0), a: { rest } });
    const laid = squarify(floored(vals, area), r.x, r.y, r.w, r.h);
    // Items that would leave one a strip too thin to read: the part is one tile.
    if (laid.some((q) => Math.min(q.w, q.h) < 14)) {
      whole();
      continue;
    }
    for (const q of laid) {
      const it = q.a.it;
      const its = it ? [it] : q.a.rest!;
      tiles.push({
        key: `${x.kind}\u0000${it ? `${it.tag ?? ""}\u0000${it.name}` : "\u0001rest"}`,
        kind: x.kind,
        it,
        its,
        tokens: (its.reduce((s, y) => s + y.tokens, 0) / Math.max(1, isum)) * x.tokens,
        x: q.x,
        y: q.y,
        w: q.w,
        h: q.h,
      });
    }
  }
  if (p.tokens < scale) {
    const fr = across ? [usedLen, 0, W - usedLen, H] : [0, usedLen, W, H - usedLen];
    tiles.push({ key: "\u0000free", kind: "free", its: [], tokens: scale - p.tokens, x: fr[0]!, y: fr[1]!, w: fr[2]!, h: fr[3]! });
  }
  return tiles;
}

/** The grid's cells: one for each 1/400 of the window. */
export const CELLS = 400;

/** A cell of the grid: the part and item it shows, or null for free space. */
export type Cell = { kind: PromptKind; it?: PromptItem } | null;

/**
 * The grid, magpie's other look: the window as `CELLS` cells, the used ones
 * shared out among the parts by their tokens (largest remainder, a part of
 * any size at least one), each cell the item its middle falls in. A window
 * smaller than the prompt (not known, or overrun) is the prompt.
 */
export function cellsOf(p: Prompt, window: number): Cell[] {
  const scale = Math.max(window, p.tokens, 1);
  const used = Math.min(CELLS, Math.round((p.tokens / scale) * CELLS));
  const parts = PARTS.map((k) => p.parts.find((x) => x.kind === k)).filter((x) => !!x && x.tokens > 0) as Prompt["parts"];
  const sum = parts.reduce((s, x) => s + x.tokens, 0) || 1;
  const want = parts.map((x) => (x.tokens / sum) * used);
  const got = want.map((w) => Math.max(1, Math.floor(w)));
  let left = used - got.reduce((s, n) => s + n, 0);
  const order = want.map((w, i) => [w - Math.floor(w), i] as const).sort((a, b) => b[0] - a[0]);
  for (let j = 0; left > 0 && order.length; j = (j + 1) % order.length, left--) got[order[j]![1]]!++;
  for (let j = got.length - 1; left < 0 && j >= 0; j--) {
    while (left < 0 && got[j]! > 1) {
      got[j]!--;
      left++;
    }
  }
  const cells: Cell[] = [];
  parts.forEach((x, pi) => {
    const items = [...x.items].sort((a, b) => b.tokens - a.tokens);
    const isum = items.reduce((s, it) => s + it.tokens, 0) || 1;
    let at = 0;
    let acc = 0;
    for (let c = 0; c < got[pi]!; c++) {
      const mid = ((c + 0.5) / got[pi]!) * isum;
      while (at < items.length - 1 && acc + items[at]!.tokens < mid) acc += items[at++]!.tokens;
      cells.push({ kind: x.kind, it: items[at] });
    }
  });
  while (cells.length < CELLS) cells.push(null);
  return cells.slice(0, CELLS);
}
