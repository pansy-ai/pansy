// The pansy window, laid out as magpie's: a sidebar of pages in groups
// (Work, Models with Plugins, Insights) with Settings — and its own pages, while it is
// open — and the gateway's state at its foot, a title bar with where the
// page is, the page, and what sits above them (modals, toasts). The title
// bar's button hides the sidebar, and its pages become tabs in the title bar.
// State lives in ui/model.ts; providers, agents and the gateway themselves
// in pansy/native (native/).
import { effect, keymap, LayoutAnimation, mount, platform, reduceMotion, Scroll, setColorScheme, signal, Text, token, UrlScheme, View } from "nsh";
import { Button } from "nsh/base";
import { currentWindow, isAvailable, Tray, Window } from "nsh/desktop";
import { AgentModals } from "./ui/AgentModals";
import { PreviewDialog } from "./ui/Preview";
import { AgentsPage } from "./ui/Agents";
import { Brand } from "./ui/Brand";
import { GatewayPage } from "./ui/Gateway";
import { GroupsPage } from "./ui/Groups";
import { fontClass, tr, trn } from "./ui/i18n";
import { ICONS } from "./ui/icons";
import { MaskingPage, maskingCrumb } from "./ui/Masking";
import { fmtN, gateway, go, loadPage, loadTrace, maskOpen, page, routes, sessionOpen, setSideShown, settings, settingsQuery, settingsTab, sideShown, start, usageToday, type Page } from "./ui/model";
import { QuestionDialog, Toasts } from "./ui/Overlays";
import { togglePanel } from "./ui/Panel";
import { Hint, Icon, IconButton, Input, MOTION, SHADOW, type IconName } from "./ui/parts";
import { PluginsPage } from "./ui/Plugins";
import { openLink, ProviderModals, ProvidersPage } from "./ui/Providers";
import { RoutingPage } from "./ui/Routing";
import { sessionCrumb, SessionsPage } from "./ui/Sessions";
import { SETTINGS_TABS, SettingsPage } from "./ui/Settings";
import { startUpdates, UpdatePill } from "./ui/Update";
import { UsagePage } from "./ui/Usage";
import { WhatsNewDialog } from "./ui/WhatsNew";

/** Each page's place: the group it is under in the sidebar, and its name. */
const PLACES = (): Record<Page, { group: string; name: string; icon: IconName }> => ({
  agents: { group: tr("Work"), name: tr("Agents"), icon: "nav-agents" },
  sessions: { group: tr("Work"), name: tr("Sessions"), icon: "nav-sessions" },
  providers: { group: tr("Models"), name: tr("Providers"), icon: "nav-providers" },
  routing: { group: tr("Models"), name: tr("Routing"), icon: "nav-routing" },
  groups: { group: tr("Models"), name: tr("Routing groups"), icon: "nav-groups" },
  gateway: { group: tr("Models"), name: tr("Gateway"), icon: "nav-gateway" },
  plugins: { group: tr("Models"), name: tr("Plugins"), icon: "nav-plugins" },
  usage: { group: tr("Insights"), name: tr("Usage"), icon: "nav-usage" },
  masking: { group: tr("Insights"), name: tr("Masking"), icon: "shield" },
  settings: { group: tr("Settings"), name: SETTINGS_TABS().find((t) => t.id === settingsTab())?.name ?? tr("General"), icon: "nav-settings" },
});

/** Whether the sidebar lists `p` (Settings › General › Sidebar pages); the open page always. */
const shown = (p: Page) => page() === p || !(settings()?.hiddenPages ?? []).includes(p);

function NavItem(props: { page: Page; label?: string }) {
  const active = () => page() === props.page;
  const place = () => PLACES()[props.page];
  return (
    <View
      role="button"
      testID={`nav-${props.page}`}
      accessibilityLabel={props.label ?? place().name}
      onPress={() => go(props.page)}
      class={`h-[30px] flex-row items-center gap-[9px] rounded-[7px] px-2 ${active() ? "bg-sidebar-accent" : "hover:bg-pill-hover"}`}
      style={active() ? { boxShadow: SHADOW.thumb() } : undefined}
    >
      <Icon name={place().icon} color={active() ? "foreground" : "fg-2"} size={props.page === "settings" ? 14 : 16} />
      <Text class={`flex-1 text-[13px] ${active() ? "font-medium text-foreground" : "text-fg-2"}`}>{props.label ?? place().name}</Text>
    </View>
  );
}

/** Settings' own pages, over Settings in the sidebar while it is open: indented, on a line. */
function SettingsNav() {
  return (
    <View testID="settings-nav" class="mb-1 ml-[15px] gap-px border-l border-border pl-[8px]">
      <Each of={SETTINGS_TABS()} key={(t) => t.id}>
        {(t) => {
          const active = () => settingsTab() === t().id && !settingsQuery().trim();
          return (
            <View
              role="button"
              testID={`settings-tab-${t().id}`}
              onPress={() => {
                settingsQuery.set("");
                settingsTab.set(t().id);
              }}
              class={`h-[26px] flex-row items-center gap-2 rounded-[6px] px-2 ${active() ? "bg-sidebar-accent" : "hover:bg-pill-hover"}`}
              style={active() ? { boxShadow: SHADOW.thumb() } : undefined}
            >
              <Icon name={t().icon} color={active() ? "foreground" : "muted-foreground"} size={13} />
              <Text class={`text-[12.5px] ${active() ? "font-medium text-foreground" : "text-fg-2"}`}>{t().name}</Text>
            </View>
          );
        }}
      </Each>
    </View>
  );
}

function GroupLabel(props: { text: string; first?: boolean }) {
  return (
    <View class={`px-2 pb-1 pt-[2px] ${props.first ? "" : "mt-[14px]"}`}>
      <Text class="text-[11px] font-medium text-muted-foreground">{props.text}</Text>
    </View>
  );
}

/** The gateway's state at the foot of the sidebar. */
function GatewayLine() {
  const state = () => {
    const g = gateway();
    if (!g) return { dot: "bg-faint", ring: "pill" as const, label: tr("Checking the gateway…") };
    if (g.running) return { dot: "bg-success", ring: "success-soft" as const, label: tr("Gateway %s", g.url.replace(/^https?:\/\//, "")) };
    return { dot: "bg-destructive", ring: "destructive-soft" as const, label: tr("Gateway stopped") };
  };
  return (
    <View role="button" testID="gateway-line" onPress={() => go("gateway")} class="h-[26px] flex-row items-center gap-2 rounded-[6px] px-[9px] hover:bg-pill-hover">
      <View class={`h-[7px] w-[7px] rounded-full ${state().dot}`} style={{ boxShadow: `0 0 0 3px ${token(state().ring)}` }} />
      <Text testID="gateway-state" class="flex-1 text-[11.5px] text-muted-foreground line-clamp-1">
        {state().label}
      </Text>
    </View>
  );
}

/** The sidebar's keystroke, as the Mac's View menu has it. */
const SIDE_KEY = platform.os === "macos" ? "ctrl+cmd+s" : "ctrl+shift+s";

/** The sidebar is drawn: while it shows, and while it slides in or away. */
const sideDrawn = signal(sideShown());

/**
 * Shows the sidebar or hides it, as one motion as magpie's (app.js navMorph):
 * it slides in from the window's edge or back to it, a third of a second
 * from rest to rest, while the page moves over and the title bar's tabs fade
 * out or in. At once for less motion.
 */
function toggleSidebar() {
  const on = !sideShown();
  if (reduceMotion()) {
    sideDrawn.set(on);
    setSideShown(on);
    return;
  }
  const morph = () =>
    LayoutAnimation.configureNext(
      { duration: 320, update: { type: "easeInEaseOut" }, create: { type: "easeInEaseOut", property: "opacity" }, delete: { type: "easeInEaseOut", property: "opacity" } },
      () => sideDrawn.set(sideShown()),
    );
  if (!on) {
    morph();
    setSideShown(false);
    return;
  }
  // drawn first at no width, off the window's edge, to slide in from there
  sideDrawn.set(true);
  requestAnimationFrame(() => {
    morph();
    setSideShown(true);
  });
}

/** The sidebar's button, first in the title bar as a Mac app has it (magpie's #sideToggle): its drawing's left pane filled while the sidebar shows. */
function SideToggle() {
  const label = () => `${sideShown() ? tr("Hide sidebar") : tr("Show sidebar")} (${keymap.label(SIDE_KEY)})`;
  return (
    <Hint text={label()}>
      <Button
        onPress={toggleSidebar}
        accessibilityLabel={sideShown() ? tr("Hide sidebar") : tr("Show sidebar")}
        testID="side-toggle"
        class={`group h-[26px] w-[26px] items-center justify-center rounded-[7px] hover:bg-pill active:scale-90 ${MOTION}`}
      >
        <View style={{ width: 15, height: 15 }}>
          <Icon name="sidebar" color="muted-foreground" hover="foreground" size={15} />
          <View class={`absolute left-0 top-0 transition-opacity duration-200 ${sideShown() ? "opacity-[0.32]" : "opacity-0"}`}>
            <Icon name="sidebar-pane" color="muted-foreground" hover="foreground" size={15} />
          </View>
        </View>
      </Button>
    </Hint>
  );
}

function Sidebar() {
  return (
    <View testID="sidebar" class="overflow-hidden" style={{ width: sideShown() ? 212 : 0 }}>
      <If when={sideDrawn()}>
        <View class="absolute bottom-0 right-0 top-0 w-[212px] border-r border-border bg-sidebar">
          <View class="h-[46px] flex-row items-center gap-2 pl-[14px] pr-[18px]">
            <SideToggle />
            <Brand />
          </View>
          <Scroll class="flex-1">
            <View class="gap-px px-[10px]">
              <If when={shown("agents") || shown("sessions")}>
                <GroupLabel text={tr("Work")} first />
              </If>
              <If when={shown("agents")}>
                <NavItem page="agents" />
              </If>
              <If when={shown("sessions")}>
                <NavItem page="sessions" />
              </If>
              <If when={shown("providers") || shown("routing") || shown("groups") || shown("gateway") || shown("plugins")}>
                <GroupLabel text={tr("Models")} first={!shown("agents") && !shown("sessions")} />
              </If>
              <If when={shown("providers")}>
                <NavItem page="providers" />
              </If>
              <If when={shown("routing")}>
                <NavItem page="routing" />
              </If>
              <If when={shown("groups")}>
                <NavItem page="groups" />
              </If>
              <If when={shown("gateway")}>
                <NavItem page="gateway" />
              </If>
              <If when={shown("plugins")}>
                <NavItem page="plugins" />
              </If>
              <If when={shown("usage") || shown("masking")}>
                <GroupLabel text={tr("Insights")} />
              </If>
              <If when={shown("usage")}>
                <NavItem page="usage" />
              </If>
              <If when={shown("masking")}>
                <NavItem page="masking" />
              </If>
            </View>
          </Scroll>
          <View class="gap-px border-t border-line-2 px-[10px] pb-[10px] pt-2">
            <If when={page() === "settings"}>
              <SettingsNav />
            </If>
            <NavItem page="settings" label={tr("Settings")} />
            <GatewayLine />
          </View>
        </View>
      </If>
    </View>
  );
}

/** The pages as tabs in the title bar while the sidebar is hidden (magpie's .seg): no track, the open one on a raised thumb; Settings is the gear beside them. */
const TAB_PAGES: Page[] = ["agents", "sessions", "providers", "routing", "groups", "gateway", "plugins", "usage", "masking"];

function NavTabs() {
  return (
    <View testID="nav-tabs" class="shrink flex-row items-center gap-px">
      <Each of={TAB_PAGES.filter(shown)} key={(p) => p}>
        {(p) => {
          const active = () => page() === p();
          return (
            <View
              role="tab"
              testID={`tab-${p()}`}
              onPress={() => go(p())}
              class={`group h-[24px] shrink items-center justify-center rounded-[6px] px-3 active:scale-96 ${active() ? "bg-seg-thumb" : ""} ${MOTION}`}
              style={active() ? { boxShadow: SHADOW.thumb() } : undefined}
            >
              <Text class={`text-[12.5px] line-clamp-1 ${active() ? "font-medium text-foreground" : "text-ctl group-hover:text-foreground"} ${MOTION}`}>{PLACES()[p()].name}</Text>
            </View>
          );
        }}
      </Each>
    </View>
  );
}

/** Back from a session or a masked request, to the list it is in. */
function Back(props: { testID: string; label: string; onPress: () => void }) {
  return (
    <View role="button" testID={props.testID} onPress={props.onPress} class="h-[24px] flex-row items-center gap-1 rounded-[6px] pr-[6px] hover:bg-pill">
      <Icon name="chevron-left" color="muted-foreground" size={14} />
      <Text class="text-[13.5px] text-muted-foreground">{props.label}</Text>
    </View>
  );
}

/** A session open on Sessions, a request open on Masking: the title bar's way back. */
const inSession = () => page() === "sessions" && sessionOpen() !== null;
const inRequest = () => page() === "masking" && maskOpen() !== null;

/** Where the page is: "Models / Providers", or the way back from a session or a request. */
function Crumb() {
  const place = () => PLACES()[page()];
  return (
    <If
      when={inSession()}
      else={
        <If
          when={inRequest()}
          else={
            <>
              <Text class="text-[13.5px] text-muted-foreground">{place().group}</Text>
              <Text class="text-[13.5px] text-faint">/</Text>
              <Text testID="page-title" class="text-[13.5px] font-semibold text-foreground">
                {settingsQuery().trim() && page() === "settings" ? tr("Find a setting") : place().name}
              </Text>
            </>
          }
        >
          <Back testID="masking-back" label={tr("Masking")} onPress={() => maskOpen.set(null)} />
          <Text class="text-[13.5px] text-faint">/</Text>
          <Text testID="page-title" class="text-[13.5px] font-semibold text-foreground">
            {maskingCrumb()}
          </Text>
        </If>
      }
    >
      <Back testID="sessions-back" label={tr("Sessions")} onPress={() => sessionOpen.set(null)} />
      <Text class="text-[13.5px] text-faint">/</Text>
      <Text testID="page-title" class="shrink text-[13.5px] font-semibold text-foreground line-clamp-1">
        {sessionCrumb()}
      </Text>
    </If>
  );
}

/**
 * The title bar beside the sidebar: where the page is (magpie's crumb), and
 * what to do on any page; Settings' has "Find a setting". With the sidebar
 * hidden: its button and the mark, the pages as tabs, and Settings' gear.
 */
function Header() {
  return (
    <View class={`h-[46px] flex-row items-center gap-[7px] pr-[10px] ${sideShown() ? "pl-5" : "pl-[14px]"}`}>
      <If
        when={sideShown()}
        else={
          <>
            <SideToggle />
            <View class="ml-px">
              <Brand />
            </View>
            <If when={inSession()}>
              <View class="ml-2">
                <Back testID="sessions-back" label={tr("Sessions")} onPress={() => sessionOpen.set(null)} />
              </View>
            </If>
            <If when={inRequest()}>
              <View class="ml-2">
                <Back testID="masking-back" label={tr("Masking")} onPress={() => maskOpen.set(null)} />
              </View>
            </If>
            <View class="flex-1" />
            <NavTabs />
          </>
        }
      >
        <Crumb />
      </If>
      <View class="flex-1" />
      <If when={sideShown() && page() === "settings"}>
        <Input value={settingsQuery()} onChange={(v) => settingsQuery.set(v)} placeholder={tr("Find a setting")} testID="settings-find" class="w-[220px]" />
      </If>
      <UpdatePill testID="update" hide />
      <IconButton icon="sync" label={tr("Look again: agents installed or removed, settings changed")} testID="sync" onPress={() => loadPage(page())} />
      <If when={!sideShown()}>
        <IconButton icon="nav-settings" label={tr("Settings")} color={page() === "settings" ? "foreground" : undefined} testID="tab-settings" onPress={() => go("settings")} />
      </If>
    </View>
  );
}

function Content() {
  return (
    <>
      <If when={page() === "agents"}>
        <AgentsPage />
      </If>
      <If when={page() === "sessions"}>
        <SessionsPage />
      </If>
      <If when={page() === "providers"}>
        <ProvidersPage />
      </If>
      <If when={page() === "routing"}>
        <RoutingPage />
      </If>
      <If when={page() === "groups"}>
        <GroupsPage />
      </If>
      <If when={page() === "gateway"}>
        <GatewayPage />
      </If>
      <If when={page() === "plugins"}>
        <PluginsPage />
      </If>
      <If when={page() === "usage"}>
        <UsagePage />
      </If>
      <If when={page() === "masking"}>
        <MaskingPage />
      </If>
      <If when={page() === "settings"}>
        <SettingsPage />
      </If>
    </>
  );
}

/**
 * The tray icon's tooltip, as magpie's: the name over what went through
 * pansy today, every token split into input, output and cache, then the
 * requests.
 */
function trayTip(): string {
  const t = usageToday();
  if (!t) return "Pansy";
  if (t.requests === 0) return `Pansy\n${tr("Today: no requests yet")}`;
  const cached = t.cacheRead + t.cacheWrite;
  const parts = [tr("%s in", fmtN(Math.max(0, t.input - cached))), tr("%s out", fmtN(t.output)), ...(cached ? [tr("%s cached", fmtN(cached))] : [])];
  const tokens = tr("%s tokens (%s)", fmtN(t.input + t.output), parts.join(" · "));
  return `Pansy\n${tr("Today: %s", tokens)}\n${trn(t.requests, "%d request", "%d requests", t.requests)}`;
}

/** The tray icon's pinwheel: a turn in TURNS frames, a frame every SPIN_MS. */
const TURNS = 12;
const SPIN_MS = 80;

/**
 * pansy's mark turned `step` frames of a turn about where its petals meet,
 * in black (the panels tint it), framed on that point so it turns in place.
 */
function trayIcon(step: number): string {
  const svg = ICONS.pansy.replace('viewBox="81 42 960 960"', 'viewBox="80 8 960 960"')
    .replace(/(<svg[^>]*>)/, `$1<g transform="rotate(${(step * 360) / TURNS} 560 488)">`)
    .replace("</svg>", "</g></svg>")
    .replaceAll("currentColor", "#000000");
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

/**
 * Turns the tray icon as a pinwheel: once round at a click on it, as
 * magpie's bird flaps, and on and on while any request is out through the
 * gateway (its trace, read twice a second), winding to the end of the turn
 * it is on once none is; still, with less motion asked for. Gives the frame
 * to show, and what turns it once.
 */
function useTrayTurn(): { frame: () => number; once: () => void } {
  const out = signal(false);
  const frame = signal(0);
  setInterval(() => {
    try {
      loadTrace();
    } catch {
      return;
    }
    out.set(routes().some((r) => !r.done));
  }, 500);
  /** Till when a click wants it turning (ms). */
  let until = 0;
  let turning: ReturnType<typeof setInterval> | undefined;
  const turn = () => {
    if (turning || reduceMotion()) return;
    turning = setInterval(() => {
      const next = (frame() + 1) % TURNS;
      if (next === 0 && ((!out() && Date.now() >= until) || reduceMotion())) {
        clearInterval(turning);
        turning = undefined;
      }
      frame.set(next);
    }, SPIN_MS);
  };
  effect(() => {
    if (out()) turn();
  });
  const once = () => {
    until = Date.now() + (TURNS - 1) * SPIN_MS;
    turn();
  };
  return { frame, once };
}

/**
 * The tray icon: a click opens the quick panel by it, or brings the window
 * back (Settings › Click the tray icon to); its menu opens the pages and
 * quits. Closing the window keeps pansy in the tray (Settings), so the
 * gateway goes on serving the agents. It turns at a click, and while a
 * request is out.
 */
function useTray() {
  if (!isAvailable) return;
  const win = currentWindow();
  win.on("closePress", () => {
    if (settings()?.closeToTray === false) {
      win.close();
    } else {
      win.hide();
    }
  });
  const open = (p?: Page) => {
    Window.get("panel")?.close();
    if (p) go(p);
    win.show();
    win.focus();
  };
  const icons = Array.from({ length: TURNS }, (_, i) => trayIcon(i));
  const { frame, once } = useTrayTurn();
  effect(() => {
    Tray.set({
      icon: icons[frame()],
      tooltip: trayTip(),
      onClick: () => {
        once();
        if (settings()?.trayClick === "window") open();
        else void togglePanel();
      },
      menu: [
        { label: tr("Open Pansy"), onSelect: () => open() },
        { type: "separator" },
        { label: tr("Agents"), onSelect: () => open("agents") },
        { label: tr("Sessions"), onSelect: () => open("sessions") },
        { label: tr("Providers"), onSelect: () => open("providers") },
        { label: tr("Usage"), onSelect: () => open("usage") },
        { label: tr("Masking"), onSelect: () => open("masking") },
        { type: "separator" },
        { role: "quit", label: tr("Quit Pansy") },
      ],
    });
  });
}

function App() {
  start();
  startUpdates();
  useTray();
  // a pansy://import?… link the user opened: the Providers page asks first
  UrlScheme.on("open", ({ url }) => openLink(url));
  keymap.bind(SIDE_KEY, toggleSidebar);
  effect(() => {
    setColorScheme(settings()?.theme ?? "system");
  });
  return (
    <View class={`flex-1 flex-row bg-background ${fontClass()}`}>
      <Sidebar />
      <View class="flex-1">
        <Header />
        <Content />
      </View>
      <ProviderModals />
      <AgentModals />
      <PreviewDialog />
      <WhatsNewDialog />
      <QuestionDialog />
      <Toasts />
    </View>
  );
}

mount(App);
