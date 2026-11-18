// Routing's context window, by its numbers: the map's tiles, the prompt as
// counted, the session's headroom.
import { expect, test } from "nsh/test";
import { drawnOf, fmtK, headroom, seriesOf, squarify, tilesOf } from "./contextMap";
import type { Prompt, Route } from "./model";

const prompt: Prompt = {
  tokens: 30_000,
  parts: [
    { kind: "system", tokens: 6_000, items: [{ name: "prompt", tokens: 6_000 }] },
    {
      kind: "tools",
      tokens: 4_000,
      items: [
        { name: "Bash", tokens: 2_500 },
        { name: "Read", tokens: 1_500 },
      ],
    },
    { kind: "memory", tokens: 49, items: [{ name: "/work/CLAUDE.md", tag: "project", tokens: 49 }] },
    {
      kind: "chat",
      tokens: 19_951,
      items: [
        { name: "1", tag: "turn", tokens: 9_951 },
        { name: "2", tag: "turn", tokens: 10_000 },
      ],
    },
  ],
  turns: 2,
  conv: "abc123def456",
};

const route = (id: number, over: Partial<Route> = {}): Route => ({
  id,
  at: "2026-11-12T10:00:00Z",
  agent: "claude-code",
  requested: "anthropic/claude-sonnet-4-5",
  lineup: [{ provider: "anthropic", model: "claude-sonnet-4-5", resting: 0, used: 0, spare: false, window: 200_000 }],
  tries: [{ slot: 0, atUs: 10, status: 200, headUs: 100, us: 1000, rest: 0 }],
  maskingUs: 0,
  done: true,
  status: 200,
  us: 1000,
  firstUs: 100,
  input: 0,
  output: 10,
  cacheRead: 0,
  prompt,
  ...over,
});

test("says token counts as magpie does", () => {
  expect([fmtK(950), fmtK(12_345), fmtK(123_456), fmtK(1_250_000)]).toEqual(["950", "12.3K", "123K", "1.25M"]);
});

test("lays rects that fill the area, each as large as its value", () => {
  const rects = squarify([{ v: 6, a: "a" }, { v: 3, a: "b" }, { v: 1, a: "c" }], 0, 0, 100, 50);
  expect(rects.length).toBe(3);
  const area = rects.reduce((s, r) => s + r.w * r.h, 0);
  expect(Math.round(area)).toBe(5000);
  expect(Math.round(rects[0]!.w * rects[0]!.h)).toBe(3000);
});

test("maps a prompt with the free space beside it, a small part still large enough to point at", () => {
  const tiles = tilesOf(prompt, 200_000, 620, 200);
  const free = tiles.find((t) => t.kind === "free")!;
  expect(free.tokens).toBe(170_000);
  // 15% full, eased: the used share still has room for its tiles.
  expect(free.x).toBeGreaterThan(300);
  const memory = tiles.find((t) => t.kind === "memory")!;
  expect(Math.min(memory.w, memory.h)).toBeGreaterThanOrEqual(15);
  // Every tile in the map.
  for (const t of tiles) {
    expect(t.x + t.w).toBeLessThanOrEqual(620.01);
    expect(t.y + t.h).toBeLessThanOrEqual(200.01);
  }
  // With no window known, the prompt is the whole map.
  expect(tilesOf(prompt, 0, 620, 200).some((t) => t.kind === "free")).toBe(false);
});

test("counts the prompt as the vendor did, its parts scaled to it", () => {
  const live = drawnOf(route(1, { done: false }))!;
  expect([live.counted, live.tokens, live.window]).toEqual([false, 30_000, 200_000]);
  const counted = drawnOf(route(1, { input: 33_000 }))!;
  expect(counted.counted).toBe(true);
  expect(counted.tokens).toBe(33_000);
  expect(counted.parts.find((p) => p.kind === "system")!.tokens).toBe(6_600);
  expect(drawnOf(route(1, { prompt: undefined }))).toBeNull();
});

test("tells the headroom by how the session's prompts grew", () => {
  const all = [10_000, 12_000, 14_500, 16_000].map((input, i) => route(i + 1, { input }));
  // Another conversation, and a subagent's, are not on its line.
  all.push(route(9, { input: 90_000, prompt: { ...prompt, conv: "other" } }));
  all.push(route(10, { input: 5_000, subagent: true }));
  const series = seriesOf(all[3]!, all);
  expect(series.map((p) => p.id)).toEqual([1, 2, 3, 4]);
  // Steps of 2000, 2500 and 1500: 2000 a request.
  expect(headroom(series, 184_000)).toBe(92);
  expect(headroom(series.slice(0, 1), 184_000)).toBeNull();
});
