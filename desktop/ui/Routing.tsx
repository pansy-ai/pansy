// Routing, as magpie plays it: the request the gateway routed last (or the
// one picked) on a stage — the agent, pansy, and the accounts routing lined
// up for it, the one that answered lit; while it is out, pansy's logo turns
// and data flows along its wires — then what happened in words and where
// the time went, the requests (live, or a day's) and how each account fared
// today. Kept in memory, and never a prompt or a reply.
import { Canvas, computed, createAnimatedValue, effect, reduceMotion, Text, token, View, type AnimatedValue, type CanvasRenderingContext2D } from "nsh";
import { tr } from "./i18n";
import { gateway, go, hideAccounts, routePicked, routes, type Route, type RouteSlot } from "./model";
import { agentLogo, Icon, Label, Logo, Page, providerLogo, TextButton, type Color } from "./parts";
import { RoutingAccounts, RoutingRequests } from "./RoutingLists";
import {
  agentName,
  captionOf,
  dur,
  failed,
  hueOf,
  listed,
  onStage,
  providerById,
  slotState,
  storyOf,
  timingOf,
  topOf,
  tryOf,
  wayChip,
  who,
  type SlotKind,
  type Step,
  type Timing,
} from "./routeStory";

/** A row of the lineup, the gap between rows, and the most rows shown. */
const ROW = 58;
const GAP = 7;
const MOST = 8;
/** How wide the wires between the cards run. */
const WIRE = 56;
/** One turn of pansy's logo while a request is out, and one pass of the data along a wire (ms). */
const TURN = 2400;
const FLOW = 1000;

/** Which way data flows while a request is out: to the account, or its answer back. */
export type Way = "out" | "back";

/** Out while pansy picks an account or waits on one, back while the answer streams; none once done. */
export function wayOf(r: Route | undefined): Way | null {
  if (!r || r.done) return null;
  const last = r.tries[r.tries.length - 1];
  return last && last.status > 0 && last.status < 400 ? "back" : "out";
}

function Head() {
  const live = () => routes().some((r) => !r.done);
  return (
    <View class="min-h-[26px] flex-row items-center px-[2px]">
      <Label text={tr("Routing")} />
      <View
        testID="routing-live"
        class="ml-[7px] h-[6px] w-[6px] rounded-full bg-success"
        style={live() ? { boxShadow: `0 0 0 3px ${token("success-soft")}` } : undefined}
      >
        <If when={live()}>
          <View class="absolute inset-0 rounded-full bg-success motion-safe:animate-ping" />
        </If>
      </View>
      <Text class="ml-2 text-[11px] text-faint">{tr("live, as the gateway decides")}</Text>
      <View class="flex-1" />
      <TextButton label={tr("Settings ›")} testID="routing-settings" onPress={() => go("groups")} />
      <TextButton
        label={hideAccounts() ? tr("Show accounts") : tr("Hide accounts")}
        icon={hideAccounts() ? "eye-s" : "eye-off-s"}
        testID="routing-hide"
        onPress={() => hideAccounts.set(!hideAccounts())}
      />
    </View>
  );
}

/** The agent that asked: its logo, its name by its color, the model it asked for. */
function Source(props: { route?: Route }) {
  const r = () => props.route;
  return (
    <View class="w-[124px] items-center gap-[2px] rounded-[12px] border border-border bg-card px-2 pb-[10px] pt-3">
      <View class="mb-[3px]">
        <Logo name={r() ? agentLogo(r()!.agent) : "generic"} size={26} />
      </View>
      <View class="max-w-full flex-row items-center gap-[6px]">
        <If when={r()}>
          <View class="h-[7px] w-[7px] rounded-full" style={{ backgroundColor: hueOf(r()!.agent) }} />
        </If>
        <Text testID="routing-agent" class="shrink text-[13px] font-semibold text-foreground line-clamp-1">
          {r() ? agentName(r()!.agent) : tr("your agent")}
        </Text>
      </View>
      <If when={r()}>
        <Text class="max-w-full text-[11px] text-muted-foreground line-clamp-1">{r()!.requested}</Text>
      </If>
    </View>
  );
}

/** Turns the logo on from where it stands, or winds it down, as fast at first, to upright. */
export function spin(turn: AnimatedValue, on: boolean) {
  void turn.stop().then(({ value }) => {
    const at = value % 360;
    if (on) {
      void turn.keyframes([at, at + 360], { duration: TURN, easing: "linear", iterations: "infinite", frameRate: 60 });
    } else if (at > 0) {
      // Ease-out leaves at 1.7 times its average speed.
      void turn.timing(value - at + 360, { duration: ((360 - at) / 360) * TURN * 1.7, easing: "easeOut", frameRate: 60 });
    }
  });
}

/** pansy in the middle: where it listens, and how the request routes; its logo turns while the request is out. */
function Hub(props: { route?: Route }) {
  const chip = () => (props.route ? wayChip(props.route) : null);
  const where = () => gateway()?.url.replace(/^https?:\/\//, "") ?? "";
  const turn = createAnimatedValue(0);
  const turning = computed(() => !reduceMotion() && wayOf(props.route) !== null);
  effect(() => spin(turn, turning()));
  return (
    <View class="w-[124px] items-center gap-[2px] rounded-[12px] border border-foreground bg-card px-2 pb-[10px] pt-3" style={{ boxShadow: `0 0 0 4px ${token("card-2")}` }}>
      <View testID="routing-hub-logo" class="mb-[3px]" style={{ rotate: turn }}>
        <Icon name="pansy" color="foreground" size={26} />
      </View>
      <Text class="text-[13px] font-semibold text-foreground">pansy</Text>
      <Text class="max-w-full text-[11px] text-muted-foreground line-clamp-1">{where()}</Text>
      <If when={chip()}>
        <View role="button" testID="routing-way" onPress={() => go("groups")} class="mt-[5px] rounded-full bg-foreground px-2 py-px">
          <Text class="text-[10.5px] font-semibold text-card">{`${chip() ?? ""} ›`}</Text>
        </View>
      </If>
    </View>
  );
}

const DOT: Record<SlotKind, { box: string; ring?: boolean }> = {
  idle: { box: "border-muted-foreground" },
  spare: { box: "border-muted-foreground" },
  rest: { box: "border-warning bg-warning" },
  asking: { box: "border-muted-foreground bg-muted-foreground", ring: true },
  on: { box: "border-foreground bg-foreground" },
  failed: { box: "border-destructive bg-destructive" },
};

/** One account lined up: its dot, who it is and the model, and what became of it. */
function SlotRow(props: { route: Route; slot: RouteSlot; at: number }) {
  const st = () => slotState(props.route, props.at);
  const kind = () => st().kind;
  const dim = () => kind() === "rest" || kind() === "spare";
  const line = (): Color => (kind() === "on" ? "foreground" : kind() === "failed" || kind() === "rest" ? "destructive" : "muted-foreground");
  return (
    <View
      testID={`route-slot-${props.at}`}
      class={`justify-center gap-[6px] rounded-[10px] border bg-card px-3 ${kind() === "on" ? "border-foreground" : "border-border"} ${dim() ? "border-dashed" : ""}`}
      style={{ height: ROW }}
    >
      <View class={`flex-row items-center gap-[10px] ${dim() ? "opacity-50" : ""}`}>
        <View
          class={`h-[8px] w-[8px] rounded-full border-[1.5px] ${DOT[kind()].box}`}
          style={DOT[kind()].ring ? { boxShadow: `0 0 0 3px ${token("pill")}` } : undefined}
        >
          <If when={DOT[kind()].ring}>
            <View class="absolute inset-0 rounded-full bg-muted-foreground motion-safe:animate-ping" />
          </If>
        </View>
        <View class="min-w-0 flex-1 flex-row items-center">
          <Logo name={providerLogo(providerById(props.slot.provider)?.preset)} size={16} />
          <Text class="ml-[7px] shrink text-[13px] font-medium text-foreground line-clamp-1">{who(props.slot)}</Text>
          <View class="ml-[7px] shrink rounded-[5px] bg-card-2 px-[5px]">
            <Text class="font-mono text-[11px] text-fg-2 line-clamp-1">{props.slot.model}</Text>
          </View>
        </View>
      </View>
      <Text testID={`route-slot-${props.at}-said`} class={`pl-[18px] text-[12px] text-${line()} line-clamp-1 ${kind() === "rest" ? "opacity-85" : ""}`}>
        {st().text}
      </Text>
    </View>
  );
}

/** A wire across the stage, from (0, `from`) to (its width, `to`), level at both ends; `way` while data flows on it. */
export interface Wire {
  from: number;
  to: number;
  color: string;
  alpha?: number;
  dashed?: boolean;
  way?: Way | null;
}

/** Where along a wire `wide` across a share `s` (0..1) of the way is, at 13 points: what packets ride between. */
const STEPS = Array.from({ length: 13 }, (_, i) => i / 12);
function points(w: Wire, wide: number): { x: number[]; y: number[] } {
  const at = STEPS.map((s) => (w.way === "back" ? 1 - s : s)).map((s) => {
    const u = 1 - s;
    return { x: 1.5 * wide * u * s + wide * s ** 3, y: w.from * (u ** 3 + 3 * u * u * s) + w.to * (3 * u * s * s + s ** 3) };
  });
  return { x: at.map((p) => p.x), y: at.map((p) => p.y) };
}

/**
 * Two packets half a pass apart on a wire data flows on, fading in and out
 * at its ends: a request filled with its agent's color, an answer ringed
 * with it. Each glows by a faint halo, not a blurred shadow: drawn again
 * every frame, the blur cost more than all the page's other motion.
 */
function packets(cx: CanvasRenderingContext2D, w: Wire, pass: AnimatedValue, wide: number) {
  const { x, y } = points(w, wide);
  for (const p of [pass, pass.add(0.5).mod(1)]) {
    const fade = p.interpolate({ inputRange: [0, 0.15, 0.85, 1], outputRange: [0, 1, 1, 0] });
    const px = p.interpolate({ inputRange: STEPS, outputRange: x });
    const py = p.interpolate({ inputRange: STEPS, outputRange: y });
    cx.fillStyle = w.color;
    cx.globalAlpha = fade.mul(0.25);
    cx.beginPath();
    cx.arc(px, py, 5.5, 0, Math.PI * 2);
    cx.fill();
    cx.globalAlpha = fade;
    cx.beginPath();
    cx.arc(px, py, 2.75, 0, Math.PI * 2);
    cx.fillStyle = w.way === "out" ? w.color : token("card");
    cx.fill();
    if (w.way === "back") cx.stroke();
  }
}

/** The wires between two cards, `width` apart (WIRE); where data flows, their dashes move with `pass` (0..1, over and over) and packets ride them. */
export function Wires(props: { wires: Wire[]; height: number; pass: AnimatedValue; width?: number }) {
  const wide = () => props.width ?? WIRE;
  return (
    <Canvas
      style={{ width: wide(), height: props.height }}
      draw={(cx) => {
        const x = wide();
        cx.lineWidth = 1.5;
        for (const w of props.wires) {
          cx.globalAlpha = w.alpha ?? 1;
          cx.strokeStyle = w.color;
          cx.setLineDash(w.way ? [3, 5] : w.dashed ? [2, 6] : []);
          cx.lineDashOffset = w.way ? props.pass.mul(w.way === "out" ? -16 : 16) : 0;
          cx.beginPath();
          cx.moveTo(0, w.from);
          cx.bezierCurveTo(x / 2, w.from, x / 2, w.to, x, w.to);
          cx.stroke();
        }
        cx.setLineDash([]);
        for (const w of props.wires) {
          if (!w.way) continue;
          cx.strokeStyle = w.color;
          packets(cx, w, props.pass, x);
        }
      }}
    />
  );
}

/** The middle of the card: the agent, pansy, and the accounts, wired; data flows on the wires the request is out on. */
function Stage(props: { route?: Route }) {
  const r = () => props.route;
  const all = () => r()?.lineup ?? [];
  const rows = () => (all().length > MOST ? all().slice(0, MOST - 1) : all());
  const more = () => all().length - rows().length;
  const count = () => Math.max(1, rows().length + (more() > 0 ? 1 : 0));
  const listH = () => count() * ROW + (count() - 1) * GAP;
  const h = () => Math.max(listH(), 114);
  const ink = () => (r() ? hueOf(r()!.agent) : token("border"));
  // With less motion asked for, the wires the request is out on are lit, not flowing.
  const way = () => (reduceMotion() ? null : wayOf(r()));
  const pass = createAnimatedValue(0);
  const flowing = computed(() => way() !== null);
  effect(() => {
    if (!flowing()) return;
    void pass.keyframes([0, 1], { duration: FLOW, easing: "linear", iterations: "infinite", frameRate: 60 });
    return () => void pass.stop();
  });
  const into = (): Wire[] => [{ from: h() / 2, to: h() / 2, color: ink(), way: way() }];
  const out = (): Wire[] => {
    const cy = h() / 2;
    const top = (h() - listH()) / 2;
    const wires: Wire[] = [];
    for (let i = 0; i < count(); i++) {
      const wire: Wire = { from: cy, to: top + i * (ROW + GAP) + ROW / 2, color: token("border") };
      const route = r();
      if (route && i < rows().length) {
        const t = tryOf(route, i);
        const kind = slotState(route, i).kind;
        if (kind === "on" || kind === "asking") {
          wire.color = ink();
          if (kind === "asking") wire.way = way();
        } else if (t && t.status >= 400) {
          wire.color = token("destructive");
          wire.alpha = 0.55;
        } else if (kind === "rest" || kind === "spare") {
          wire.dashed = true;
          wire.alpha = 0.45;
        }
      }
      wires.push(wire);
    }
    return wires;
  };
  return (
    <View testID="routing-stage" class="flex-row items-center px-[2px] py-[22px]">
      <Source route={r()} />
      <Wires wires={into()} height={h()} pass={pass} />
      <Hub route={r()} />
      <Wires wires={out()} height={h()} pass={pass} />
      <View class="min-w-0 flex-1" style={{ gap: GAP }}>
        <If
          when={r() && all().length > 0}
          else={
            <View class="items-center justify-center rounded-[10px] border border-dashed border-border px-3" style={{ height: ROW }}>
              <Text class={`text-[13px] line-clamp-2 ${r() ? "text-destructive" : "text-muted-foreground"}`}>{r() ? tr("No account could take it") : tr("No request yet")}</Text>
            </View>
          }
        >
          <Each of={rows()} key={(s, i) => `${i}${s.provider}${s.account ?? ""}${s.model}`}>
            {(s, i) => <SlotRow route={r()!} slot={s()} at={i()} />}
          </Each>
          <If when={more() > 0}>
            <View class="items-center justify-center rounded-[10px] border border-dashed border-border" style={{ height: ROW }}>
              <Text class="text-[12px] text-muted-foreground">{tr("%d more lined up after these", more())}</Text>
            </View>
          </If>
        </If>
      </View>
    </View>
  );
}

function Stat(props: { n: number; label: string; testID?: string }) {
  return (
    <View>
      <Text testID={props.testID} class="text-[19px] font-semibold text-foreground">
        {String(props.n)}
      </Text>
      <Text class="text-[11px] text-muted-foreground">{props.label}</Text>
    </View>
  );
}

/** Under the stage: how the request ended, and the counts of those listed. */
function Foot(props: { route?: Route }) {
  const list = () => listed();
  return (
    <View class="flex-row items-center gap-5 border-t border-border pt-[10px]">
      <View class="min-h-[38px] flex-1 justify-center">
        <Text testID="routing-caption" class="text-[13px] leading-[19px] text-foreground">
          {captionOf(props.route)}
        </Text>
      </View>
      <View class="flex-row gap-[18px]">
        <Stat n={list().length} label={tr("requests")} testID="routing-requests" />
        <Stat n={list().filter((r) => r.tries.length > 1).length} label={tr("rerouted")} />
        <Stat n={list().filter(failed).length} label={tr("errors your agent saw")} />
      </View>
    </View>
  );
}

const BULLET: Record<Step["kind"], string> = {
  plain: "h-[6px] w-[6px] bg-muted-foreground",
  why: "h-[6px] w-[6px] bg-foreground",
  aside: "h-[5px] w-[5px] border border-muted-foreground",
  ok: "h-[6px] w-[6px] bg-success",
  bad: "h-[6px] w-[6px] bg-destructive",
  wait: "h-[6px] w-[6px] border border-muted-foreground",
};

function StepRow(props: { step: Step }) {
  const k = () => props.step.kind;
  return (
    <View class="flex-row">
      <View class="w-[20px] pl-[5px] pt-[6px]">
        <View class={`rounded-full ${BULLET[k()]}`} />
      </View>
      <View class="min-w-0 flex-1">
        <Text class={`text-[12.5px] leading-[18px] ${k() === "why" ? "text-foreground" : k() === "aside" ? "text-muted-foreground" : "text-fg-2"}`}>{props.step.text}</Text>
      </View>
    </View>
  );
}

/** A swatch and what it stands for: "■ pansy 3.6 ms". */
function Swatch(props: { color: string; name: string; value: string; small?: boolean }) {
  return (
    <View class="flex-row items-center gap-[5px]">
      <View class="h-[7px] w-[7px] rounded-[2px]" style={{ backgroundColor: props.color }} />
      <Text class={props.small ? "text-[11.5px] text-muted-foreground" : "text-[12.5px] text-fg-2"}>{props.name}</Text>
      <Text class={props.small ? "text-[11.5px] text-fg-2" : "text-[12.5px] text-fg-2"}>{props.value}</Text>
    </View>
  );
}

/** Where the time went: pansy's, earlier tries', the vendor's, on one bar. */
function TimeSpent(props: { t: Timing }) {
  const own = () => token("primary");
  const tries = () => token("warning");
  const vendor = () => token("fg-2");
  const parts = () =>
    [
      { name: tr("Masking"), us: props.t.masking, color: own() },
      { name: tr("In pansy"), us: props.t.pansy - props.t.masking, color: own() },
      { name: tr("Earlier tries"), us: props.t.before, color: tries() },
      { name: tr("Vendor (first byte)"), us: props.t.head, color: vendor() },
      { name: tr("First token"), us: props.t.firstToken, color: vendor() },
      { name: tr("Streaming"), us: props.t.streaming, color: vendor() },
    ].filter((p, i) => p.us > 0 || i === 1 || i === 3);
  return (
    <View testID="routing-time" class="flex-row">
      <View class="w-[20px] pl-[5px] pt-[6px]">
        <View class={`rounded-full ${BULLET.aside}`} />
      </View>
      <View class="min-w-0 flex-1 gap-[5px]" style={{ maxWidth: 560 }}>
        <View class="flex-row flex-wrap items-center gap-x-3 gap-y-1">
          <Text class="text-[12.5px] text-fg-2">{tr("Where the time went")}</Text>
          <Swatch color={own()} name="pansy" value={dur(props.t.pansy)} />
          <If when={props.t.before > 0}>
            <Swatch color={tries()} name={tr("earlier tries")} value={dur(props.t.before)} />
          </If>
          <Swatch color={vendor()} name={tr("vendor")} value={dur(props.t.vendor)} />
        </View>
        <View class="h-[6px] flex-row gap-px overflow-hidden rounded-[3px] bg-border">
          <Each of={parts()} key={(p) => p.name}>
            {(p) => <View style={{ flexGrow: Math.max(p().us, 1), minWidth: 2, backgroundColor: p().color }} />}
          </Each>
        </View>
        <View class="flex-row flex-wrap gap-x-[14px] gap-y-[2px]">
          <Each of={parts()} key={(p) => p.name}>
            {(p) => <Swatch small color={p().color} name={p().name} value={dur(p().us)} />}
          </Each>
        </View>
      </View>
    </View>
  );
}

/** How the request was routed, step by step. */
function Log(props: { route: Route }) {
  const steps = () => storyOf(props.route);
  const time = () => timingOf(props.route);
  return (
    <View testID="routing-log" class="mt-[10px] border-t border-border pt-[10px]">
      <View class="flex-row items-center">
        <Text class="flex-1 text-[12px] font-semibold text-foreground">
          {routePicked() === null ? tr("How the last request was routed") : tr("How this request was routed")}
        </Text>
        <TextButton label={tr("View usage")} testID="routing-usage" onPress={() => go("usage")} />
      </View>
      <View class="mt-[6px] gap-[5px]">
        <Each of={steps()} key={(s, i) => `${i}${s.kind}${s.text}`}>
          {(s) => <StepRow step={s()} />}
        </Each>
        <If when={time()}>
          <TimeSpent t={time()!} />
        </If>
      </View>
    </View>
  );
}

/** The card: what was asked for and how it routes, the stage, the end, the story. */
function Card() {
  const r = () => onStage();
  const top = () => (r() ? topOf(r()!) : null);
  const elsewhere = () => gateway()?.running === true && gateway()?.own === false;
  return (
    <View testID="routing-card" class="rounded-[14px] border border-border bg-card pb-3 pl-4 pr-4 pt-[14px]">
      <If
        when={!elsewhere()}
        else={
          <Text class="py-[18px] text-center text-[13px] text-muted-foreground">
            {tr("Another pansy (pansy serve) answers on this port, so the requests it routes play in its own place, not here.")}
          </Text>
        }
      >
        <View class="gap-[3px]">
          <Text testID="routing-what" class="text-[13px] text-muted-foreground">
            <Text class="font-semibold text-foreground">{top()?.name ?? tr("Waiting for a request")}</Text>
            {top()?.rest ?? ""}
          </Text>
          <Text class="text-[12.5px] leading-[18px] text-fg-2">
            {top()?.mode ?? tr("Send one from any agent connected to pansy and it plays here as it happens: who routing put first and why, each try, and what each answered.")}
          </Text>
        </View>
        <Stage route={r()} />
        <Foot route={r()} />
        <If when={r()}>
          <Log route={r()!} />
        </If>
      </If>
    </View>
  );
}

export function RoutingPage() {
  return (
    <Page testID="page-routing">
      <Head />
      <Card />
      <RoutingRequests />
      <RoutingAccounts />
    </Page>
  );
}
