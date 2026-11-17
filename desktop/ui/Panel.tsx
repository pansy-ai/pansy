// The quick panel by the tray icon, as magpie's: a click on the icon drops
// it under the bar where the pointer is (Hyprland; elsewhere the system
// places it), and a click outside it, Escape or the icon again puts it
// away. Its head has Update once a newer pansy is out, and the window's
// tools (look again, the main window, Settings). Its tabs: Agents, a quiet
// line each, the model each starts on in words at the right, a line opened
// in place to connect its agent and pick its models; Usage, a period's
// totals, its days and its providers; Routing, today's count, a small
// stage of who asked lately and where it went, and the latest requests,
// where each went and how fast it wrote. Its foot has the profiles (on
// Agents), what just happened, and Quit. magpie's Allowances tab shows
// subscriptions' and keys' allowances, which pansy doesn't read yet: as
// magpie's with none, it has no tab.
import { colorScheme, computed, createAnimatedValue, createMotion, effect, reduceMotion, Scroll, signal, Text, token, View, type JSX } from "nsh";
import { Popover } from "nsh/base";
import { Application, Window } from "nsh/desktop";
import { Agents, Panel as Place, Usage } from "pansy/native";
import { DisconnectDialog } from "./AgentModals";
import { HEART, WAG } from "./Brand";
import { arrange, askDisconnect, connect, disconnectIn, disconnecting, here, isHidden, kindOf, status } from "./agentState";
import { fontClass, tr, trn } from "./i18n";
import {
  act,
  choiceName,
  choices,
  fmtN,
  gateway,
  go,
  isBusy,
  loadAgents,
  loadAllowances,
  loadPage,
  loadProfiles,
  loadRouting,
  loadTrace,
  loadUsageToday,
  profiles,
  providers,
  reason,
  routePicked,
  routes,
  toast,
  toasts,
  usageDays,
  usageToday,
  type Agent,
  type Page,
  type Route,
  type RouteTry,
  type Totals,
  type UsageSummary,
} from "./model";
import { ChoiceLogo, ModelPicker } from "./Picker";
import { ProfilesBar } from "./ProfilesBar";
import { AllowanceList } from "./Allowances";
import { adding } from "./Providers";
import { agentName, cancelled, clock, dur, failed, hueOf, providerName, short, speedOf, who } from "./routeStory";
import { spin, wayOf, Wires, type Wire } from "./Routing";
import { UpdatePill } from "./Update";
import { dayLabel } from "./Usage";
import { agentLogo, Icon, IconButton, Logo, MOTION, providerLogo, Segs, SHADOW, Switch, TextButton } from "./parts";

/** The panel's title, by which Hyprland's rule finds it, and its size: magpie's 440 points wide, 220 to 560 tall. */
const TITLE = "pansy panel";
const WIDTH = 440;
const LEAST = 220;
const MOST = 560;

type Tab = "agents" | "usage" | "allowances" | "routing";
/** The tab shown, the one last picked. */
const tab = signal<Tab>("agents");
/** The agent whose line is opened, and the picker open in it. */
const opened = signal<string | null>(null);
const picking = signal<{ id: string; small: boolean } | null>(null);
/** The folded agents are shown under the others. */
const more = signal(false);
/** The profiles' pop-up over the foot is open. */
const profilesOpen = signal(false);
/** The Usage tab: its period in days, what it counts, and what it read. */
const period = signal(1);
const metric = signal<"tokens" | "calls">("tokens");
const spent = signal<UsageSummary | null>(null);

async function loadSpent() {
  try {
    spent.set(JSON.parse(await Usage.summary(period())) as UsageSummary);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** Shows tab `t`, read again. */
function show(t: Tab) {
  tab.set(t);
  profilesOpen.set(false);
  if (t === "usage") void loadSpent();
  if (t === "allowances") void loadAllowances();
  if (t === "routing") {
    loadTrace();
    void loadUsageToday();
  }
}

/** The head's refresh: what the tab shown draws, read again. */
function refresh(): Promise<unknown> {
  if (tab() === "usage") return loadSpent();
  if (tab() === "allowances") return loadAllowances(true);
  if (tab() === "routing") return Promise.all([loadRouting(), loadUsageToday()]);
  return loadPage("agents");
}

/** A card's edge, as magpie's panel draws its list, and a field's ring. */
const edge = () => (colorScheme() === "dark" ? "inset 0 0 0 1px rgba(255,255,255,0.06)" : "0 0 0 1px rgba(0,0,0,0.05), 0 1px 2px rgba(0,0,0,0.04)");
const ring = () => (colorScheme() === "dark" ? "rgba(255,255,255,0.11)" : "rgba(0,0,0,0.12)");

let opening = false;

/** Opens the panel under the tray icon, or closes it when it is open. */
export async function togglePanel() {
  const open = Window.get("panel");
  if (open) {
    open.close();
    return;
  }
  if (opening) return;
  opening = true;
  try {
    const room = await Place.place(TITLE, WIDTH).catch(() => 0);
    const panel = Window.open(() => <QuickPanel room={room} />, {
      id: "panel",
      kind: "popup",
      title: TITLE,
      defaultSize: { width: WIDTH, height: LEAST },
      sizeToContent: "height",
      // Hyprland's focus follows the mouse: a click outside closes it there
      closeOnBlur: room === 0,
    });
    panel.on("close", () => {
      turnedOpening = false;
      picking.set(null);
      profilesOpen.set(false);
      if (disconnectIn() === "panel") disconnecting.set(null);
      void Place.closed().catch(() => {});
    });
    void loadAgents();
    void loadProfiles();
    show(tab());
    void Place.outsideClick(TITLE).then(
      (outside) => outside && panel.close(),
      () => {},
    );
  } finally {
    opening = false;
  }
}

/** The main window, on page `p`; the panel goes. */
function openMain(p?: Page) {
  Window.get("panel")?.close();
  if (p) go(p);
  const main = Window.get("main");
  main?.show();
  main?.focus();
}

/** The open panel's mark has turned once, as the panel opened. */
let turnedOpening = false;

/** pansy's mark in the head: it turns once as a pinwheel as the panel opens (with the tray icon), and when the pointer comes over it; not for less motion. */
function Mark() {
  const turn = createMotion();
  let moving = false;
  const once = () => {
    if (moving || reduceMotion()) return;
    moving = true;
    void turn.play({ ...WAG, keyframes: { rotate: [0, 360] } }).finally(() => (moving = false));
  };
  if (!turnedOpening) {
    turnedOpening = true;
    setTimeout(once, 120);
  }
  return (
    <View testID="panel-mark" onMouseEnter={once} style={{ width: 18, height: 18, transformOrigin: HEART, ...turn.style }}>
      <Icon name="pansy" color="foreground" size={18} />
    </View>
  );
}

function Head() {
  return (
    <View class="h-[46px] flex-row items-center gap-2 pl-[14px] pr-[10px]">
      <Mark />
      <Text class="flex-1 text-[13.5px] font-semibold text-foreground">pansy</Text>
      <UpdatePill testID="panel-update" />
      <View class="flex-row gap-[2px]">
        <IconButton icon="sync" label={tab() === "agents" ? tr("Look again: agents installed or removed, settings changed") : tr("Refresh")} testID="panel-sync" onPress={refresh} />
        <IconButton icon="link-out" label={tr("Open the main window")} testID="panel-open" onPress={() => openMain()} />
        <IconButton icon="nav-settings" label={tr("Settings")} testID="panel-settings" onPress={() => openMain("settings")} />
      </View>
    </View>
  );
}

/** The tabs under the head, the one shown on a raised card: Agents (with how many), Usage, Allowances (with a subscription), Routing. */
function Tabs() {
  const tabs = (): { id: Tab; label: string; count: string }[] => [
    { id: "agents", label: tr("Agents"), count: here().length ? String(here().length) : "" },
    { id: "usage", label: tr("Usage"), count: "" },
    ...(providers().some((p) => p.subscription) ? [{ id: "allowances" as Tab, label: tr("Allowances"), count: "" }] : []),
    { id: "routing", label: tr("Routing"), count: "" },
  ];
  return (
    <View class="mx-3 mb-2 flex-row gap-[2px]">
      <Each of={tabs()} key={(t) => t.id}>
        {(t) => {
          const on = () => tab() === t().id;
          return (
            <View
              role="button"
              testID={`panel-tab-${t().id}`}
              onPress={() => show(t().id)}
              class={`group h-[26px] flex-1 flex-row items-center justify-center gap-[3px] rounded-[7px] ${on() ? "bg-seg-thumb" : ""} ${MOTION}`}
              style={on() ? { boxShadow: `0 1px 2px rgba(0,0,0,0.08), ${edge()}` } : undefined}
            >
              <Text class={`text-[12px] ${on() ? "font-medium text-foreground" : "text-muted-foreground group-hover:text-fg-2"} ${MOTION}`}>{t().label}</Text>
              <If when={t().count}>
                <Text class="text-[12px] text-faint">{t().count}</Text>
              </If>
            </View>
          );
        }}
      </Each>
    </View>
  );
}

/** Switched on in the panel: back on what it was on when it was switched off, else its model is picked first. */
function switchOn(agent: Agent) {
  const last = agent.recent.find((m) => choices().some((c) => c.id === m));
  if (last) connect(agent, last);
  else picking.set({ id: agent.id, small: false });
}

/** A setting of an opened line, magpie's field: its name, its value, the model's logo and a chevron; a press opens the picker. */
function Field(props: { agent: Agent; small?: boolean }) {
  const a = () => props.agent;
  const model = () => (props.small ? a().smallModel : a().model);
  const open = () => picking()?.id === a().id && picking()?.small === !!props.small;
  const set = () => a().connected && !!model();
  return (
    <ModelPicker
      open={open()}
      onOpenChange={(o) => picking.set(o ? { id: a().id, small: !!props.small } : null)}
      current={set() ? model() : null}
      reset={props.small ? { label: tr("The same as the main model"), note: tr("titles, summaries and other small requests") } : undefined}
      onPick={(id) => {
        if (props.small) void act(`agent:${a().id}`, () => Agents.setSmallModel(a().id, id)).then(() => loadAgents());
        else connect(a(), id);
      }}
      align="start"
      width={WIDTH - 24}
      listHeight={240}
      trigger={
        <View
          testID={props.small ? `panel-small-${a().id}` : `panel-model-${a().id}`}
          class={`h-[30px] flex-row items-center gap-2 rounded-[8px] bg-card px-[10px] ${MOTION}`}
          style={{ boxShadow: `inset 0 0 0 1px ${open() ? token("primary") : ring()}` }}
        >
          <Text class="w-[62px] text-[11px] text-faint line-clamp-1">{props.small ? tr("Quick tasks") : tr("Model")}</Text>
          <Text class={`flex-1 text-[12.5px] line-clamp-1 ${set() ? "text-foreground" : "text-muted-foreground"}`}>
            {set() ? choiceName(model()!) : props.small ? tr("same as the main model") : tr("default")}
          </Text>
          <If when={set()}>
            <ChoiceLogo id={model()!} size={14} />
          </If>
          <Icon name="chev-down" color="faint" size={11} />
        </View>
      }
    />
  );
}

/** An opened line: connected or not, said, with its switch; then the models it starts on. */
function Body(props: { agent: Agent }) {
  const a = () => props.agent;
  const kind = () => kindOf(a());
  return (
    <View class="gap-2 pb-3 pl-[42px] pr-3 pt-[3px]">
      <View class="min-h-[28px] flex-row items-center gap-2">
        <Text class="w-[62px] text-[11px] text-faint">pansy</Text>
        <Text class="flex-1 text-[11.5px] text-muted-foreground line-clamp-1">{status(a()).text}</Text>
        <Switch
          checked={a().connected}
          label={tr("Connect %s to pansy", a().name)}
          testID={`panel-connect-${a().id}`}
          disabled={isBusy(`agent:${a().id}`) || kind() === "empty"}
          onChange={(on) => (on ? switchOn(a()) : askDisconnect(a(), "panel"))}
        />
      </View>
      <If when={kind() === "empty"}>
        <View class="flex-row">
          <TextButton
            label={tr("Add a provider")}
            testID={`panel-add-provider-${a().id}`}
            onPress={() => {
              adding.set(true);
              openMain("providers");
            }}
          />
        </View>
      </If>
      <If when={kind() === "ok"}>
        <Field agent={a()} />
        <If when={a().connected && a().hasSmallModel}>
          <Field agent={a()} small />
        </If>
      </If>
    </View>
  );
}

/** One agent's quiet line: its logo, its name, and the model it starts on in words; a press opens it in place. */
function Line(props: { agent: Agent; first: boolean; inFold?: boolean }) {
  const a = () => props.agent;
  const gone = () => kindOf(a()) === "gone";
  const open = () => opened() === a().id && !gone();
  const model = () => (a().connected ? a().model : null);
  const said = () => (gone() ? tr("CLI not found") : model() ? choiceName(model()!) : tr("default"));
  return (
    <View testID={`panel-agent-${a().id}`} class={`${props.first ? "" : "border-t border-line-2"} ${open() ? "bg-card-2" : ""} ${MOTION}`}>
      <View
        role="button"
        testID={`panel-line-${a().id}`}
        accessibilityLabel={a().name}
        onPress={() => {
          if (gone()) return;
          picking.set(null);
          opened.set(open() ? null : a().id);
        }}
        class="group h-9 flex-row items-center gap-[10px] px-3"
      >
        <View style={{ opacity: props.inFold && isHidden(a()) ? 0.6 : 1 }}>
          <Logo name={agentLogo(a().id)} size={18} />
        </View>
        <Text class="max-w-[60%] text-[13px] font-medium text-foreground line-clamp-1" style={{ opacity: props.inFold && isHidden(a()) ? 0.6 : 1 }}>
          {a().name}
        </Text>
        <View class="min-w-0 flex-1 flex-row items-center justify-end gap-[10px]">
          <View class="min-w-0 shrink flex-row items-center justify-end gap-1">
            <If when={model()}>
              <View class={`scale-60 opacity-0 group-hover:scale-100 group-hover:opacity-100 ${MOTION}`}>
                <ChoiceLogo id={model()!} size={14} />
              </View>
            </If>
            <Text class="min-w-0 shrink text-right text-[12px] text-muted-foreground line-clamp-1">{said()}</Text>
          </View>
          <View class={`w-[10px] items-center ${open() ? "" : "-rotate-90"} ${MOTION}`} style={{ opacity: gone() ? 0 : 1 }}>
            <Icon name="chev-down" color="faint" size={10} />
          </View>
        </View>
      </View>
      <If when={open()}>
        <Body agent={a()} />
      </If>
    </View>
  );
}

/** The Agents tab: the agents in use, then "Show N more" for the rest. */
function AgentsTab() {
  const list = () => arrange();
  return (
    <View class="overflow-hidden rounded-[10px] bg-card" style={{ boxShadow: edge() }}>
      <Each of={list().shown} key={(a) => a.id}>
        {(a, i) => <Line agent={a()} first={i() === 0} />}
      </Each>
      <If when={more()}>
        <Each of={list().folded} key={(a) => a.id}>
          {(a, i) => <Line agent={a()} first={i() === 0 && list().shown.length === 0} inFold />}
        </Each>
      </If>
      <If when={list().folded.length > 0}>
        <View
          role="button"
          testID="panel-more"
          onPress={() => more.set(!more())}
          class={`group flex-row items-center gap-[6px] px-[14px] py-[9px] hover:bg-card-2 ${list().shown.length || more() ? "border-t border-line-2" : ""} ${MOTION}`}
        >
          <Text class={`text-[12px] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{more() ? tr("Hide") : tr("Show %d more", list().folded.length)}</Text>
          <Icon name={more() ? "chev-up" : "chev-down"} color="muted-foreground" hover="foreground" size={10} />
        </View>
      </If>
    </View>
  );
}

/** A card of the Usage and Routing tabs, edged as magpie's. */
function Card(props: { children?: JSX.Element; class?: string; testID?: string }) {
  return (
    <View testID={props.testID} class={`rounded-[11px] bg-card ${props.class ?? ""}`} style={{ boxShadow: edge() }}>
      {props.children}
    </View>
  );
}

/** Nothing to show yet: what is missing in bold, and why, as magpie says it. */
function Nothing(props: { title: string; text: string; testID?: string }) {
  return (
    <Card class="gap-[2px] p-3" testID={props.testID}>
      <Text class="text-[12px] font-semibold text-foreground">{props.title}</Text>
      <Text class="text-[12px] leading-[19px] text-muted-foreground">{props.text}</Text>
    </Card>
  );
}

/** One of the totals: its name, its figure, and what is under it. */
function Figure(props: { label: string; value: string; sub: string; bad?: boolean; first?: boolean }) {
  return (
    <View class={`min-w-0 flex-1 gap-px px-[10px] py-2 ${props.first ? "" : "border-l border-line-2"}`}>
      <Text class="text-[10.5px] text-muted-foreground line-clamp-1">{props.label}</Text>
      <Text class="text-[16px] font-semibold text-foreground line-clamp-1">{props.value}</Text>
      <Text class={`text-[10.5px] ${props.bad ? "text-destructive" : "text-muted-foreground"}`}>{props.sub}</Text>
    </View>
  );
}

/** Tokens split as magpie's Usage tab says them: input without the cache, output, and the cache read and written. */
function tokensSaid(t: Totals | undefined): string {
  if (!t) return "";
  const cached = t.cacheRead + t.cacheWrite;
  const said = tr("%s in · %s out", fmtN(Math.max(0, t.input - cached)), fmtN(t.output));
  return cached ? `${said} · ${tr("%s cached", fmtN(cached))}` : said;
}

/** The Usage tab: the period, its totals, its days as columns and its providers by share. */
function UsageTab() {
  const s = () => spent();
  const t = () => s()?.total;
  const count = (x: Totals) => (metric() === "tokens" ? x.input + x.output : x.requests);
  const days = () => Object.entries(s()?.byDay ?? {}).sort((a, b) => a[0].localeCompare(b[0]));
  const top = () => Math.max(1, ...days().map(([, x]) => count(x)));
  const ranked = () =>
    Object.entries(s()?.byProvider ?? {})
      .map(([id, x]) => ({ id, n: count(x) }))
      .filter((p) => p.n > 0)
      .sort((a, b) => b.n - a.n);
  const whole = () => Math.max(1, ranked().reduce((n, p) => n + p.n, 0));
  const hit = () => {
    const x = t();
    return x && x.input > 0 ? `${Math.round((Math.min(x.cacheRead, x.input) / x.input) * 100)}%` : "—";
  };
  const amount = (n: number) => (metric() === "tokens" ? fmtN(n) : String(n));
  return (
    <View class="gap-2 px-[2px] pb-[6px] pt-[2px]">
      <View class="min-h-[24px] flex-row items-center gap-2">
        <Segs
          small
          testID="panel-period"
          value={String(period())}
          onChange={(v) => {
            period.set(Number(v));
            void loadSpent();
          }}
          options={[
            { value: "1", label: tr("Today") },
            { value: "7", label: tr("7 days") },
            { value: "30", label: tr("30 days") },
          ]}
        />
        <View class="flex-1" />
        <TextButton
          label={tr("Open Usage")}
          testID="panel-open-usage"
          onPress={() => {
            usageDays.set(period());
            openMain("usage");
          }}
        />
      </View>
      <If
        when={s()}
        else={
          <>
            <View class="h-[58px] rounded-[11px] bg-pill" />
            <View class="h-[150px] rounded-[11px] bg-pill" />
          </>
        }
      >
        <If
          when={(t()?.requests ?? 0) > 0}
          else={<Nothing testID="panel-usage-none" title={period() === 1 ? tr("No requests today") : tr("No requests these days")} text={tr("Every request an agent sends to pansy is counted here.")} />}
        >
          <Card class="flex-row" testID="panel-totals">
            <Figure
              first
              label={tr("Tokens")}
              value={fmtN((t()?.input ?? 0) + (t()?.output ?? 0))}
              sub={tokensSaid(t())}
            />
            <Figure label={tr("Requests")} value={String(t()?.requests ?? 0)} sub={t()?.errors ? tr("%d failed", t()!.errors) : tr("none failed")} bad={!!t()?.errors} />
            <Figure label={tr("Cache hit rate")} value={hit()} sub={tr("of the prompt")} />
          </Card>
          <Card class="gap-2 px-[10px] pb-2 pt-2">
            <Segs
              small
              testID="panel-metric"
              value={metric()}
              onChange={(v) => metric.set(v)}
              options={[
                { value: "tokens", label: tr("Tokens") },
                { value: "calls", label: tr("Calls") },
              ]}
            />
            <If when={days().length > 1}>
              <View>
                <Text class="self-end text-[10.5px] text-faint">{amount(top())}</Text>
                <View class="mt-1 h-[96px] flex-row items-end gap-[3px] border-b border-line-2">
                  <Each of={days()} key={(d) => d[0]}>
                    {(d) => <View class="flex-1 rounded-t-[2px] bg-primary" style={{ height: `${Math.max(1, Math.round((count(d()[1]) / top()) * 100))}%` }} />}
                  </Each>
                </View>
                <View class="mt-1 flex-row">
                  <Text class="flex-1 text-[10.5px] text-muted-foreground">{dayLabel(days()[0]?.[0] ?? "")}</Text>
                  <Text class="text-[10.5px] text-muted-foreground">{dayLabel(days().at(-1)?.[0] ?? "")}</Text>
                </View>
              </View>
            </If>
            <View class="gap-[2px]">
              <Each of={ranked().slice(0, 6)} key={(p) => p.id}>
                {(p) => (
                  <View class="h-[26px] flex-row items-center gap-2">
                    <Logo name={providerLogo(providers().find((x) => x.id === p().id)?.preset)} size={14} />
                    <Text class="w-[120px] text-[12px] text-foreground line-clamp-1">{providerName(p().id)}</Text>
                    <View class="h-1 flex-1 overflow-hidden rounded-[2px] bg-pill">
                      <View class="h-full rounded-[2px] bg-primary/70" style={{ width: `${Math.round((p().n / whole()) * 1000) / 10}%` }} />
                    </View>
                    <Text class="w-[52px] text-right text-[12px] font-semibold text-foreground">{amount(p().n)}</Text>
                  </View>
                )}
              </Each>
            </View>
          </Card>
        </If>
      </If>
    </View>
  );
}

/** A request's tokens as magpie's rows say them: 870, 45.5k, 1.2M. */
const tokensSaidShort = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(1)}k` : String(Math.round(n)));

/**
 * A request of the Routing tab: who asked for which model and when, then
 * where it went and how it went, and its tries, tokens, time and speed:
 * beside where it went when both fit, else under it at the right. A press
 * opens it on the Routing page.
 */
function Request(props: { route: Route }) {
  const r = () => props.route;
  const last = () => r().tries[r().tries.length - 1];
  const slot = () => (last() ? r().lineup[last()!.slot] : undefined);
  const bad = () => failed(r());
  const to = () => {
    const s = slot();
    if (!r().done) return s ? tr("%s is answering…", who(s)) : tr("routing…");
    if (cancelled(r())) return tr("499 · canceled");
    if (bad()) return [String(r().status), short(r().error, s?.provider ?? "")].filter(Boolean).join(" · ");
    return s ? who(s) : String(r().status);
  };
  const model = () => (r().done && !bad() && !cancelled(r()) ? slot()?.model : undefined);
  const meta = () =>
    [
      r().tries.length > 1 ? trn(r().tries.length, "%d try", "%d tries", r().tries.length) : "",
      r().input + r().output ? tr("%s tokens", tokensSaidShort(r().input + r().output)) : "",
      r().done ? dur(r().us) : "",
      speedOf(r()),
    ]
      .filter(Boolean)
      .join(" · ");
  return (
    <View
      role="button"
      testID={`panel-route-${r().id}`}
      onPress={() => {
        routePicked.set(r().id);
        openMain("routing");
      }}
      class={`gap-[2px] rounded-[8px] px-2 py-[6px] hover:bg-card-2 ${MOTION}`}
    >
      <View class="flex-row items-center gap-[6px]">
        <Logo name={agentLogo(r().agent)} size={14} />
        <Text class="text-[12px] font-medium text-foreground">{agentName(r().agent)}</Text>
        <View class="min-w-0 shrink rounded-[5px] bg-card-2 px-[5px]">
          <Text class="font-mono text-[11px] font-medium text-fg-2 line-clamp-1">{r().requested}</Text>
        </View>
        <View class="flex-1" />
        <Text class="text-[11px] text-muted-foreground">{clock(r().at)}</Text>
      </View>
      <View class="flex-row flex-wrap items-center gap-x-[10px] gap-y-[2px] pl-5">
        <View class="min-w-0 flex-auto flex-row items-center gap-[5px]">
          <Text class={`-ml-5 w-[14px] text-center text-[11px] ${bad() ? "text-destructive" : r().done ? "text-faint" : "text-fg-2"}`}>→</Text>
          <Text class={`min-w-0 shrink text-[12px] line-clamp-1 ${bad() ? "text-destructive" : "text-fg-2"}`}>{to()}</Text>
          <If when={model()}>
            <Text class="min-w-[24px] shrink text-[12px] text-muted-foreground line-clamp-1">{`· ${model() ?? ""}`}</Text>
          </If>
        </View>
        <If when={meta()}>
          <Text class="ml-auto max-w-full text-right text-[11px] text-muted-foreground line-clamp-1 tabular-nums">{meta()}</Text>
        </If>
      </View>
    </View>
  );
}

/** The small stage's height, the room its wires take between its columns, a place's height and the gap between two, and the most places a side. */
const STAGE = 104;
const SPAN = 38;
const NODE = 22;
const NODE_GAP = 6;
const SIDE = 3;

/** Where a try went on the small stage: its provider, the account or key there when it has several, and how its latest try there went. */
interface Seat {
  key: string;
  name: string;
  sub: string;
  how?: "ok" | "bad" | "wait";
}

function seatOf(r: Route, t: RouteTry): Seat {
  const s = r.lineup[t.slot];
  if (!s) return { key: "?", name: "?", sub: "" };
  const name = providerName(s.provider);
  const account = who(s);
  return { key: s.provider, name, sub: account === name ? "" : account };
}

/** Those on the stage kept where they are while still wanted, the newcomers taking the free places. */
function keep(had: string[], want: string[]): string[] {
  const out: (string | null)[] = had.map((k) => (want.includes(k) ? k : null));
  for (const k of want) {
    if (out.includes(k)) continue;
    const free = out.indexOf(null);
    if (free >= 0) out[free] = k;
    else out.push(k);
  }
  return out.filter((k): k is string => k !== null).slice(0, SIDE);
}

/** The middle of place `i` of the `n` a column holds, from the stage's top. */
const middleOf = (i: number, n: number) => (STAGE - (n * NODE + (n - 1) * NODE_GAP)) / 2 + i * (NODE + NODE_GAP) + NODE / 2;

/** A place on the small stage: an agent's logo or a provider's dot, and its name. */
function Spot(props: { name: string; sub?: string; agent?: string; how?: Seat["how"]; testID?: string }) {
  const dot = () => (props.how === "bad" ? "bg-destructive" : props.how === "wait" ? "bg-muted-foreground" : "bg-success");
  return (
    <View testID={props.testID} class="h-[22px] max-w-full flex-row items-center gap-[5px] rounded-[7px] bg-card-2 px-2" style={{ boxShadow: `inset 0 0 0 1px ${token("border")}` }}>
      <If when={props.agent} else={<View class={`h-[6px] w-[6px] rounded-full ${dot()}`} />}>
        <Logo name={agentLogo(props.agent!)} size={13} />
      </If>
      <Text class="min-w-0 shrink text-[11.5px] font-medium text-foreground line-clamp-1">
        {props.name}
        <Text class="text-[10.5px] font-normal text-muted-foreground">{props.sub ? ` ${props.sub}` : ""}</Text>
      </Text>
    </View>
  );
}

/**
 * magpie's small stage over the requests: the agents that asked lately,
 * pansy, and the providers their requests went to, three a side at most,
 * each kept in its place while it stays. A provider says how its latest try
 * went (green, red, grey while it answers) and, when it has several, which
 * account. While a request is out, its wires take its agent's color and
 * data flows on them, out to the provider and back with the answer, and
 * pansy turns as a pinwheel; still, with less motion asked for.
 */
function MiniStage() {
  let agentsHad: string[] = [];
  let seatsHad: string[] = [];
  const cast = computed(() => {
    const recent = [...routes()].sort((a, b) => b.id - a.id);
    const live = recent.filter((r) => !r.done);
    const agents: string[] = [];
    const seats = new Map<string, Seat>();
    for (const r of live) {
      if (!agents.includes(r.agent)) agents.push(r.agent);
      for (const t of r.tries) {
        const s = seatOf(r, t);
        if (!seats.has(s.key)) seats.set(s.key, s);
      }
    }
    for (const r of recent) {
      if (agents.length < SIDE && !agents.includes(r.agent)) agents.push(r.agent);
      const t = r.tries[r.tries.length - 1];
      if (!t) continue;
      const s = seatOf(r, t);
      if (!seats.has(s.key) && seats.size < SIDE) seats.set(s.key, s);
    }
    // each provider says how its latest try went, and to which account
    for (const r of recent) {
      for (const t of [...r.tries].reverse()) {
        const s = seatOf(r, t);
        const seat = seats.get(s.key);
        if (!seat || seat.how) continue;
        seat.how = t.status === 0 || (!r.done && t.status < 400) ? "wait" : t.status < 400 ? "ok" : "bad";
        seat.sub = s.sub;
      }
    }
    agentsHad = keep(agentsHad, agents.slice(0, SIDE));
    seatsHad = keep(seatsHad, [...seats.keys()].slice(0, SIDE));
    return { agents: agentsHad, seats: seatsHad.map((k) => seats.get(k)!), live };
  });
  // with less motion asked for, the wires a request is out on are lit, not flowing
  const way = (r: Route) => (reduceMotion() ? null : wayOf(r));
  const into = (): Wire[] => {
    const { agents, live } = cast();
    return agents.map((a, i) => {
      const r = live.find((x) => x.agent === a);
      return { from: middleOf(i, agents.length), to: STAGE / 2, color: r ? hueOf(a) : token("border"), way: r ? way(r) : null };
    });
  };
  const out = (): Wire[] => {
    const { seats, live } = cast();
    return seats.map((s, i) => {
      const r = live.find((x) => {
        const t = x.tries[x.tries.length - 1];
        return !!t && seatOf(x, t).key === s.key;
      });
      return { from: STAGE / 2, to: middleOf(i, seats.length), color: r ? hueOf(r.agent) : token("border"), way: r ? way(r) : null };
    });
  };
  const pass = createAnimatedValue(0);
  const flowing = computed(() => into().some((w) => w.way) || out().some((w) => w.way));
  effect(() => {
    if (!flowing()) return;
    void pass.keyframes([0, 1], { duration: 1000, easing: "linear", iterations: "infinite", frameRate: 60 });
    return () => void pass.stop();
  });
  // pansy turns as a pinwheel while any request is out, as the tray icon does
  const turn = createAnimatedValue(0);
  const turning = computed(() => cast().live.length > 0 && !reduceMotion());
  effect(() => spin(turn, turning()));
  return (
    <View testID="panel-stage" class="flex-row items-center rounded-[11px] bg-card px-[10px]" style={{ height: STAGE, boxShadow: edge() }}>
      <View class="min-w-0 flex-1 items-end" style={{ gap: NODE_GAP }}>
        <Each of={cast().agents} key={(a) => a}>
          {(a) => <Spot agent={a()} name={agentName(a())} testID={`panel-stage-agent-${a()}`} />}
        </Each>
      </View>
      <Wires wires={into()} height={STAGE} pass={pass} width={SPAN} />
      <View class="h-10 w-10 items-center justify-center rounded-full bg-card" style={{ boxShadow: `inset 0 0 0 1px ${token("border")}, 0 1px 3px rgba(0,0,0,0.08)` }}>
        <View testID="panel-stage-hub" style={{ rotate: turn }}>
          <Icon name="pansy" color="foreground" size={24} />
        </View>
      </View>
      <Wires wires={out()} height={STAGE} pass={pass} width={SPAN} />
      <View class="min-w-0 flex-1 items-start" style={{ gap: NODE_GAP }}>
        <Each of={cast().seats} key={(s) => s.key}>
          {(s) => <Spot name={s().name} sub={s().sub} how={s().how} testID={`panel-stage-to-${s().key}`} />}
        </Each>
      </View>
    </View>
  );
}

/** The Routing tab: whether a request is out, today's count, the small stage, and the latest six requests, newest first. */
function RoutingTab() {
  const live = () => routes().some((r) => !r.done);
  const latest = () => routes().slice(-6).reverse();
  const elsewhere = () => gateway()?.running === true && gateway()?.own === false;
  // today's counts in bold, wherever the language puts them in its words
  const calls = () => trn(usageToday()?.requests ?? 0, "%d call", "%d calls", "\u0000").split("\u0000");
  const tokens = () => tr("%s tokens", "\u0000").split("\u0000");
  return (
    <View class="gap-2 px-[2px] pb-[6px] pt-[2px]">
      <View class="min-h-[22px] flex-row items-center gap-2 pl-1 pr-[2px]">
        <View class={`h-[7px] w-[7px] rounded-full ${live() ? "bg-success" : "bg-faint"}`}>
          <If when={live()}>
            <View class="absolute inset-0 rounded-full bg-success motion-safe:animate-ping" />
          </If>
        </View>
        <If when={usageToday()} else={<View class="h-[9px] w-[150px] rounded-[4px] bg-pill" />}>
          <Text testID="panel-today" class="min-w-0 shrink text-[12px] text-muted-foreground line-clamp-1 tabular-nums">
            {`${tr("today")} ${calls()[0] ?? ""}`}
            <Text class="font-semibold text-foreground">{String(usageToday()!.requests)}</Text>
            {`${calls()[1] ?? ""} · ${tokens()[0] ?? ""}`}
            <Text class="font-semibold text-foreground">{fmtN(usageToday()!.input + usageToday()!.output)}</Text>
            {tokens()[1] ?? ""}
          </Text>
        </If>
        <View class="flex-1" />
        <TextButton label={tr("Open Routing")} testID="panel-open-routing" onPress={() => openMain("routing")} />
      </View>
      <If
        when={!elsewhere()}
        else={<Nothing title={tr("No request yet")} text={tr("Another pansy (pansy serve) answers on this port, so the requests it routes play in its own place, not here.")} />}
      >
        <If
          when={latest().length > 0}
          else={<Nothing testID="panel-routing-none" title={tr("No request yet")} text={tr("Requests through pansy show here as they happen, the last 60 of them.")} />}
        >
          <MiniStage />
          <Card class="p-1" testID="panel-routes">
            <Each of={latest()} key={(r) => r.id}>
              {(r) => <Request route={r()} />}
            </Each>
          </Card>
        </If>
      </If>
    </View>
  );
}

/** The foot: the profiles (on Agents), what just happened, and Quit. */
function Foot() {
  const last = () => toasts().at(-1);
  const current = () => profiles().find((p) => p.active)?.name;
  return (
    <View class="flex-row items-center gap-[2px] border-t border-line-2 pb-[7px] pl-[14px] pr-2 pt-[5px]">
      <If when={tab() === "agents"}>
        <View class="-ml-[7px]">
          <Popover
            open={profilesOpen()}
            onOpenChange={({ open }) => profilesOpen.set(open)}
            placement="top"
            align="start"
            offset={6}
            margin={8}
            trigger={
              <View
                testID="panel-profiles"
                class={`group h-[23px] flex-row items-center gap-1 rounded-[6px] px-[7px] hover:bg-pill aria-expanded:bg-pill-hover active:scale-96 ${MOTION}`}
              >
                <Text class={`text-[12px] text-ctl group-hover:text-foreground group-aria-expanded:text-foreground ${MOTION}`}>{tr("Profiles")}</Text>
                <Text class={`max-w-[160px] text-[12px] line-clamp-1 ${current() ? "text-foreground" : "text-faint"}`}>{current() ?? (profiles().length ? String(profiles().length) : "")}</Text>
              </View>
            }
            popupClass="overflow-hidden rounded-[11px] border border-border bg-popover"
            popupStyle={{ boxShadow: SHADOW.pop() }}
          >
            <View class="w-[360px]">
              <ProfilesBar stacked />
            </View>
          </Popover>
        </View>
      </If>
      <Text testID="panel-status" class={`min-w-0 flex-1 px-[6px] text-[12px] line-clamp-1 ${last()?.tone === "error" ? "text-destructive" : "text-muted-foreground"}`}>
        {last()?.message ?? ""}
      </Text>
      <TextButton label={tr("Quit")} testID="panel-quit" onPress={() => Application.quit()} />
    </View>
  );
}

/** The panel, `room` points at most where Hyprland placed it (0: anywhere). */
function QuickPanel(props: { room: number }) {
  const most = () => (props.room > 0 ? Math.min(MOST, props.room) : MOST);
  // a question asked here takes the room it needs
  const asking = () => disconnectIn() === "panel" && disconnecting() !== null;
  const least = () => Math.min(asking() || profilesOpen() ? 460 : LEAST, most());
  // Routing plays the requests as the gateway routes them, while it shows
  effect(() => {
    if (tab() !== "routing") return;
    const timer = setInterval(() => {
      if (!Window.get("panel")) clearInterval(timer);
      else if (loadTrace()) void loadUsageToday();
    }, 700);
    return () => clearInterval(timer);
  });
  return (
    <View testID="panel" class={`border border-border bg-background ${fontClass()}`} style={{ minHeight: least(), maxHeight: most() }}>
      <Head />
      <Tabs />
      <Scroll class="min-h-0 shrink">
        <View class="px-3 pb-[10px] pt-[2px]">
          <If when={tab() === "agents"}>
            <AgentsTab />
          </If>
          <If when={tab() === "usage"}>
            <UsageTab />
          </If>
          <If when={tab() === "allowances"}>
            <AllowanceList compact />
          </If>
          <If when={tab() === "routing"}>
            <RoutingTab />
          </If>
        </View>
      </Scroll>
      <View class="flex-1" />
      <Foot />
      <DisconnectDialog where="panel" width={WIDTH - 48} />
    </View>
  );
}
