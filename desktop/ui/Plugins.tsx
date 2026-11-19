// Models › Plugins, as magpie's market (plugins.js): Discover, every
// community plugin whoever wrote it — this week's pick and the most
// downloaded, then shelves by kind (OpenCode plugins and pi packages
// first, middleware, agents), each plugin a card with its one button — and
// a search that looks on npm too; Installed, a row for each plugin added
// with what it serves and its switches. A provider a plugin serves is
// signed in to here: its questions, a page (and a code to paste back), or
// a key, as the plugin asks. The plugins run on Bun (pansy-plugins).
import { Clipboard, colorScheme, effect, Image, openUrl, Scroll, signal, Text, View, type ScrollRef } from "nsh";
import { Plugins } from "pansy/native";
import { locale, tr } from "./i18n";
import {
  act,
  ask,
  compact,
  go,
  isBusy,
  loadMarket,
  loadPlugins,
  loadProviders,
  magpieFound,
  market,
  pluginStats,
  pluginTab,
  plugins,
  reason,
  setSetting,
  settings,
  toast,
  type MarketEntry,
  type PluginFlow,
  type PluginProvider,
  type PluginRow,
  type Shelf,
} from "./model";
import { CopyButton, Dropdown, hasLogo, Hint, Icon, IconButton, Input, List, Logo as BrandLogo, PrimaryButton, Row, Segs, SHADOW, Switch, TextButton, type Color, type IconName, type LogoName } from "./parts";

/** The provider whose sign-in is open. */
const signingIn = signal<PluginProvider | null>(null);
/** The plugin whose options are open, and what is typed there. */
const editing = signal<{ spec: string; text: string } | null>(null);
/** What is typed in the search field. */
const query = signal("");
/** npm's answer for the query: loading, its plugins, or why not. */
const hits = signal<{ q: string; list?: MarketEntry[]; error?: string } | null>(null);
/** The package being installed. */
const installing = signal("");
/** The shelf picked ("": every shelf), the source and the order. */
const shelfPick = signal<Shelf | "">("");
const sourcePick = signal<"" | "ours" | "third">("");
const sortPick = signal<"hot" | "name">("hot");

const lang = () => (locale() === "zh-Hans" ? "zh" : "en");
const summaryOf = (m: MarketEntry) => m.summary[lang()] || m.summary.en || "";
/** A label in the window's language, when the plugin gives one. */
const labelOf = (label: string, labels?: Record<string, string>) => labels?.[lang()] ?? label;

/** Each kind of plugin: its glyph, its color, its word and what it does (magpie's KINDS). */
const KINDS = (): Record<string, { icon: IconName; color: Color; word: string; tip: string }> => ({
  provider: { icon: "pm-plug", color: "primary", word: tr("Provider"), tip: tr("Signs in to a subscription and serves its models") },
  pi: { icon: "pm-plug", color: "primary", word: tr("pi package"), tip: tr("A provider pi's packages add") },
  middleware: { icon: "pm-pipe", color: "success", word: tr("Middleware"), tip: tr("Runs in pansy's gateway on the requests your agents send and on the replies") },
  hooks: { icon: "pm-pipe", color: "success", word: tr("Request hooks"), tip: tr("Hears each request before it goes out") },
  agent: { icon: "pm-term", color: "warning", word: tr("Agent"), tip: tr("Adds an agent pansy has no setup of its own for") },
  tools: { icon: "pm-tool", color: "fg-2", word: tr("Tools"), tip: tr("Gives every agent tools of its own") },
  page: { icon: "pm-page", color: "fg-2", word: tr("Page"), tip: tr("Adds a page of its own") },
});
const kindOf = (m: MarketEntry) => (m.shelf === "pi" ? "pi" : m.kind);

/** The market's shelves: their names, and how many cards each shows before See all. */
/** The shelves, each showing `rows` full rows of cards until See all. */
const SHELVES = (): { id: Shelf; name: string; short: string; rows: number }[] => [
  { id: "opencode", name: tr("OpenCode plugins"), short: "OpenCode", rows: 2 },
  { id: "pi", name: tr("pi packages"), short: "pi", rows: 2 },
  { id: "middleware", name: tr("Gateway middleware"), short: tr("Middleware"), rows: 1 },
  { id: "agent", name: tr("Agents"), short: tr("Agents"), rows: 1 },
  { id: "tools", name: tr("Tools for agents"), short: tr("Tools"), rows: 1 },
  { id: "page", name: tr("Pages"), short: tr("Pages"), rows: 1 },
];

function reload() {
  return Promise.all([loadPlugins(), loadProviders()]);
}

/** What each plugin is busy with, by its spec: the words shown with a spinner (Updating…, Removing…). */
const working = signal<Record<string, string>>({});

/** Runs `work` on the plugin `spec`, its row saying `doing` with a spinner until it and the lists read again are done. */
function busyWith(spec: string, doing: string, work: () => Promise<unknown>, done?: string) {
  if (working()[spec]) return Promise.resolve(false);
  working.update((w) => ({ ...w, [spec]: doing }));
  return act(`plugin:${spec}`, async () => {
    await work();
    await reload();
  }, done).finally(() =>
    working.update((w) => {
      const next = { ...w };
      delete next[spec];
      return next;
    }),
  );
}

/** A turning ring, as a loader. */
function Spinner(props: { size?: number; color?: Color }) {
  return (
    <View class="motion-safe:animate-spin">
      <Icon name="loader-circle" color={props.color ?? "muted-foreground"} size={props.size ?? 12} />
    </View>
  );
}

/** A thin glowing bar along a card's or a row's foot while work on it is under way. */
function Working() {
  return <View class="absolute bottom-0 left-0 right-0 h-[2px] bg-primary/70 motion-safe:animate-pulse" />;
}

/** A spinner and what is being done, where a button was. */
function Doing(props: { label: string; testID?: string }) {
  return (
    <View testID={props.testID} class="h-[26px] flex-row items-center gap-[6px] px-2">
      <Spinner />
      <Text class="text-[12px] font-medium text-muted-foreground">{props.label}</Text>
    </View>
  );
}

/** The plugin added for a community plugin's package. */
const installedOf = (pkg: string) => plugins().find((p) => p.package === pkg);

function install(spec: string) {
  const s = spec.trim();
  if (!s || installing()) return;
  installing.set(s);
  void act("plugin-add", async () => {
    try {
      const name = await Plugins.add(s);
      await reload();
      toast(tr("%s is installed", name));
    } finally {
      installing.set("");
    }
  });
}

/** The provider of these whose sign-in is open, if one is. */
const signingInOne = (providers: PluginProvider[]) => providers.some((p) => p.id === signingIn()?.id);

/** A plugin's page on npm, or its folder's place. */
const details = (pkg: string) => void openUrl(`https://www.npmjs.com/package/${pkg}`);

/** The brand of a provider a plugin signs in to, by OpenCode's id, for a plugin that gives no picture (magpie's `pluginIcon`). */
const PROVIDER_BRANDS: Record<string, string> = {
  "github-copilot": "githubcopilot",
  "github-copilot-enterprise": "githubcopilot",
  anthropic: "claude-color",
  openai: "openai",
  google: "gemini-color",
  "google-vertex": "gemini-color",
  qwen: "qwen-color",
  alibaba: "qwen-color",
  moonshotai: "kimi",
  "kimi-for-coding": "kimi",
};
const brandOf = (providers: string[]) => providers.map((id) => PROVIDER_BRANDS[id]).find(Boolean) ?? "";
/** A market entry's icon: the market's, else its provider's brand, else pi's mark (LobeHub's, as magpie shows the pi agent) for a pi package. */
const iconOf = (m: MarketEntry) => m.icon || brandOf(m.providers) || (m.shelf === "pi" ? "pi" : "");

/** A picture a plugin gives, as magpie takes one: an https URL or a data:image URI of at most 1.5 MB; "" for anything else. */
function pictureOf(icon: string) {
  const l = icon.toLowerCase();
  return icon.length <= 3 << 19 && (l.startsWith("https://") || l.startsWith("data:image/")) ? icon : "";
}

/** LobeHub's picture of an icon pansy doesn't bundle, in the theme's tone (`getLobeIconCDN`'s PNG): from npmmirror with 国内镜像 on, unpkg otherwise. */
const lobeHub = (name: string) =>
  `${settings()?.chinaMirror ? "https://registry.npmmirror.com/@lobehub/icons-static-png/latest/files" : "https://unpkg.com/@lobehub/icons-static-png@latest"}/${colorScheme() === "dark" ? "dark" : "light"}/${name}.png`;

/**
 * A plugin's picture in a rounded box: the brand its `icon` names (LobeHub's
 * icons, bundled, or else from LobeHub's CDN), or the picture it gives. Until
 * a picture is in, and for one that never is, magpie's glyph for its kind.
 */
function Logo(props: { kind: string; icon?: string; size?: number; busy?: boolean }) {
  const size = () => props.size ?? 36;
  const k = () => KINDS()[props.kind];
  const icon = () => (props.icon ?? "").trim();
  const src = () => pictureOf(icon()) || (/^[a-z0-9-]+$/.test(icon()) && !hasLogo(icon()) ? lobeHub(icon()) : "");
  /** The picture drawn so far: the glyph shows until it is `src()`. */
  const loaded = signal("");
  const inner = () => Math.round(size() * 0.6);
  const glyph = () => (
    <If when={!src() || loaded() !== src()}>
      <Icon name={k()?.icon === "pm-plug" || !k() ? "pm-puzzle" : k()!.icon} color="fg-2" size={Math.round(size() * 0.5)} />
    </If>
  );
  return (
    <View class={`items-center justify-center bg-pill ${props.busy ? "motion-safe:animate-pulse" : ""}`} style={{ width: size(), height: size(), borderRadius: Math.round(size() * 0.28) }}>
      <If when={hasLogo(icon())} else={glyph()}>
        <BrandLogo name={icon() as LogoName} size={inner()} />
      </If>
      <If when={src()}>
        <Image
          src={src()}
          onLoad={() => loaded.set(src())}
          style={{ position: "absolute", left: (size() - inner()) / 2, top: (size() - inner()) / 2, width: inner(), height: inner(), borderRadius: Math.round(inner() * 0.22), opacity: loaded() === src() ? 1 : 0 }}
        />
      </If>
    </View>
  );
}

/** What a plugin is, as a chip: its glyph and word, in its kind's color. */
function KindChip(props: { kind: string }) {
  const k = () => KINDS()[props.kind] ?? KINDS().provider!;
  return (
    <Hint text={k().tip}>
      <View class="flex-row items-center gap-1">
        <Icon name={k().icon} color={k().color} size={10} />
        <Text class="text-[11px] text-fg-2">{k().word}</Text>
      </View>
    </Hint>
  );
}

function Chip(props: { label: string; tone?: "soft" | "bad" | "up" }) {
  const tone = () =>
    ({ soft: "border border-border bg-transparent text-muted-foreground", bad: "bg-destructive-soft text-destructive", up: "bg-success-soft text-success" })[props.tone ?? "soft"] ?? "";
  return (
    <View class={`h-[17px] justify-center rounded-[5px] px-[6px] ${tone().split(" ").filter((c) => !c.startsWith("text-")).join(" ")}`}>
      <Text class={`text-[10.5px] font-medium ${tone().split(" ").find((c) => c.startsWith("text-")) ?? "text-fg-2"}`}>{props.label}</Text>
    </View>
  );
}

/** Who a plugin is from, under its name: magpie's community, or its publisher, alike in plain words. */
function Byline(props: { m: MarketEntry }) {
  return (
    <Text class="text-[11px] text-muted-foreground line-clamp-1">
      {props.m.community ? tr("magpie community") : props.m.publisher || props.m.package}
    </Text>
  );
}

/** A plugin's one button, as magpie's: Install, Installing…, Switch on, Sign in, Signed in or Installed. */
function Action(props: { m: MarketEntry }) {
  const m = () => props.m;
  const base = "h-[26px] min-w-[64px] flex-row items-center justify-center gap-[5px] rounded-[13px] px-3";
  return (
    <If when={doingOf(m())} else={<Ready m={m()} base={base} />}>
      <View testID={`market-busy-${m().name}`} class={`${base} border border-border bg-pill`}>
        <Spinner />
        <Text class="text-[12px] font-medium text-muted-foreground">{doingOf(m())}</Text>
      </View>
    </If>
  );
}

/** What is under way for a community plugin, in words: installing it, its row's work, or its sign-in; "" when nothing is. */
function doingOf(m: MarketEntry): string {
  if (installing() === m.package) return tr("Installing…");
  const e = installedOf(m.package);
  if (!e) return "";
  if (working()[e.spec]) return working()[e.spec]!;
  return !e.off && signingInOne(e.providers) ? tr("Signing in…") : "";
}

/** Action's buttons when nothing is under way. */
function Ready(props: { m: MarketEntry; base: string }) {
  const m = () => props.m;
  const e = () => installedOf(m().package);
  const providers = () => e()?.providers ?? [];
  const signedIn = () => providers().some((p) => p.accounts.length > 0);
  const base = props.base;
  return (
    <>
      <If when={!e() && !m().runs}>
        <View class={`${base} bg-pill opacity-60`}>
          <Text class="text-[12px] font-semibold text-fg-2">{tr("Not yet in pansy")}</Text>
        </View>
      </If>
      <If when={!e() && m().runs}>
        <View
          role="button"
          testID={`market-add-${m().name}`}
          onPress={() => install(m().package)}
          class={`${base} border border-border ${installing() ? "opacity-55" : "hover:bg-pill"}`}
        >
          <Icon name="pm-down" color="foreground" size={12} />
          <Text class="text-[12px] font-semibold text-foreground">{tr("Install")}</Text>
        </View>
      </If>
      <If when={e()?.off}>
        <View role="button" onPress={() => void busyWith(e()!.spec, tr("Switching on…"), () => Plugins.setOn(e()!.spec, true))} class={`${base} bg-pill hover:bg-pill-hover`}>
          <Text class="text-[12px] font-semibold text-fg-2">{tr("Switch on")}</Text>
        </View>
      </If>
      <If when={e() && !e()!.off && providers().length > 0 && !signedIn()}>
        <View role="button" testID={`market-sign-in-${m().name}`} onPress={() => beginSignIn(providers()[0]!)} class={`${base} border border-border hover:bg-pill`}>
          <Text class="text-[12px] font-semibold text-foreground">{tr("Sign in")}</Text>
        </View>
      </If>
      <If when={e() && !e()!.off && (signedIn() || providers().length === 0)}>
        <View
          role="button"
          onPress={() => (signedIn() ? go("providers") : undefined)}
          class={`h-[26px] flex-row items-center gap-1 rounded-[13px] px-2 ${signedIn() ? "hover:bg-success-soft" : ""}`}
        >
          <Icon name="pm-check" color="success" size={11} />
          <Text class="text-[12px] font-semibold text-success">{signedIn() ? tr("Signed in") : tr("Installed")}</Text>
        </View>
      </If>
    </>
  );
}

/**
 * As many columns of 262 points or more as a width takes (magpie's .pm-grid,
 * `auto-fill, minmax(262px, 1fr)`): `measure` is the grid's onLayout. A
 * card's width is rounded down, so a row of them never wraps a point early.
 */
function columns() {
  const width = signal(0);
  const cols = () => Math.max(1, Math.floor((width() + 10) / 272));
  return {
    cols,
    cardW: () => (width() ? Math.floor((width() - 10 * (cols() - 1)) / cols()) : 262),
    measure: (e: { bounds: { size: { width: number } } }) => width.set(e.bounds.size.width),
  };
}

/** A grid of cards (magpie's .pm-grid), measured by `c` when a shelf counts its columns too. */
function Grid(props: { list: MarketEntry[]; c?: ReturnType<typeof columns> }) {
  const c = props.c ?? columns();
  return (
    <View class="flex-row flex-wrap gap-[10px]" onLayout={c.measure}>
      <Each of={props.list} key={(m) => m.package}>
        {(m) => <Card m={m()} width={c.cardW()} />}
      </Each>
    </View>
  );
}

/** A community plugin: its picture, name and author, its one button, two lines of what it does, and its facts. */
function Card(props: { m: MarketEntry; width: number }) {
  const m = () => props.m;
  const e = () => installedOf(m().package);
  return (
    <View
      role="button"
      testID={`market-${m().name}`}
      onPress={() => details(m().package)}
      class={`gap-2 overflow-hidden rounded-[12px] border bg-card pb-[11px] pl-[13px] pr-3 pt-3 hover:border-ring ${doingOf(m()) ? "border-ring" : "border-border"}`}
      style={{ width: props.width }}
    >
      <If when={doingOf(m())}>
        <Working />
      </If>
      <View class="flex-row items-center gap-[10px]">
        <Logo kind={kindOf(m())} icon={iconOf(m())} busy={!!doingOf(m())} />
        <View class="min-w-0 flex-1 gap-[2px]">
          <Text class="text-[13px] font-semibold text-foreground line-clamp-1">{m().name}</Text>
          <Byline m={m()} />
        </View>
        <Action m={m()} />
      </View>
      <Text class="h-[35px] text-[12px] leading-[17px] text-fg-2 line-clamp-2">{summaryOf(m())}</Text>
      <View class="min-h-[18px] flex-row flex-wrap items-center gap-2">
        <KindChip kind={kindOf(m())} />
        <If when={m().weekly > 0}>
          <View class="flex-row items-center gap-[3px]">
            <Icon name="pm-down" color="muted-foreground" size={11} />
            <Text class="text-[11px] text-muted-foreground">{tr("%s/week", compact(m().weekly))}</Text>
          </View>
        </If>
        <If when={e()?.version || m().version}>
          <Text class="font-mono text-[10.5px] text-muted-foreground">{`v${e()?.version || m().version}`}</Text>
        </If>
        <If when={e()?.error && !e()?.off}>
          <Chip label={tr("Didn't load")} tone="bad" />
        </If>
        <If when={e()?.off}>
          <Chip label={tr("Off")} />
        </If>
      </View>
    </View>
  );
}

/** Most downloaded first (magpie's "Most popular"), or by name. */
const ranked = (list: MarketEntry[], how = sortPick()) =>
  [...list].sort((a, b) => (how === "name" ? a.name.localeCompare(b.name) : b.weekly - a.weekly || a.name.localeCompare(b.name)));

/** The day in the reader's own calendar, which the day's pick turns on. */
const today = () => {
  const d = new Date();
  return Math.floor((d.getTime() - d.getTimezoneOffset() * 6e4) / 864e5);
};

/** This week: the day's pick (one of the most downloaded not installed, a different one each day) and the most downloaded. */
function ThisWeek(props: { all: MarketEntry[] }) {
  const listed = () => ranked(props.all.filter((m) => m.weekly > 0 && m.runs), "hot");
  const day = () => {
    let pool = listed().filter((m) => !installedOf(m.package)).slice(0, 14);
    if (!pool.length) pool = listed().slice(0, 14);
    return pool[today() % Math.max(1, pool.length)];
  };
  const date = () => {
    const d = new Date();
    return lang() === "zh" ? `${d.getMonth() + 1}月${d.getDate()}日` : `${["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][d.getMonth()]} ${d.getDate()}`;
  };
  return (
    <If when={day()}>
      <View class="gap-[10px]" testID="market-week">
        <View class="flex-row items-baseline gap-2 px-[2px]">
          <Text class="text-[15px] font-semibold text-foreground">{tr("This week")}</Text>
          <Text class="text-[11.5px] text-muted-foreground">{date()}</Text>
        </View>
        <View class="flex-row gap-[10px]">
          <Hero m={day()!} />
          <View class="flex-1 rounded-[14px] border border-border bg-card px-2 pb-2 pt-3">
            <View class="flex-row items-baseline gap-2 px-2 pb-1">
              <Text class="text-[12.5px] font-semibold text-foreground">{tr("Most downloaded")}</Text>
              <Text class="text-[11px] text-muted-foreground">{tr("last week, on npm")}</Text>
            </View>
            <Each of={listed().slice(0, 5)} key={(m) => m.package}>
              {(m, i) => (
                <View role="button" onPress={() => details(m().package)} class="flex-row items-center gap-[9px] rounded-[9px] px-2 py-[6px] hover:bg-pill">
                  <Text class="w-[14px] text-center text-[11.5px] font-semibold text-faint">{String(i() + 1)}</Text>
                  <Logo kind={kindOf(m())} icon={iconOf(m())} size={28} busy={!!doingOf(m())} />
                  <View class="min-w-0 flex-1">
                    <Text class="text-[12.5px] font-semibold text-foreground line-clamp-1">{m().name}</Text>
                    <Text class="text-[11px] text-muted-foreground line-clamp-1">{`${KINDS()[kindOf(m())]?.word ?? ""} · ↓${tr("%s/week", compact(m().weekly))}`}</Text>
                  </View>
                  <Action m={m()} />
                </View>
              )}
            </Each>
          </View>
        </View>
      </View>
    </If>
  );
}

/** The day's pick, large: its picture, name, author, what it does, its numbers, and its button beside Details. */
function Hero(props: { m: MarketEntry }) {
  const m = () => props.m;
  const stat = (value: string, key: string) => (
    <View class="gap-px">
      <Text class="text-[14px] font-semibold text-foreground">{value}</Text>
      <Text class="text-[11px] text-muted-foreground">{key}</Text>
    </View>
  );
  return (
    <View role="button" onPress={() => details(m().package)} class="flex-[1.3] gap-[10px] rounded-[14px] border border-border bg-card px-[18px] py-4 hover:border-ring">
      <Text class="text-[11px] font-semibold text-primary">{tr("Today's pick")}</Text>
      <View class="flex-row items-center gap-[10px]">
        <Logo kind={kindOf(m())} icon={iconOf(m())} size={52} busy={!!doingOf(m())} />
        <View class="min-w-0 flex-1 gap-[2px]">
          <Text class="text-[15px] font-semibold text-foreground line-clamp-1">{m().name}</Text>
          <Byline m={m()} />
        </View>
      </View>
      <Text class="text-[12.5px] leading-[18px] text-fg-2 line-clamp-3">{summaryOf(m())}</Text>
      <View class="flex-1" />
      <View class="flex-row gap-[22px]">
        {stat(compact(m().weekly), tr("Downloads last week"))}
        {stat(KINDS()[kindOf(m())]?.word ?? "", tr("Kind"))}
        <If when={m().version}>{stat(`v${m().version}`, tr("Version"))}</If>
      </View>
      <View class="flex-row items-center gap-2">
        <Action m={m()} />
        <View role="button" onPress={() => details(m().package)} class="h-[26px] flex-row items-center gap-1 rounded-[13px] border border-border px-3 hover:bg-pill">
          <Text class="text-[12px] font-semibold text-fg-2">{tr("Details")}</Text>
          <Icon name="pm-out" color="fg-2" size={11} />
        </View>
      </View>
    </View>
  );
}

/** A shelf: its name and count, See all, and its cards (all of them once picked). */
function ShelfView(props: { id: Shelf; list: MarketEntry[] }) {
  const shelf = () => SHELVES().find((s) => s.id === props.id)!;
  const picked = () => shelfPick() === props.id;
  const count = signal(60);
  const c = columns();
  const limit = () => shelf().rows * c.cols();
  const shown = () => props.list.slice(0, picked() ? count() : limit());
  return (
    <View class="gap-[10px]" testID={`shelf-${props.id}`}>
      <View class="flex-row items-baseline gap-2 px-[2px]">
        <Text class="text-[12.5px] font-semibold text-foreground">{shelf().name}</Text>
        <Text class="text-[11px] text-faint">{String(props.list.length)}</Text>
        <View class="flex-1" />
        <If when={!picked() && props.list.length > limit()}>
          <View role="button" testID={`shelf-all-${props.id}`} onPress={() => shelfPick.set(props.id)} class="flex-row items-center gap-1">
            <Text class="text-[12px] text-primary">{tr("See all")}</Text>
            <Icon name="chev-right" color="primary" size={11} />
          </View>
        </If>
      </View>
      <Grid list={shown()} c={c} />
      <If when={picked() && props.list.length > count()}>
        <TextButton label={tr("Show the next %d", Math.min(60, props.list.length - count()))} onPress={() => count.set(count() + 60)} />
      </If>
    </View>
  );
}

/** The bar over the shelves: the kind (with how many of each), the source and the order. */
function BrowseBar(props: { all: MarketEntry[] }) {
  const n = (id: Shelf | "") => props.all.filter((m) => !id || m.shelf === id).length;
  const shelves = () => SHELVES().filter((s) => n(s.id) > 0);
  return (
    <View class="flex-row flex-wrap items-center gap-2">
      <Segs
        testID="market-shelf"
        value={shelfPick()}
        onChange={(v) => shelfPick.set(v)}
        options={[{ value: "" as Shelf | "", label: `${tr("All")} ${n("")}` }, ...shelves().map((s) => ({ value: s.id as Shelf | "", label: `${s.short} ${n(s.id)}` }))]}
      />
      <View class="flex-1" />
      <Dropdown
        testID="market-source"
        width={130}
        value={sourcePick()}
        onChange={(v) => sourcePick.set(v)}
        options={[
          { value: "", label: tr("All sources") },
          { value: "ours", label: tr("magpie community") },
          { value: "third", label: tr("Other authors") },
        ]}
      />
      <Dropdown
        testID="market-sort"
        width={110}
        value={sortPick()}
        onChange={(v) => sortPick.set(v)}
        options={[
          { value: "hot", label: tr("Most popular") },
          { value: "name", label: tr("Name") },
        ]}
      />
    </View>
  );
}

/** A package, a repository or a folder the market doesn't list. */
function Manual() {
  const spec = signal("");
  return (
    <View class="flex-row items-center gap-2 rounded-[12px] border border-dashed border-border px-3 py-[10px]">
      <Text class="text-[12px] font-medium text-fg-2">{tr("Have one in mind?")}</Text>
      <Input
        value={spec()}
        onChange={(v) => spec.set(v)}
        onSubmit={() => install(spec())}
        placeholder={tr("npm package, GitHub repo (github:owner/repo) or a folder")}
        testID="plugin-spec"
        class="flex-1"
      />
      <If
        when={installing() && installing() === spec().trim()}
        else={<PrimaryButton label={tr("Install")} disabled={!!installing() || !spec().trim()} onPress={() => install(spec())} testID="plugin-add" />}
      >
        <Doing label={tr("Installing…")} testID="plugin-adding" />
      </If>
    </View>
  );
}

/** What a plugin is to the reader, and how one is written, pinned under the list with Manual. */
function Footer() {
  const docs = () => `https://usemagpie.ai/docs/${lang() === "zh" ? "zh/" : ""}plugins`;
  return (
    <View class="flex-row flex-wrap items-center gap-x-[14px] gap-y-[6px] px-[2px]">
      <View class="flex-1 flex-row items-center gap-[6px]">
        <Icon name="pm-shield" color="success" size={12} />
        <Text class="text-[11.5px] text-muted-foreground">
          {tr("A plugin is someone else's code with your sign-in or your requests: install the ones you trust.")}
        </Text>
      </View>
      <View role="button" onPress={() => void openUrl(docs())} class="flex-row items-center gap-1">
        <Text class="text-[12px] text-primary">{tr("Write a plugin")}</Text>
        <Icon name="pm-out" color="primary" size={11} />
      </View>
    </View>
  );
}

/** Cards still to come, laid out as a card is, in soft bars. */
function Ghosts(props: { n: number }) {
  const c = columns();
  return (
    <View testID="plugins-discover" class="flex-row flex-wrap gap-[10px]" onLayout={c.measure}>
      <Each of={Array.from({ length: props.n }, (_, i) => i)} key={(i) => String(i)}>
        {() => (
          <View class="gap-[9px] rounded-[12px] border border-border/70 bg-card p-3" style={{ width: c.cardW() }}>
            <View class="flex-row items-center gap-[10px]">
              <View class="h-9 w-9 rounded-[10px] bg-pill" />
              <View class="flex-1 gap-[6px]">
                <View class="h-[10px] w-[55%] rounded-[5px] bg-pill" />
                <View class="h-[8px] w-[35%] rounded-[5px] bg-pill" />
              </View>
            </View>
            <View class="h-[9px] w-full rounded-[5px] bg-pill" />
            <View class="h-[9px] w-[70%] rounded-[5px] bg-pill" />
          </View>
        )}
      </Each>
    </View>
  );
}

/** Discover: this week's picks, the browse bar and the shelves; a search's matches and npm's, while one is typed. */
function Discover() {
  const q = () => query().trim().toLowerCase();
  const all = () =>
    (market() ?? []).filter(
      (m) =>
        (!sourcePick() || (sourcePick() === "ours") === m.community) &&
        (!q() || [m.name, m.package, summaryOf(m), m.providers.join(" "), m.publisher].join(" ").toLowerCase().includes(q())),
    );
  const known = () => new Set((market() ?? []).map((m) => m.package));
  return (
    <If when={market()} else={<Ghosts n={6} />}>
      <If
        when={q()}
        else={
          <>
            <If when={!shelfPick() && !sourcePick()}>
              <ThisWeek all={market() ?? []} />
            </If>
            <BrowseBar all={all()} />
            <Each of={SHELVES().filter((s) => !shelfPick() || s.id === shelfPick())} key={(s) => s.id}>
              {(s) => (
                <If when={all().some((m) => m.shelf === s().id)}>
                  <ShelfView id={s().id} list={ranked(all().filter((m) => m.shelf === s().id))} />
                </If>
              )}
            </Each>
            <If when={!all().length}>
              <View class="items-center gap-[6px] rounded-[12px] border border-dashed border-border px-3 py-[22px]">
                <Text class="text-[12px] text-muted-foreground">{tr("No plugin matches")}</Text>
                <TextButton label={tr("Show all")} onPress={() => (shelfPick.set(""), sourcePick.set(""))} />
              </View>
            </If>
          </>
        }
      >
        <If when={all().length}>
          <View class="gap-[10px]">
            <Text class="px-[2px] text-[12.5px] font-semibold text-foreground">{tr("Plugins")}</Text>
            <Grid list={ranked(all(), "hot").slice(0, 60)} />
          </View>
        </If>
        <View class="gap-[10px]">
          <View class="flex-row flex-wrap items-baseline gap-2 px-[2px]">
            <Text class="text-[12.5px] font-semibold text-foreground">{tr("On npm")}</Text>
            <Text class="text-[11.5px] text-muted-foreground">{tr("OpenCode plugins and pi packages anyone published — read what one does before you install it")}</Text>
          </View>
          <If when={q().length < 2}>
            <Text class="px-[2px] text-[12px] text-muted-foreground">{tr("Type two letters or more to search npm")}</Text>
          </If>
          <If when={q().length >= 2 && !hits()?.list && !hits()?.error}>
            <Ghosts n={2} />
          </If>
          <If when={hits()?.error}>
            <Text class="px-[2px] text-[12px] text-destructive">{hits()?.error ?? ""}</Text>
          </If>
          <If when={hits()?.list}>
            <If
              when={(hits()?.list ?? []).some((m) => !known().has(m.package))}
              else={<Text class="px-[2px] text-[12px] text-muted-foreground">{all().length ? tr("Nothing else on npm") : tr("No plugin called “%s”", query().trim())}</Text>}
            >
              <Grid list={(hits()?.list ?? []).filter((m) => !known().has(m.package))} />
            </If>
          </If>
        </View>
      </If>
    </If>
  );
}

/** Asks npm a moment after the typing stops, for two letters or more. */
function useSearch() {
  effect(() => {
    const s = query().trim();
    if (s.length < 2) {
      hits.set(null);
      return;
    }
    hits.set({ q: s });
    const timer = setTimeout(() => {
      Plugins.search(s).then(
        (text) => hits()?.q === s && hits.set({ q: s, list: JSON.parse(text) as MarketEntry[] }),
        (error) => hits()?.q === s && hits.set({ q: s, error: reason(error) }),
      );
    }, 350);
    return () => clearTimeout(timer);
  });
}

// --- Installed ---------------------------------------------------------------

/** A provider a plugin signs in to, on its row: signed in as whom, or not, and Sign out. */
function ProviderLine(props: { provider: PluginProvider }) {
  const p = () => props.provider;
  return (
    <View class="gap-[2px]">
      <View class="flex-row items-center gap-[6px]">
        <View class={`h-[6px] w-[6px] rounded-full ${p().accounts.length ? "bg-success" : "bg-faint"}`} />
        <Text class="text-[11.5px] text-muted-foreground">
          {p().accounts.length ? tr("%s: signed in", p().name) : tr("%s: not signed in", p().name)}
          {` · ${tr("%d models", p().models)}`}
        </Text>
      </View>
      <If when={p().listError}>
        <Text class="pl-3 text-[11px] text-warning">{tr("Its models couldn't be listed now: %s", p().listError)}</Text>
      </If>
      <Each of={p().accounts} key={(a) => a.key}>
        {(a) => (
          <View class="flex-row items-center gap-2 pl-3">
            <Text class="text-[11.5px] text-fg-2 line-clamp-1">{a().user || a().key}</Text>
            <If when={working()[`${p().id}:${a().key}`]}>
              <Doing label={working()[`${p().id}:${a().key}`]!} />
            </If>
            <TextButton
              label={tr("Sign out")}
              tone="muted"
              disabled={!!working()[`${p().id}:${a().key}`]}
              onPress={() =>
                void (async () => {
                  if (await ask({ title: tr("Sign out %s?", a().user || a().key), description: tr("The plugin forgets this account's sign-in."), confirm: tr("Sign out"), danger: true })) {
                    await busyWith(`${p().id}:${a().key}`, tr("Signing out…"), () => Plugins.signOut(p().id, a().key));
                  }
                })()
              }
            />
          </View>
        )}
      </Each>
    </View>
  );
}

/** A plugin added, as magpie's Installed row: its picture, name, version and kinds, what it serves and how it runs; Sign in, Options, Update, Switch off, Remove. */
function InstalledRow(props: { plugin: PluginRow; first: boolean }) {
  const p = () => props.plugin;
  const stats = () => (p().middleware ? pluginStats()[p().middleware!] : undefined);
  const open = () => editing()?.spec === p().spec;
  const listed = () => (market() ?? []).find((m) => m.package === p().package);
  const kinds = () => p().kinds.filter((k) => k !== "provider" || p().providers.length > 0);
  const outs = () => p().providers.filter((x) => x.accounts.length === 0);
  const serves = () => p().providers.map((x) => x.name);
  const remove = async () => {
    if (await ask({ title: tr("Remove %s?", listed()?.name ?? p().package), description: tr("Removes the plugin and what it installed; its sign-ins are kept until you sign out"), confirm: tr("Remove"), danger: true })) {
      await busyWith(p().spec, tr("Removing…"), () => Plugins.remove(p().spec), tr("%s removed", listed()?.name ?? p().package));
    }
  };
  /** What the row is busy with (Updating…, Removing…). */
  const doing = () => working()[p().spec] ?? "";
  /** The provider of this plugin being signed in to, whose strip is under the row. */
  const signing = () => p().providers.find((x) => x.id === signingIn()?.id);
  return (
    <View>
    <Row first={props.first} class={`relative items-start py-[10px] ${p().off && !doing() ? "opacity-70" : ""}`} testID={`plugin-${p().name}`}>
      <If when={doing()}>
        <Working />
      </If>
      <Logo busy={!!doing()} icon={p().icon || listed()?.icon || brandOf(p().providers.map((x) => x.id)) || (p().kinds.includes("pi") ? "pi" : "")} kind={p().kinds.includes("middleware") || p().kinds.includes("hooks") ? (p().providers.length ? "provider" : "middleware") : p().kinds.includes("agent") && !p().providers.length ? "agent" : "provider"} />
      <View class="min-w-0 flex-1 gap-[4px]">
        <View class="flex-row flex-wrap items-center gap-[7px]">
          <Text class="text-[13px] font-medium text-foreground">{listed()?.name ?? (p().path ? p().name : p().package)}</Text>
          <If when={p().version}>
            <Text class="font-mono text-[10.5px] text-muted-foreground">{`v${p().version}`}</Text>
          </If>
          <Each of={kinds()} key={(k) => k}>
            {(k) => <KindChip kind={k()} />}
          </Each>
          <If when={serves().length && !p().off}>
            <View class="h-[17px] flex-row items-center gap-1 rounded-[5px] bg-primary/15 px-[6px]">
              <View class="h-[5px] w-[5px] rounded-full bg-primary" />
              <Text class="text-[10.5px] font-medium text-primary">{tr("Serves %s", serves().join(", "))}</Text>
            </View>
          </If>
        </View>
        <If when={p().off}>
          <Text class="text-[11.5px] text-muted-foreground">{tr("Off")}</Text>
        </If>
        <If when={!p().off && p().error}>
          <Text class="text-[11.5px] text-destructive">{tr("Didn't load: %s", p().error ?? "")}</Text>
        </If>
        <If when={!p().off && !p().error}>
          <Each of={p().providers} key={(x) => x.id}>
            {(x) => <ProviderLine provider={x()} />}
          </Each>
          <If when={stats()}>
            <Text class="text-[11.5px] text-muted-foreground">
              {tr("Gateway middleware: %s", stats()!.hooks.join(", "))}
              {stats()!.calls ? ` · ${tr("%d calls", stats()!.calls)}` : ""}
              {stats()!.failures ? ` · ${tr("%d failed", stats()!.failures)}` : ""}
            </Text>
            <If when={stats()!.lastError}>
              <Text class="text-[11px] text-warning">{tr("Last failure: %s", stats()!.lastError)}</Text>
            </If>
          </If>
          <If when={p().kinds.includes("middleware") || p().kinds.includes("hooks")}>
            <Text class="text-[11px] text-faint">{tr("Sees what agents send before it is masked, and runs with your permissions.")}</Text>
          </If>
          <Each of={p().agents} key={(a) => a.id}>
            {(a) => <Text class="text-[11.5px] text-muted-foreground">{tr("Adds the agent %s: connect it on the Agents page", a().name)}</Text>}
          </Each>
        </If>
        <If when={open()}>
          <View class="gap-[6px] pt-1">
            <Input multiline mono value={editing()?.text ?? ""} onChange={(v) => editing.set({ spec: p().spec, text: v })} testID={`plugin-options-${p().name}`} />
            <If when={p().example}>
              <Text class="text-[11px] text-faint">{tr("Its example: %s", JSON.stringify(p().example))}</Text>
            </If>
            <View class="flex-row gap-2">
              <PrimaryButton
                label={tr("Save")}
                onPress={() =>
                  void busyWith(p().spec, tr("Saving…"), () => Plugins.setOptions(p().spec, editing()?.text || "{}"), tr("Saved")).then((ok) => {
                    if (ok) editing.set(null);
                  })
                }
              />
              <TextButton label={tr("Cancel")} onPress={() => editing.set(null)} />
            </View>
          </View>
        </If>
      </View>
      <If when={!doing()} else={<Doing label={doing()} testID={`plugin-busy-${p().name}`} />}>
      <View class="flex-row items-center gap-[2px]">
        <If when={!p().off && !p().error && outs().length > 0}>
          <TextButton
            label={outs().length === p().providers.length ? tr("Sign in") : tr("Sign in to %s", outs()[0]!.name)}
            tone={outs().length === p().providers.length ? "accent" : "default"}
            testID={`plugin-sign-in-${outs()[0]!.id}`}
            onPress={() => beginSignIn(outs()[0]!)}
          />
        </If>
        <If when={!p().off && p().providers.some((x) => x.accounts.length > 0) && outs().length === 0}>
          <TextButton label={tr("Add another account")} onPress={() => beginSignIn(p().providers[0]!)} />
        </If>
        <If when={!p().off && !open() && (p().kinds.includes("middleware") || p().kinds.includes("hooks") || Object.keys(p().options).length > 0 || p().example)}>
          <TextButton label={tr("Options")} onPress={() => editing.set({ spec: p().spec, text: JSON.stringify(Object.keys(p().options).length ? p().options : (p().example ?? {}), null, 2) })} />
        </If>
        <If when={!p().path}>
          <TextButton label={tr("Update")} onPress={() => void busyWith(p().spec, tr("Updating…"), () => Plugins.update(p().spec), tr("Updated %s", p().package))} />
        </If>
        <TextButton label={p().off ? tr("Switch on") : tr("Switch off")} testID={`plugin-on-${p().name}`} onPress={() => void busyWith(p().spec, p().off ? tr("Switching on…") : tr("Switching off…"), () => Plugins.setOn(p().spec, p().off))} />
        <TextButton label={tr("Remove")} tone="muted" onPress={() => void remove()} />
      </View>
      </If>
    </Row>
    <If when={signing()}>
      <SigningStrip provider={signing()!} spec={p().package || p().spec} />
    </If>
    </View>
  );
}

/** Whether a plugin not yet added is being installed (one already added is updated in its row). */
const installingNew = () => !!installing() && !plugins().some((p) => p.package === installing() || p.spec === installing());

/** The plugin being installed, as its row will be: its picture and name, soft bars where its lines will be, and a spinner. */
function InstallingRow() {
  const m = () => (market() ?? []).find((x) => x.package === installing());
  return (
    <Row first class="relative items-start py-[10px]" testID="plugin-installing">
      <Working />
      <Logo kind={m() ? kindOf(m()!) : "provider"} icon={m() ? iconOf(m()!) : ""} busy />
      <View class="min-w-0 flex-1 gap-[7px] pt-[2px]">
        <Text class="text-[13px] font-medium text-foreground">{m()?.name ?? installing()}</Text>
        <View class="h-[9px] w-[46%] rounded-[5px] bg-pill motion-safe:animate-pulse" />
        <View class="h-[9px] w-[28%] rounded-[5px] bg-pill motion-safe:animate-pulse" />
      </View>
      <Doing label={tr("Installing…")} />
    </Row>
  );
}

/** Installed: a row for each plugin added; none yet, a way to Discover (and magpie's to bring over). */
function Installed() {
  const found = () => magpieFound();
  return (
    <>
      <If when={found() && plugins().length === 0}>
        <List>
          <Row first>
            <View class="min-w-0 flex-1 gap-[1px]">
              <Text class="text-[13px] font-medium text-foreground">{tr("Bring magpie's plugins over")}</Text>
              <Text class="text-[11.5px] text-muted-foreground line-clamp-1">{tr("In %s: %d plugins and %d sign-ins", found()!.dir, found()!.plugins.length, found()!.accounts)}</Text>
            </View>
            <PrimaryButton
              label={tr("Bring over")}
              disabled={isBusy("plugin-import")}
              onPress={() =>
                void act("plugin-import", async () => {
                  const done = JSON.parse(await Plugins.importMagpie()) as { added: string[]; failed: [string, string][]; accounts: number };
                  toast(tr("%d plugins added, %d sign-ins copied", done.added.length, done.accounts));
                  for (const [spec, error] of done.failed) toast(`${spec}: ${error}`, "error");
                  await reload();
                })
              }
            />
          </Row>
        </List>
      </If>
      <If
        when={plugins().length || installingNew()}
        else={
          <View class="items-center gap-[10px] py-10">
            <Logo kind="provider" size={44} />
            <Text class="text-[13px] font-semibold text-foreground">{tr("No plugins yet")}</Text>
            <Text class="text-[12px] text-muted-foreground">{tr("Install a subscription or a middleware from Discover; a subscription signs in from here.")}</Text>
            <PrimaryButton label={tr("Discover plugins")} onPress={() => pluginTab.set("discover")} testID="plugins-to-discover" />
          </View>
        }
      >
        <List testID="plugins-list">
          <If when={installingNew()}>
            <InstallingRow />
          </If>
          <Each of={plugins()} key={(p) => p.spec}>
            {(p, i) => <InstalledRow plugin={p()} first={i() === 0 && !installingNew()} />}
          </Each>
        </List>
        <InstalledFoot />
      </If>
    </>
  );
}

/** Installed's foot, under the list: the Bun plugins run on, list again, update all. */
function InstalledFoot() {
  return (
    <View class="flex-row items-center gap-2 px-[2px]">
      <Text class="flex-1 text-[11.5px] text-muted-foreground">{tr("Plugins run on Bun %s", "1.4.2")}</Text>
      <If
        when={isBusy("plugin-refresh")}
        else={<TextButton label={tr("List their providers again")} disabled={isBusy("plugin-update")} onPress={() => void act("plugin-refresh", () => Plugins.refresh()).then(reload)} />}
      >
        <Doing label={tr("Listing…")} />
      </If>
      <If
        when={isBusy("plugin-update")}
        else={<TextButton label={tr("Update all")} disabled={isBusy("plugin-refresh")} onPress={() => void act("plugin-update", () => Plugins.update(""), tr("Plugins updated")).then(reload)} />}
      >
        <Doing label={tr("Updating…")} />
      </If>
    </View>
  );
}

/** The sign-in under way (`signingIn` is its provider): its way picked, starting, or the plugin's flow. */
const signStep = signal<"method" | "starting" | "flow">("method");
const signFlow = signal<PluginFlow | null>(null);
const signMethod = signal(0);
const signAnswer = signal("");

/** Opens a provider's sign-in under its plugin's row in Installed, as magpie's: its way asked first when it has several. */
function beginSignIn(provider: PluginProvider) {
  cancelSignIn();
  signingIn.set(provider);
  if (provider.methods.length > 1) signStep.set("method");
  else startSignIn(0);
}

function cancelSignIn() {
  const f = signFlow();
  if (f?.id && !["done", "failed", "canceled"].includes(f.state)) Plugins.cancel(f.id);
  signFlow.set(null);
  signAnswer.set("");
  signingIn.set(null);
}

async function settleSignIn(f: PluginFlow) {
  const p = signingIn();
  if (!p) return;
  signFlow.set(f);
  signStep.set("flow");
  signAnswer.set("");
  if (f.state === "done") {
    signFlow.set(null);
    signingIn.set(null);
    toast(tr("Signed in to %s", p.name));
    await reload();
  }
  if (f.state === "waiting" && !f.opened && f.url) void openUrl(f.url);
}

function startSignIn(method: number) {
  const p = signingIn();
  if (!p) return;
  signMethod.set(method);
  signFlow.set(null);
  signStep.set("starting");
  void Plugins.signIn(p.id, method).then(
    (text) => settleSignIn(JSON.parse(text) as PluginFlow),
    (error) => {
      if (signingIn()?.id !== p.id) return;
      signStep.set("flow");
      signFlow.set({ id: "", provider: p.id, method, state: "failed", prompt: null, invalid: "", url: "", instructions: "", opened: false, account: "", error: reason(error) });
    },
  );
}

function answerSignIn(value: string) {
  const f = signFlow();
  if (!f || isBusy("plugin-sign-in")) return;
  void act("plugin-sign-in", async () => settleSignIn(JSON.parse(await Plugins.answer(f.id, value)) as PluginFlow));
}

/** A way or an answer to pick, as magpie's choice buttons. */
function Choice(props: { label: string; onPress: () => void; hint?: string; testID?: string }) {
  return (
    <View role="button" testID={props.testID} onPress={props.onPress} class="min-h-[26px] justify-center rounded-[7px] border border-border bg-card px-[10px] py-1 hover:bg-pill">
      <Text class="text-[12px] font-medium text-foreground">{props.label}</Text>
    </View>
  );
}

/** The sign-in strip's lines: its title, and what is said under it. */
const N = (props: { text: string }) => <Text class="text-[12.5px] font-medium leading-[17px] text-foreground">{props.text}</Text>;
const S = (props: { text: string; bad?: boolean }) => <Text class={`text-[11.5px] leading-[16px] ${props.bad ? "text-destructive" : "text-muted-foreground"}`}>{props.text}</Text>;

/** magpie's sign-in strip (`.signing`), under its plugin's row: a spinner and what to do in the browser with its link, the way to sign in, the plugin's questions, or why it didn't finish. */
function SigningStrip(props: { provider: PluginProvider; spec: string; framed?: boolean }) {
  const p = () => props.provider;
  const f = () => signFlow();
  const state = () => (signStep() === "flow" ? (f()?.state ?? "starting") : signStep());
  const turning = () => ["starting", "waiting", "code"].includes(state());
  const prompt = () => f()?.prompt;
  /** The key's title: the way's own label, unless it only says "API key". */
  const keyTitle = () => {
    const way = p().methods[signMethod()];
    const own = (way?.label ?? "").trim();
    return own && own.toLowerCase() !== "api key" ? labelOf(own, way?.labels) : tr("%s API key", p().name);
  };
  effect(() => {
    const now = f();
    if (!now || now.state !== "waiting") return;
    const timer = setInterval(() => {
      const next = JSON.parse(Plugins.status(now.id)) as PluginFlow | null;
      if (next && next.state !== "waiting") void settleSignIn(next);
    }, 1000);
    return () => clearInterval(timer);
  });
  const field = (label: string, masked: boolean, placeholder: string | undefined, button: string, testID: string) => (
    <View class="flex-row items-center gap-2 pt-[6px]">
      <Input
        value={signAnswer()}
        onChange={(v) => signAnswer.set(v)}
        onSubmit={() => answerSignIn(signAnswer())}
        masked={masked}
        placeholder={placeholder ?? label}
        autoFocus
        testID={testID}
        class="flex-1"
      />
      <If when={!isBusy("plugin-sign-in")} else={<Spinner size={14} />}>
        <TextButton label={button} tone="accent" disabled={!signAnswer().trim()} onPress={() => answerSignIn(signAnswer())} testID="plugin-sign-in-next" />
      </If>
    </View>
  );
  return (
    <View testID="plugin-sign-in" class={`flex-row items-start gap-[11px] bg-card-2 py-[11px] pl-[14px] pr-[10px] ${props.framed ? "" : "border-t border-line-2"}`}>
      <If when={turning()}>
        <View class="pt-[1px]">
          <Spinner size={16} color="foreground" />
        </View>
      </If>
      <If when={state() === "failed"}>
        <View class="mt-[1px] h-4 w-4 items-center justify-center rounded-full bg-destructive">
          <Text class="text-[11px] font-bold text-card">!</Text>
        </View>
      </If>
      <View class="min-w-0 flex-1 gap-[2px]">
        <If when={state() === "method"}>
          <N text={tr("How do you sign in to %s?", p().name)} />
          <S text={tr("The plugin %s signs in and sends %s's requests; pansy only passes them on.", props.spec, p().name)} />
          <View class="flex-row flex-wrap gap-[6px] pt-[6px]">
            <Each of={p().methods} key={(m) => m.label}>
              {(m, i) => <Choice label={labelOf(m().label, m().labels) || (m().type === "api" ? tr("API key") : tr("Browser"))} onPress={() => startSignIn(i())} testID={`plugin-sign-in-way-${i()}`} />}
            </Each>
          </View>
        </If>
        <If when={turning()}>
          <N text={tr("Finish signing in to %s in your browser", p().name)} />
          <S
            text={
              state() === "starting"
                ? tr("Starting the sign-in…")
                : f()?.instructions || (state() === "code" ? tr("pansy opened the sign-in page. Paste the code it shows below.") : tr("pansy opened the sign-in page. The account shows up here as soon as you're done."))
            }
          />
          <If when={f()?.url}>
            <View class="flex-row items-center gap-1 pb-[1px] pt-[5px]">
              <View class="min-w-0 flex-1 rounded-[6px] border border-line-2 bg-card px-[7px] py-1">
                <Text class="font-mono text-[11px] text-fg-2 line-clamp-1">{f()!.url}</Text>
              </View>
              <CopyButton label={tr("Copy link")} onCopy={() => Clipboard.writeText(f()!.url)} />
            </View>
            <View class="-ml-1 flex-row flex-wrap gap-[2px] pt-[2px]">
              <TextButton label={tr("Open again")} tone="muted" onPress={() => void openUrl(f()!.url)} testID="plugin-sign-in-open" />
            </View>
          </If>
          <If when={state() === "code"}>{field(tr("Code"), false, undefined, tr("Finish sign-in"), "plugin-sign-in-code")}</If>
        </If>
        <If when={state() === "asking"}>
          <N text={prompt()?.message ?? ""} />
          <If
            when={prompt()?.options?.length}
            else={field(prompt()?.message ?? "", prompt()?.type === "password", prompt()?.placeholder, tr("Next"), "plugin-sign-in-answer")}
          >
            <View class="flex-row flex-wrap gap-[6px] pt-[6px]">
              <Each of={prompt()?.options ?? []} key={(o) => o.value}>
                {(o) => <Choice label={o().label} hint={o().hint} onPress={() => answerSignIn(o().value)} />}
              </Each>
            </View>
          </If>
        </If>
        <If when={state() === "key"}>
          <N text={keyTitle()} />
          {field(keyTitle(), true, p().methods[signMethod()]?.placeholder, tr("Sign in"), "plugin-sign-in-key")}
        </If>
        <If when={f()?.invalid && state() !== "failed"}>
          <S text={f()!.invalid} bad />
        </If>
        <If when={state() === "failed"}>
          <N text={tr("Sign-in didn't finish")} />
          <S text={f()?.error ?? ""} />
        </If>
      </View>
      <TextButton label={tr("Cancel")} onPress={cancelSignIn} testID="plugin-sign-in-cancel" />
      <If when={state() === "failed"}>
        <TextButton label={tr("Try again")} tone="accent" onPress={() => (p().methods.length > 1 ? signStep.set("method") : startSignIn(signMethod()))} testID="plugin-sign-in-start" />
      </If>
    </View>
  );
}

/** magpie's 「国内镜像」: Bun and plugins asked of a mirror in China first; the plugin list needs none (pansy's hub is reached from China as it is). */
function MirrorSwitch() {
  const on = () => settings()?.chinaMirror === true;
  return (
    <Hint text={tr("Download plugins and Bun from a mirror in China first (npmmirror), their official addresses after. Each is a copy of the very file, checked as the original is")}>
      <View class="flex-row items-center gap-[7px]">
        <Text class={`text-[12px] ${on() ? "text-foreground" : "text-muted-foreground"}`}>{tr("Mirrors in China")}</Text>
        <Switch
          testID="plugins-mirror"
          label={tr("Mirrors in China")}
          checked={on()}
          disabled={!settings() || isBusy("setting:chinaMirror")}
          onChange={(v) =>
            void setSetting("chinaMirror", v).then((ok) => {
              if (ok && v) void loadMarket(true);
            })
          }
        />
      </View>
    </Hint>
  );
}

/** Models › Plugins: Discover and Installed under one head, with the search field. */
export function PluginsPage() {
  useSearch();
  let scroller: ScrollRef | undefined;
  // The offset changes every frame of a scroll; the capsule changes only at
  // its thresholds, so only those go into a signal and a gesture redraws
  // nothing in between.
  const raw = { y: 0, view: 0, content: 0 };
  const ends = signal({ shown: false, top: true, bottom: false });
  const measure = () => {
    const end = Math.max(0, raw.content - raw.view);
    const next = { shown: raw.view > 0 && end > 0 && raw.y > raw.view / 2, top: raw.y < 1, bottom: raw.y >= end - 2 };
    const was = ends();
    if (next.shown !== was.shown || next.top !== was.top || next.bottom !== was.bottom) ends.set(next);
  };
  const signing = () => signingIn() && plugins().find((p) => !p.off && p.providers.some((x) => x.id === signingIn()!.id));
  return (
    <View class="flex-1">
      <View class="flex-1">
        <Scroll
          ref={(r) => (scroller = r)}
          class="flex-1"
          onScroll={(e) => {
            raw.y = e.offset.y;
            raw.view = e.size.height;
            raw.content = e.contentSize.height;
            measure();
          }}
          onLayout={(e) => {
            raw.view = e.bounds.size.height;
            measure();
          }}
        >
          <View testID="page-plugins" class="gap-[10px] pb-4 pl-4 pr-4 pt-[2px]" onLayout={(e) => {
              raw.content = e.bounds.size.height;
              measure();
            }}>
            <PluginsHead />
            <If when={pluginTab() === "installed"} else={<Discover />}>
              <Installed />
            </If>
          </View>
        </Scroll>
        <ScrollEnds ends={ends()} to={(bottom) => scroller?.scrollTo({ y: bottom ? Math.max(0, raw.content - raw.view) : 0, animated: true })} />
      </View>
      <If when={pluginTab() === "discover" && market() && !query().trim()}>
        <View testID="plugins-foot" class="gap-2 border-t border-line-2 bg-background px-4 pb-3 pt-[10px]">
          <If when={signing()}>
            <View class="overflow-hidden rounded-[10px] border border-line-2">
              <SigningStrip provider={signingIn()!} spec={signing()!.package || signing()!.spec} framed />
            </View>
          </If>
          <Manual />
          <Footer />
        </View>
      </If>
    </View>
  );
}

/**
 * magpie's Scroll to top and Scroll to bottom (scrollends.js): a quiet
 * capsule at the page's lower right, there once the page is half a screen
 * down (to its foot too: the cards' corner under it is empty, and Write a
 * plugin is in the pinned foot), each end greyed when the page is at it.
 */
function ScrollEnds(props: { ends: { shown: boolean; top: boolean; bottom: boolean }; to: (bottom: boolean) => void }) {
  return (
    <If when={props.ends.shown}>
      <View
        class="absolute bottom-3 right-4 items-center rounded-[9px] border border-line-2 bg-popover p-[2px]"
        style={{ boxShadow: SHADOW.pop() }}
      >
        <IconButton icon="chev-up" label={tr("Scroll to top")} disabled={props.ends.top} onPress={() => props.to(false)} testID="plugins-to-top" />
        <View class="my-[1px] h-px w-[14px] bg-line-2" />
        <IconButton icon="chev-down" label={tr("Scroll to bottom")} disabled={props.ends.bottom} onPress={() => props.to(true)} testID="plugins-to-bottom" />
      </View>
    </If>
  );
}

/** The page's head: Discover or Installed, mirrors, look again, and the search field. */
function PluginsHead() {
  return (
      <View class="flex-row items-center gap-[10px]">
        <Segs
          testID="plugins-tab"
          value={pluginTab()}
          onChange={(v) => pluginTab.set(v)}
          options={[
            { value: "discover", label: tr("Discover") },
            { value: "installed", label: plugins().length ? `${tr("Installed")} · ${plugins().length}` : tr("Installed") },
          ]}
        />
        <View class="flex-1" />
        <MirrorSwitch />
        <IconButton icon="refresh-cw" label={tr("Look again")} onPress={() => act("plugin-market", () => Promise.all([loadMarket(true), reload()]))} />
        <Input
          value={query()}
          onChange={(v) => {
            query.set(v);
            if (v.trim()) pluginTab.set("discover");
          }}
          placeholder={tr("Search plugins and npm…")}
          testID="market-find"
          class="w-[260px]"
        />
      </View>
  );
}

