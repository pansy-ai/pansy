// Routing's context window, as magpie draws it under the story: what the
// request's prompt held against its model's window — how full, how much the
// vendor read from its cache, the map (each part a tile sized by its
// tokens, its items inside it, the free space hatched), the contents part
// by part, and the session's requests one after another. Names and sizes
// only: the gateway keeps no prompt's text.
import { Canvas, colorScheme, Scroll, signal, Text, token, View, type JSX } from "nsh";
import { Tooltip } from "nsh/base";
import { PARTS, cacheOf, drawnOf, fmtK, headroom, health, pct, seriesOf, tilesOf, type Drawn, type Point, type Tile } from "./contextMap";
import { tr, trn } from "./i18n";
import { contextShut, routePicked, type PromptItem, type PromptKind, type Route } from "./model";
import { agentLogo, Icon, Logo, SHADOW, Segs, TextButton } from "./parts";
import { agentName, listed } from "./routeStory";

/** magpie's part colors, light and dark. */
const INK: Record<PromptKind, [string, string]> = {
  system: ["#8b5cf6", "#c4b5fd"],
  tools: ["#0ea5e9", "#7dd3fc"],
  skills: ["#64748b", "#94a3b8"],
  memory: ["#f59e0b", "#fcd34d"],
  files: ["#10b981", "#6ee7b7"],
  results: ["#ea580c", "#fdba74"],
  chat: ["#4f46e5", "#a5b4fc"],
};
export const inkOf = (k: PromptKind) => INK[k][colorScheme() === "dark" ? 1 : 0];

/** A color's red, green and blue: `#rrggbb(aa)`, or `rgb(…)` as a theme token may come. */
const rgb = (color: string): [number, number, number] | null => {
  const hex = /^#([0-9a-f]{6})(?:[0-9a-f]{2})?$/i.exec(color.trim());
  if (hex) {
    const n = parseInt(hex[1]!, 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  const fn = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(color.trim());
  return fn ? [Math.round(+fn[1]!), Math.round(+fn[2]!), Math.round(+fn[3]!)] : null;
};
/** `hex` at `a` opacity. */
const alpha = (hex: string, a: number) => {
  const c = rgb(hex);
  return c ? `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${a})` : hex;
};
/** `share` of `a` over `b`, as CSS's color-mix. */
const mix = (a: string, b: string, share: number) => {
  const x = rgb(a);
  const y = rgb(b);
  if (!x || !y) return a;
  const c = x.map((v, i) => Math.round(v * share + y[i]! * (1 - share)));
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
};

const PART_NAMES: Record<PromptKind, () => string> = {
  system: () => tr("System prompt"),
  tools: () => tr("Tools"),
  skills: () => tr("Skills"),
  memory: () => tr("Memory"),
  files: () => tr("Files"),
  results: () => tr("Tool results"),
  chat: () => tr("Conversation"),
};
const PART_NOTES: Record<PromptKind, () => string> = {
  system: () => tr("The agent's own instructions"),
  tools: () => tr("The definitions of the tools it can call"),
  skills: () => tr("The skills it can load, by name and what each is for"),
  memory: () => tr("Instruction files, memory and reminders"),
  files: () => tr("Files it read, and images"),
  results: () => tr("What the other tools it called returned"),
  chat: () => tr("What you and the model said, turn by turn"),
};
export const partName = (k: PromptKind) => PART_NAMES[k]();

/** What an item is called: a tool's name or a file's path as it is; pansy's own names for the rest. */
export function itemName(kind: PromptKind, it: PromptItem): string {
  if (it.tag === "turn") return tr("Turn %s", it.name);
  if (it.tag === "turns") return tr("%d earlier turns", it.n ?? 0);
  if (it.tag === "more") return tr("%d more", it.n ?? 0);
  const own: Partial<Record<string, () => string>> = {
    "system:prompt": () => tr("Agent prompt"),
    "system:billing": () => tr("Billing header"),
    "system:harness": () => tr("Harness messages"),
    "memory:environment": () => tr("Environment notes"),
    "memory:reminders": () => tr("System reminders"),
    "skills:skills": () => tr("Skills"),
    "chat:compacted": () => tr("Compacted summary"),
    "files:image": () => tr("Image"),
  };
  return own[`${kind}:${it.name}`]?.() ?? it.name;
}

/** The tag beside an item where it says more than its part does. */
function tagOf(it: PromptItem): string {
  switch (it.tag) {
    case "mcp":
      return "MCP";
    case "shell":
      return "shell";
    case "read":
      return tr("read");
    case "user":
      return tr("user");
    case "project":
      return tr("project");
    case "local":
      return tr("local");
    case "reminder":
      return tr("reminder");
    default:
      return "";
  }
}

const itemKey = (kind: PromptKind, it: PromptItem) => `${kind}\u0000${it.tag ?? ""}\u0000${it.name}`;

/** What the pointer is on: a part (the legend), a tile, or an item (the contents); its tiles stay lit, the others dim. */
type Focus = { kind: PromptKind | "free" } | { tile: string } | { item: string } | null;
const lit = (f: Focus, t: Tile) => {
  if (!f) return true;
  if ("kind" in f) return t.kind === f.kind;
  if ("tile" in f) return t.key === f.tile;
  return t.kind !== "free" && t.its.some((i) => itemKey(t.kind as PromptKind, i) === f.item);
};

/** A thin bar of the window, a part each: the folded card's, and the head of each tile's tip. */
function StackBar(props: { p: Drawn; width?: number }) {
  const scale = () => Math.max(props.p.window, props.p.tokens, 1);
  return (
    <View testID="context-stack" class="h-[6px] flex-row overflow-hidden rounded-[3px] bg-border" style={{ width: props.width ?? 120 }}>
      <Each of={PARTS.map((k) => ({ k, n: props.p.parts.find((x) => x.kind === k)?.tokens ?? 0 })).filter((x) => x.n > 0)} key={(x) => x.k}>
        {(x) => <View style={{ width: `${(x().n / scale()) * 100}%`, backgroundColor: inkOf(x().k) }} />}
      </Each>
    </View>
  );
}

function Dot(props: { kind: PromptKind | "free" }) {
  return (
    <View
      class={`h-[8px] w-[8px] rounded-[2px] ${props.kind === "free" ? "border border-foreground/15 bg-foreground/5" : ""}`}
      style={props.kind === "free" ? undefined : { backgroundColor: inkOf(props.kind) }}
    />
  );
}

/** A tip's line: a name and its tokens. */
function TipRow(props: { name: string; value: string; on?: boolean }) {
  return (
    <View class="flex-row gap-3">
      <Text class={`max-w-[240px] flex-1 text-[11.5px] line-clamp-1 ${props.on ? "font-semibold text-foreground" : "text-fg-2"}`}>{props.name}</Text>
      <Text class="text-[11.5px] text-muted-foreground">{props.value}</Text>
    </View>
  );
}

/** A tile's tip: the item, its part and share; a part's or the small ones' tile, its largest items. */
function TileTip(props: { tile: Tile; p: Drawn }) {
  const t = () => props.tile;
  const share = (n: number) => pct(n / Math.max(1, props.p.tokens));
  return (
    <Tooltip class="min-w-[180px] max-w-[320px] gap-1 rounded-[8px] bg-popover px-[10px] py-[8px]" style={{ boxShadow: SHADOW.pop() }}>
      <View class="flex-row items-center gap-[6px]">
        <Dot kind={t().kind} />
        <Text class="flex-1 text-[12px] font-semibold text-foreground line-clamp-1">
          {t().kind === "free" ? tr("Free space") : t().it ? itemName(t().kind as PromptKind, t().it!) : partName(t().kind as PromptKind)}
        </Text>
        <If when={!t().it}>
          <Text class="text-[11.5px] text-muted-foreground">{t().kind === "free" ? fmtK(t().tokens) : `${fmtK(t().tokens)} · ${share(t().tokens)}`}</Text>
        </If>
      </View>
      <If when={t().it}>
        <Text class="text-[11.5px] text-muted-foreground">{`${partName(t().kind as PromptKind)} · ${tr("%s tokens", fmtK(t().it!.tokens))} · ${share(t().it!.tokens)}`}</Text>
      </If>
      <If when={!t().it && t().kind !== "free"}>
        <Each of={t().its.slice(0, 5)} key={(it) => `${it.tag ?? ""}${it.name}`}>
          {(it) => <TipRow name={itemName(t().kind as PromptKind, it())} value={fmtK(it().tokens)} />}
        </Each>
        <If when={t().its.length > 5}>
          <Text class="text-[11px] text-muted-foreground">{tr("%d more", t().its.length - 5)}</Text>
        </If>
      </If>
    </Tooltip>
  );
}

/** The free space's hatching. */
function Hatch() {
  return (
    <Canvas
      class="absolute inset-0"
      draw={(cx, size) => {
        cx.strokeStyle = token("foreground");
        cx.globalAlpha = 0.07;
        cx.lineWidth = 1;
        cx.beginPath();
        for (let d = -size.height; d < size.width; d += 6) {
          cx.moveTo(d, size.height);
          cx.lineTo(d + size.height, 0);
        }
        cx.stroke();
      }}
    />
  );
}

/** One tile of the map, a gap of 3 between tiles. */
function TileView(props: { tile: Tile; p: Drawn; focus: Focus; onFocus: (f: Focus) => void }) {
  const t = () => props.tile;
  const free = () => t().kind === "free";
  const ink = () => (free() ? token("foreground") : inkOf(t().kind as PromptKind));
  const on = () => !!props.focus && "tile" in props.focus && props.focus.tile === t().key;
  const bare = () => t().w < 54 || t().h < 30;
  const low = () => t().h < 46;
  const name = () => {
    const tl = t();
    if (tl.kind === "free") return tr("Free space");
    if (tl.it) {
      let n = itemName(tl.kind, tl.it);
      // A file by its name, where it is in the tip.
      if (tl.kind === "files" || tl.kind === "memory") n = n.slice(n.lastIndexOf("/", n.length - 2) + 1) || n;
      if ((tl.it.n ?? 1) > 1 && tl.it.tag !== "turns" && tl.it.tag !== "more") n += ` ×${tl.it.n}`;
      return n;
    }
    if (tl.key === tl.kind) return partName(tl.kind);
    // The small files together: their folder, when they share one.
    const dirs = new Set(tl.its.map((it) => it.name.slice(0, it.name.lastIndexOf("/") + 1)));
    const [dir] = dirs;
    if (tl.kind === "files" && dirs.size === 1 && dir) return dir.slice(dir.lastIndexOf("/", dir.length - 2) + 1);
    return tr("%d more", tl.its.length);
  };
  const value = () => {
    const tl = t();
    if (tl.kind === "free") return `${fmtK(tl.tokens)} · ${pct(tl.tokens / Math.max(props.p.window, props.p.tokens, 1))}`;
    if (tl.key === tl.kind) return fmtK(tl.tokens);
    return `${fmtK(tl.tokens)} · ${partName(tl.kind)}`;
  };
  return (
    <View
      testID={`context-tile-${t().kind}`}
      class={`absolute overflow-hidden rounded-[7px] border ${free() ? "border-foreground/10 bg-foreground/2" : ""} ${bare() ? "" : low() ? "px-[9px] py-[5px]" : "px-[9px] py-[7px]"}`}
      style={{
        left: t().x + 1.5,
        top: t().y + 1.5,
        width: Math.max(0, t().w - 3),
        height: Math.max(0, t().h - 3),
        ...(free() ? {} : { backgroundColor: alpha(ink(), on() ? 0.19 : 0.11), borderColor: alpha(ink(), on() ? 0.48 : 0.24) }),
        opacity: lit(props.focus, t()) ? 1 : 0.35,
      }}
      onMouseEnter={() => props.onFocus({ tile: t().key })}
      onMouseLeave={() => props.onFocus(null)}
      tooltip={<TileTip tile={t()} p={props.p} />}
    >
      <If when={free()}>
        <Hatch />
      </If>
      <If when={!bare()}>
        <Text class={`text-[11.5px] line-clamp-1 ${free() ? "font-medium text-muted-foreground" : "font-semibold"}`} style={free() ? undefined : { color: mix(ink(), token("foreground"), 0.72) }}>
          {name()}
        </Text>
        <If when={!low()}>
          <Text class="text-[10.5px] line-clamp-1" style={{ color: free() ? token("muted-foreground") : mix(ink(), token("muted-foreground"), 0.42) }}>
            {value()}
          </Text>
        </If>
      </If>
    </View>
  );
}

/** The map: the window as tiles, as wide as the card, a third as high (half, narrow). */
function ContextMap(props: { p: Drawn; focus: Focus; onFocus: (f: Focus) => void }) {
  const wide = signal(0);
  const high = () => Math.round(Math.min(232, Math.max(132, wide() / (wide() <= 520 ? 2 : 3.1))));
  const tiles = () => (wide() > 0 ? tilesOf(props.p, props.p.window, wide(), high()) : []);
  return (
    <View testID="context-map" class="relative" style={{ height: high() }} onLayout={(e) => wide.set(e.bounds.size.width)}>
      <Each of={tiles()} key={(t) => t.key}>
        {(t) => <TileView tile={t()} p={props.p} focus={props.focus} onFocus={props.onFocus} />}
      </Each>
    </View>
  );
}

/** The session's prompts against the window, this request's marked; a point pressed puts its request on the stage. */
function Spark(props: { series: Point[]; at: number; window: number; ink: string }) {
  let wide = 0;
  const H = 34;
  const pad = 3;
  const peak = () => Math.max(...props.series.map((p) => p.tokens), 1);
  const top = () => (props.window && peak() > props.window * 0.6 ? Math.max(props.window, peak()) : peak() * 1.2);
  const x = (i: number, W: number) => (props.series.length < 2 ? W / 2 : pad + (i / (props.series.length - 1)) * (W - pad * 2));
  const y = (n: number) => H - pad - (n / top()) * (H - pad * 2);
  return (
    <Canvas
      testID="context-spark"
      class="flex-1"
      style={{ height: H, minWidth: 120 }}
      onMouseUp={(e) => {
        if (!wide || !props.series.length) return;
        let best = 0;
        props.series.forEach((_, i) => {
          if (Math.abs(x(i, wide) - e.position.x) < Math.abs(x(best, wide) - e.position.x)) best = i;
        });
        routePicked.set(props.series[best]!.id);
      }}
      draw={(cx, size) => {
        wide = size.width;
        const W = size.width;
        if (props.window && props.window <= top()) {
          cx.strokeStyle = token("muted-foreground");
          cx.globalAlpha = 0.5;
          cx.lineWidth = 1;
          cx.setLineDash([3, 3]);
          cx.beginPath();
          cx.moveTo(0, y(props.window));
          cx.lineTo(W, y(props.window));
          cx.stroke();
          cx.setLineDash([]);
        }
        const pts = props.series.map((p, i) => [x(i, W), y(p.tokens)] as const);
        if (pts.length > 1) {
          cx.globalAlpha = 0.14;
          cx.fillStyle = props.ink;
          cx.beginPath();
          cx.moveTo(pts[0]![0], H);
          for (const [px, py] of pts) cx.lineTo(px, py);
          cx.lineTo(pts[pts.length - 1]![0], H);
          cx.closePath();
          cx.fill();
          cx.globalAlpha = 1;
          cx.strokeStyle = props.ink;
          cx.lineWidth = 1.5;
          cx.lineJoin = "round";
          cx.beginPath();
          pts.forEach(([px, py], i) => (i ? cx.lineTo(px, py) : cx.moveTo(px, py)));
          cx.stroke();
        }
        const i = props.series.findIndex((p) => p.id === props.at);
        if (i >= 0) {
          const [px, py] = pts[i]!;
          cx.globalAlpha = 1;
          cx.fillStyle = token("card");
          cx.strokeStyle = props.ink;
          cx.lineWidth = 1.6;
          cx.beginPath();
          cx.arc(px, py, 3.2, 0, Math.PI * 2);
          cx.fill();
          cx.stroke();
        }
      }}
    />
  );
}

function Stat(props: { label: string; badge?: JSX.Element; big: string; unit: string; sub: string; bar?: number; dim?: boolean; testID?: string }) {
  return (
    <View class={`min-w-0 flex-1 gap-[3px] ${props.dim ? "opacity-60" : ""}`}>
      <View class="flex-row items-center gap-[6px]">
        <Text class="text-[11.5px] text-muted-foreground">{props.label}</Text>
        {props.badge}
      </View>
      <View class="flex-row items-baseline gap-[5px]">
        <Text testID={props.testID} class="text-[22px] font-semibold text-foreground">
          {props.big}
        </Text>
        <Text class="text-[12px] text-muted-foreground">{props.unit}</Text>
      </View>
      <If when={props.bar !== undefined}>
        <View class="h-[4px] overflow-hidden rounded-[2px] bg-border">
          <View class="h-full bg-success" style={{ width: `${(props.bar ?? 0) * 100}%` }} />
        </View>
      </If>
      <Text class="text-[11.5px] text-fg-2 line-clamp-1">{props.sub}</Text>
    </View>
  );
}

const HEALTH = {
  ok: { word: () => tr("Healthy"), bg: "bg-success-soft", ink: "text-success" },
  warn: { word: () => tr("Filling up"), bg: "bg-warning-soft", ink: "text-warning" },
  bad: { word: () => tr("Near full"), bg: "bg-destructive-soft", ink: "text-destructive" },
};

/** How full the window is, and how much of it the vendor read from its cache. */
function Top(props: { r: Route; p: Drawn; series: Point[] }) {
  const w = () => props.p.window;
  const full = () => (w() ? props.p.tokens / w() : 0);
  const sub = () => {
    if (!w()) return tr("the model's window isn't known");
    const free = Math.max(0, w() - props.p.tokens);
    const turns = headroom(props.series, free);
    return `${tr("%s full", pct(full()))} · ${turns === null ? tr("%s free", fmtK(free)) : turns >= 999 ? tr("plenty of headroom") : tr("~%d requests of headroom", turns)}`;
  };
  const cache = () => (props.p.counted ? cacheOf(props.r) : null);
  return (
    <View class="flex-row gap-6">
      <Stat
        label={tr("Context used")}
        testID="context-used"
        badge={
          <If when={w()}>
            <View testID="context-health" class={`rounded-full px-[6px] ${HEALTH[health(full())].bg}`}>
              <Text class={`text-[10.5px] font-semibold ${HEALTH[health(full())].ink}`}>{HEALTH[health(full())].word()}</Text>
            </View>
          </If>
        }
        big={fmtK(props.p.tokens)}
        unit={w() ? `/ ${fmtK(w())} ${tr("tokens")}` : tr("tokens")}
        sub={sub()}
      />
      <Stat
        label={tr("Prompt cache")}
        testID="context-cache"
        dim={!cache()}
        big={cache() ? pct(cache()!.read / cache()!.total) : "—"}
        unit={tr("hit")}
        bar={cache() ? cache()!.read / cache()!.total : 0}
        sub={cache() ? tr("%s cached · %s new", fmtK(cache()!.read), fmtK(cache()!.total - cache()!.read)) : props.r.done ? tr("The vendor didn't say") : tr("Read once the vendor answers")}
      />
    </View>
  );
}

/** The parts by color and tokens, the free space after them; one pointed at lights its tiles. */
function Legend(props: { p: Drawn; onFocus: (f: Focus) => void }) {
  return (
    <View class="flex-row flex-wrap gap-x-[14px] gap-y-[4px]">
      <Each of={PARTS.filter((k) => (props.p.parts.find((x) => x.kind === k)?.tokens ?? 0) > 0)} key={(k) => k}>
        {(k) => (
          <View
            testID={`context-legend-${k()}`}
            class="flex-row items-center gap-[5px]"
            onMouseEnter={() => props.onFocus({ kind: k() })}
            onMouseLeave={() => props.onFocus(null)}
            tooltip={
              <Tooltip class="max-w-[280px] rounded-[7px] bg-popover px-[9px] py-[6px]" style={{ boxShadow: SHADOW.pop() }}>
                <Text class="text-[11.5px] text-foreground">{PART_NOTES[k()]()}</Text>
              </Tooltip>
            }
          >
            <Dot kind={k()} />
            <Text class="text-[12px] text-fg-2">{partName(k())}</Text>
            <Text class="text-[12px] text-muted-foreground">{fmtK(props.p.parts.find((x) => x.kind === k())?.tokens ?? 0)}</Text>
          </View>
        )}
      </Each>
      <If when={props.p.window > props.p.tokens}>
        <View class="flex-row items-center gap-[5px]" onMouseEnter={() => props.onFocus({ kind: "free" })} onMouseLeave={() => props.onFocus(null)}>
          <Dot kind="free" />
          <Text class="text-[12px] text-fg-2">{tr("Free space")}</Text>
          <Text class="text-[12px] text-muted-foreground">{fmtK(props.p.window - props.p.tokens)}</Text>
        </View>
      </If>
    </View>
  );
}

/** A file's path: its name first, where it is after it. */
function ItemLabel(props: { kind: PromptKind; it: PromptItem }) {
  const name = () => itemName(props.kind, props.it);
  const at = () => ((props.kind === "files" || props.kind === "memory") && name().includes("/") ? name().lastIndexOf("/") : -1);
  return (
    <Text class="min-w-0 shrink text-[12px] line-clamp-1">
      <Text class="font-medium text-foreground">{at() < 0 ? name() : name().slice(at() + 1)}</Text>
      <If when={at() >= 0}>
        <Text class="text-muted-foreground">{`  ${name().slice(0, at() + 1)}`}</Text>
      </If>
    </Text>
  );
}

/** What the prompt holds, item by item, largest first: all of it, or a part's. */
function Contents(props: { p: Drawn; onFocus: (f: Focus) => void }) {
  const tab = signal<"all" | PromptKind>("all");
  const more = signal(false);
  const tabs = () => [{ value: "all" as const, label: tr("All") }, ...PARTS.filter((k) => (props.p.parts.find((x) => x.kind === k)?.tokens ?? 0) > 0).map((k) => ({ value: k, label: partName(k) }))];
  const shown = () => (tabs().some((t) => t.value === tab()) ? tab() : "all");
  const rows = () =>
    props.p.parts
      .filter((x) => shown() === "all" || x.kind === shown())
      .flatMap((x) => x.items.map((it) => ({ it, kind: x.kind })))
      .sort((a, b) => b.it.tokens - a.it.tokens);
  const most = () => Math.max(1, ...rows().map((x) => x.it.tokens));
  const listedRows = () => (more() ? rows() : rows().slice(0, 8));
  return (
    <View class="gap-[6px]">
      <Text class="text-[11.5px] text-muted-foreground">{tr("Contents")}</Text>
      <Scroll horizontal class="flex-row">
        <Segs
          small
          testID="context-tab"
          options={tabs()}
          value={shown()}
          onChange={(v) => {
            tab.set(v);
            more.set(false);
          }}
        />
      </Scroll>
      <View testID="context-rows" class="gap-[2px]">
        <Each of={listedRows()} key={(x) => itemKey(x.kind, x.it)}>
          {(x) => (
            <View
              class="h-[24px] flex-row items-center gap-[8px] rounded-[6px] px-[6px] hover:bg-pill"
              onMouseEnter={() => props.onFocus({ item: itemKey(x().kind, x().it) })}
              onMouseLeave={() => props.onFocus(null)}
            >
              <Dot kind={x().kind} />
              <ItemLabel kind={x().kind} it={x().it} />
              <If when={(x().it.n ?? 1) > 1 && x().it.tag !== "turns" && x().it.tag !== "more"}>
                <Text class="text-[11px] text-muted-foreground">{`×${x().it.n}`}</Text>
              </If>
              <If when={tagOf(x().it)}>
                <View class="rounded-[4px] bg-card-2 px-[4px]">
                  <Text class="font-mono text-[10.5px] text-fg-2">{tagOf(x().it)}</Text>
                </View>
              </If>
              <View class="flex-1" />
              <View class="h-[4px] w-[80px] overflow-hidden rounded-[2px] bg-border">
                <View class="h-full" style={{ width: `${Math.max(2, (x().it.tokens / most()) * 100)}%`, backgroundColor: inkOf(x().kind) }} />
              </View>
              <Text class="w-[44px] text-right text-[12px] text-fg-2">{fmtK(x().it.tokens)}</Text>
            </View>
          )}
        </Each>
        <If when={!rows().length}>
          <Text class="px-[6px] text-[12px] text-muted-foreground">{tr("Nothing in it")}</Text>
        </If>
        <If when={rows().length > 8}>
          <View class="flex-row">
            <TextButton testID="context-more" label={more() ? tr("Show fewer") : tr("Show all %d", rows().length)} onPress={() => more.set(!more())} />
          </View>
        </If>
      </View>
    </View>
  );
}

const STATES = {
  live: { word: () => tr("Live"), tip: () => tr("The request is on its way: the prompt is estimated from what the agent sent"), dot: "bg-success" },
  counted: { word: () => tr("Counted"), tip: () => tr("As many tokens as the vendor counted; the parts are measured from the request and scaled to it"), dot: "bg-foreground" },
  est: { word: () => tr("Estimated"), tip: () => tr("Estimated from what the agent sent: the vendor didn't say how many tokens it read"), dot: "bg-muted-foreground" },
};

/** The head: the title (pressed, it folds the card), folded how full, whose request it is and the model, and how it was counted. */
function Head(props: { r: Route; p: Drawn }) {
  const state = () => STATES[!props.r.done ? "live" : props.p.counted ? "counted" : "est"];
  const model = () => {
    const last = props.r.tries[props.r.tries.length - 1];
    return (last ? props.r.lineup[last.slot]?.model : undefined) ?? props.r.lineup[0]?.model ?? props.r.requested;
  };
  return (
    <View class="min-h-[24px] flex-row items-center gap-[10px]">
      <View role="button" testID="context-fold" onPress={() => contextShut.set(!contextShut())} class="flex-row items-center gap-[5px]">
        <Icon name={contextShut() ? "chevron-right" : "chevron-down"} color="muted-foreground" size={12} />
        <Text class="text-[12px] font-semibold text-foreground">{tr("Context window")}</Text>
      </View>
      <If when={contextShut()}>
        <Text testID="context-short" class="text-[12px] text-fg-2">
          {props.p.window ? `${fmtK(props.p.tokens)} / ${fmtK(props.p.window)} · ${pct(props.p.tokens / props.p.window)}` : fmtK(props.p.tokens)}
        </Text>
        <StackBar p={props.p} />
      </If>
      <View class="flex-1" />
      <View class="min-w-0 shrink flex-row items-center gap-[6px]">
        <Logo name={agentLogo(props.r.agent)} size={14} />
        <Text class="text-[12px] text-fg-2 line-clamp-1">{agentName(props.r.agent)}</Text>
        <Text class="text-[12px] text-muted-foreground">›</Text>
        <View class="shrink rounded-[5px] bg-card-2 px-[5px]">
          <Text class="font-mono text-[11px] text-fg-2 line-clamp-1">{model()}</Text>
        </View>
      </View>
      <View
        testID="context-state"
        class="flex-row items-center gap-[5px] rounded-full bg-pill px-[7px] py-px"
        tooltip={
          <Tooltip class="max-w-[300px] rounded-[7px] bg-popover px-[9px] py-[6px]" style={{ boxShadow: SHADOW.pop() }}>
            <Text class="text-[11.5px] text-foreground">{state().tip()}</Text>
          </Tooltip>
        }
      >
        <View class={`h-[6px] w-[6px] rounded-full ${state().dot}`} />
        <Text class="text-[11px] text-fg-2">{state().word()}</Text>
      </View>
    </View>
  );
}

/** The card, for the request on the stage; none while its prompt is not read (a day's requests keep none). */
export function ContextCard(props: { route?: Route }) {
  const focus = signal<Focus>(null);
  const r = () => props.route;
  const p = () => (r() ? drawnOf(r()!) : null);
  const series = () => (r() ? seriesOf(r()!, listed()) : []);
  return (
    <If when={r() && p()}>
      <View testID="routing-context" class="gap-[12px] rounded-[14px] border border-border bg-card px-4 pb-3 pt-[12px]">
        <Head r={r()!} p={p()!} />
        <If when={!contextShut()}>
          <Top r={r()!} p={p()!} series={series()} />
          <ContextMap p={p()!} focus={focus()} onFocus={(f) => focus.set(f)} />
          <Legend p={p()!} onFocus={(f) => focus.set(f)} />
          <Contents p={p()!} onFocus={(f) => focus.set(f)} />
          <View class="flex-row items-center gap-3 border-t border-border pt-[10px]">
            <Text class="font-mono text-[11px] text-muted-foreground">{`${agentName(r()!.agent)} › #${r()!.id}`}</Text>
            <If when={p()!.turns}>
              <Text class="text-[11px] text-muted-foreground">{trn(p()!.turns ?? 0, "1 turn", "%d turns", p()!.turns ?? 0)}</Text>
            </If>
            <View class="flex-1" />
            <If when={series().length > 1}>
              <View class="min-w-0 max-w-[360px] flex-1 flex-row items-center gap-[8px]">
                <Text class="text-[11.5px] text-muted-foreground">{tr("This session")}</Text>
                <Spark series={series()} at={r()!.id} window={p()!.window} ink={inkOf("chat")} />
                <Text class="text-[11.5px] text-muted-foreground">{trn(series().length, "1 request", "%d requests", series().length)}</Text>
              </View>
            </If>
          </View>
        </If>
      </View>
    </If>
  );
}
