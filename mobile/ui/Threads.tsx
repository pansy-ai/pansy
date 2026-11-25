// 对话: every machine's threads in one list (phone-ui.md "1. 对话"), as T3's
// home: the header (pansy, or the machines it is reconnecting to; search),
// the filters, the rows in progress, then the snoozed and settled shelves;
// a held row's menu.
import { cleanup, effect, Image, Scroll, signal, Text, token, untracked, View } from "nsh";
import { ContextMenu, MenuItem, NavPage } from "nsh/base";
import type { Machine } from "../net/client";
import type { ThreadSummary } from "../net/types";
import { connState, entries, machines, ready, shells, wakeAll, type Entry } from "../state/store";
import { archive, copyLink, markUnread, pin, remove, renaming, snooze, tillTomorrow } from "./actions";
import { tr } from "./i18n";
import { ICONS } from "./icons";
import { agentLogo, agentName, AddDel, BLUR, Btn, Chip, Field, GlassButton, Icon, Logo, PickSheet, ProjectMark, Spinner, type IconName } from "./parts";
import { newTask, openThread, pairOpen } from "./nav";
import { Status, statusLook } from "./status";
import { now, since, tick } from "./time";

/** T3's shelves: the rows in progress (no head), snoozed, settled. A settled thread is one the user archived. */
type Shelf = "active" | "snoozed" | "settled";

export function shelfOf(t: ThreadSummary, at = Date.now()): Shelf {
  if (t.archived) return "settled";
  if (t.snoozedUntil && Date.parse(t.snoozedUntil) > at) return "snoozed";
  return "active";
}

/** T3's settled shelf: ten rows, then 25 more each "Show more". */
export const SETTLED_FIRST = 10;
export const SETTLED_PAGE = 25;

const project = (t: ThreadSummary) => t.folder.split(/[\\/]/).filter(Boolean).pop() ?? t.folder;

/** The filters: a machine, a project, an agent; null for all. */
const filter = { machine: signal<string | null>(null), project: signal<string | null>(null), agent: signal<string | null>(null) };
const query = signal("");
const searching = signal(false);
const collapsed = signal<Record<Shelf, boolean>>({ active: false, snoozed: true, settled: false });
const settledShown = signal(SETTLED_FIRST);
const picking = signal<"machine" | "project" | "agent" | null>(null);

function visible(): Entry[] {
  const q = query().trim().toLowerCase();
  return entries()
    .filter((e) => !filter.machine() || e.machine.id === filter.machine())
    .filter((e) => !filter.project() || project(e.thread) === filter.project())
    .filter((e) => !filter.agent() || e.thread.agent === filter.agent())
    .filter((e) => !q || e.thread.title.toLowerCase().includes(q))
    .sort((a, b) => Number(!!b.thread.pinned) - Number(!!a.thread.pinned) || Date.parse(b.thread.updatedAt) - Date.parse(a.thread.updatedAt));
}

/** What the title says while no machine is online (T3's `workspace-connection-status`): who it is reconnecting to, or that none is connected. */
export function connectionTitle(states: { name: string; state: string }[]): { label: string; spin: boolean } | null {
  if (states.length === 0 || states.some((m) => m.state === "online")) return null;
  const trying = states.filter((m) => m.state === "connecting" || m.state === "offline");
  const spin = trying.some((m) => m.state === "connecting");
  if (trying.length === 1) return { label: tr("Reconnecting to %s", trying[0].name), spin };
  if (trying.length > 1) return { label: tr("Reconnecting %d machines", trying.length), spin };
  return { label: tr("Not connected"), spin: false };
}

/** T3 waits this long before the title says so, which hides a brief reconnect. */
const STATUS_DELAY = 800;

/** pansy's mark and name, or the machines it is reconnecting to. */
function Title() {
  const status = () => connectionTitle(machines().map((m) => ({ name: m.name, state: connState(m.id) })));
  // when the status run began; a gap restarts the delay
  const since = signal<number | null>(null);
  effect(() => {
    const on = !!status();
    if (on !== (untracked(since) !== null)) since.set(on ? Date.now() : null);
  });
  const shown = () => {
    const at = since();
    return at !== null && now() - at >= STATUS_DELAY ? status() : null;
  };
  return (
    <If
      when={shown()}
        else={
          <View class="flex-row items-center gap-2">
            <Image src={`data:image/svg+xml,${encodeURIComponent(ICONS.pansy.replaceAll("currentColor", token("primary")))}`} style={{ width: 20, height: 20 }} />
            <Text class="text-[21px] font-medium tracking-[-0.5px] text-fg-2">pansy</Text>
          </View>
        }
      >
        <View testID="connection-title" class="min-w-0 flex-1 flex-row items-center gap-2">
          <If when={shown()?.spin} else={<Icon name="wifi-off" color="fg-2" size={15} />}>
            <Spinner size={16} color="fg-2" />
          </If>
          <Text class="shrink text-[16px] font-bold text-fg-2 line-clamp-1">{shown()?.label ?? ""}</Text>
        </View>
    </If>
  );
}

/** Pansy or the status on the left; search alone on the right, as T3's (compose floats beside the tab bar); nothing to search before a pairing. */
function Header() {
  return (
    <View class="h-[52px] flex-row items-center gap-2 px-4">
      <Title />
      <View class="flex-1" />
      <If when={machines().length > 0}>
        <GlassButton icon={searching() ? "x" : "search"} label={searching() ? tr("Close search") : tr("Search")} testID="threads-search" onPress={() => {
          if (searching()) query.set("");
          searching.set(!searching());
        }} size={44} />
      </If>
    </View>
  );
}

function Filters() {
  const name = (id: string | null) => machines().find((m) => m.id === id)?.name;
  return (
    <Scroll horizontal class="grow-0" scrollbar={false}>
      <View class="flex-row items-center gap-2 px-5 pb-[10px] pt-[2px]">
        <Chip label={name(filter.machine()) ?? tr("All machines")} on={!!filter.machine()} chevron testID="filter-machine" onPress={() => picking.set("machine")} />
        <Chip label={filter.project() ?? tr("All projects")} on={!!filter.project()} chevron testID="filter-project" onPress={() => picking.set("project")} />
        <Chip label={filter.agent() ? agentName(filter.agent()!) : tr("All agents")} on={!!filter.agent()} chevron testID="filter-agent" onPress={() => picking.set("agent")} />
        <If when={filter.machine() || filter.project() || filter.agent()}>
          <View
            role="button"
            accessibilityLabel={tr("Reset thread filters")}
            testID="filter-reset"
            class="h-8 w-8 items-center justify-center active:opacity-70"
            onPress={() => {
              filter.machine.set(null);
              filter.project.set(null);
              filter.agent.set(null);
            }}
          >
            <Icon name="circle-x" color="fg-3" size={18} />
          </View>
        </If>
      </View>
    </Scroll>
  );
}

function FilterSheets() {
  const all = <T extends string>(label: string, values: { value: T; label: string; logo?: ReturnType<typeof agentLogo> }[]) => [{ value: "" as T, label }, ...values];
  const uniq = (xs: string[]) => [...new Set(xs)].sort();
  return (
    <>
      <PickSheet
        open={picking() === "machine"}
        onClose={() => picking.set(null)}
        title={tr("Machine")}
        testID="pick-machine"
        value={filter.machine() ?? ""}
        onPick={(v) => filter.machine.set(v || null)}
        options={all(tr("All machines"), machines().map((m) => ({ value: m.id, label: m.name })))}
      />
      <PickSheet
        open={picking() === "project"}
        onClose={() => picking.set(null)}
        title={tr("Project")}
        testID="pick-project"
        value={filter.project() ?? ""}
        onPick={(v) => filter.project.set(v || null)}
        options={all(tr("All projects"), uniq(entries().map((e) => project(e.thread))).map((p) => ({ value: p, label: p })))}
      />
      <PickSheet
        open={picking() === "agent"}
        onClose={() => picking.set(null)}
        title={tr("Agent")}
        testID="pick-agent"
        value={filter.agent() ?? ""}
        onPick={(v) => filter.agent.set(v || null)}
        options={all(tr("All agents"), uniq(entries().map((e) => e.thread.agent)).map((a) => ({ value: a, label: agentName(a), logo: agentLogo(a) })))}
      />
    </>
  );
}

/** A shelf's head: its name, a hairline to the edge, a chevron that turns. */
function ShelfHead(props: { shelf: Shelf; label: string; count: number }) {
  const shut = () => collapsed()[props.shelf];
  return (
    <View
      role="button"
      testID={`shelf-${props.shelf}`}
      onPress={() => collapsed.set({ ...collapsed(), [props.shelf]: !shut() })}
      class="mb-[6px] mt-4 flex-row items-center gap-[10px] px-5"
    >
      <Text class="text-[12px] font-medium text-fg-3">{shut() ? `${props.label} (${props.count})` : props.label}</Text>
      <View class="h-px flex-1 bg-border" />
      <View style={{ transform: [{ rotate: shut() ? "0deg" : "180deg" }] }} class="transition-transform duration-180">
        <Icon name="chevron-down" color="fg-3" size={10} />
      </View>
    </View>
  );
}

/** The row's menu: on a held finger (a secondary click on the desktops). */
function RowMenu(props: { machine: Machine; thread: ThreadSummary }) {
  const item = (label: string, icon: IconName, onSelect: () => void, testID: string, danger = false) => (
    <MenuItem onSelect={onSelect} testID={testID} class="min-h-[44px] flex-row items-center gap-3 rounded-[10px] px-4 py-[10px] focus:bg-line active:bg-line">
      <Text class={`flex-1 text-[16px] ${danger ? "text-danger" : "text-fg"}`}>{label}</Text>
      <Icon name={icon} color={danger ? "danger" : "fg"} size={18} />
    </MenuItem>
  );
  const m = () => props.machine.id;
  const t = () => props.thread;
  return (
    <>
      {item(t().pinned ? tr("Unpin") : tr("Pin"), t().pinned ? "pin-off" : "pin", () => void pin(m(), t()), "menu-pin")}
      {item(tr("Rename"), "pencil", () => renaming.set({ machine: m(), thread: t() }), "menu-rename")}
      {item(tr("Mark as unread"), "circle", () => void markUnread(m(), t()), "menu-unread")}
      {item(t().archived ? tr("Unsettle") : tr("Settle"), t().archived ? "archive-restore" : "archive", () => void archive(m(), t(), !t().archived), "menu-settle")}
      <If when={shelfOf(t()) === "snoozed"} else={item(tr("Snooze until tomorrow morning"), "clock", () => void snooze(m(), t(), tillTomorrow()), "menu-snooze")}>
        {item(tr("Unsnooze"), "clock", () => void snooze(m(), t(), null), "menu-unsnooze")}
      </If>
      {item(tr("Copy link"), "link", () => void copyLink(m(), t()), "menu-copy")}
      {item(tr("Delete thread record"), "trash-2", () => void remove(m(), t()), "menu-delete", true)}
    </>
  );
}

function Row(props: { entry: Entry; first: boolean }) {
  const t = () => props.entry.thread;
  const m = () => props.entry.machine;
  const busy = () => statusLook(t())?.busy ?? false;
  const fade = () => ({ opacity: busy() ? 0.55 : 1 });
  const row = (
    <View role="button" testID={`thread-${t().id}`} onPress={() => openThread(m().id, t().id)} class="gap-[3px] px-5 py-[10px] active:bg-line">
      <If when={!props.first}>
        <View class="absolute left-5 right-0 top-0 h-px bg-line" />
      </If>
      <View class="flex-row items-start gap-2">
        <View class="mt-[2px]" style={fade()}>
          <ProjectMark name={project(t())} />
        </View>
        <Text class="min-w-0 flex-1 text-[16px] font-medium leading-[23px] text-fg line-clamp-2" style={fade()}>
          {t().title || tr("Untitled")}
        </Text>
        <If when={t().pinned}>
          <View class="mt-[5px]">
            <Icon name="pin" color="fg-3" size={12} />
          </View>
        </If>
        <Status thread={t()} testID={`status-${t().id}`} />
      </View>
      <If
        when={t().status === "failed" && t().error}
        else={
          <View class="flex-row items-center gap-[6px] overflow-hidden pl-7" style={fade()}>
            <If when={t().source === "terminal"} else={<Icon name={m().os === "linux" ? "server" : "laptop"} color="fg-3" size={13} />}>
              <Icon name="terminal" color="fg-3" size={13} />
            </If>
            <Text class="text-[13px] text-fg-3 line-clamp-1 shrink">{t().source === "terminal" ? tr("Opened in a terminal") : m().name}</Text>
            <If when={t().branch}>
              <Text class="text-[13px] text-fg-3 opacity-50">·</Text>
              <Text class="font-mono text-[12px] text-fg-3 line-clamp-1 shrink">{t().branch ?? ""}</Text>
            </If>
            <Text class="text-[13px] text-fg-3 opacity-50">·</Text>
            <Logo name={agentLogo(t().agent)} size={13} />
            <Text class="text-[13px] text-fg-3 line-clamp-1 shrink">{t().model}</Text>
            <If when={t().pr}>
              <Text class="text-[13px] text-fg-3 opacity-50">·</Text>
              <Icon name={t().pr?.state === "merged" ? "git-merge" : t().pr?.state === "closed" ? "git-pull-request-closed" : "git-pull-request"} color={t().pr?.state === "merged" ? "violet" : t().pr?.state === "closed" ? "rose" : "emerald"} size={13} />
              <Text class="font-mono text-[12px] text-fg-3">#{t().pr?.number ?? ""}</Text>
            </If>
            <If when={t().diff && (t().diff!.add || t().diff!.del)}>
              <Text class="text-[13px] text-fg-3 opacity-50">·</Text>
              <AddDel add={t().diff?.add ?? 0} del={t().diff?.del ?? 0} />
            </If>
          </View>
        }
      >
        <Text class="pl-7 text-[13px] text-danger line-clamp-1">{t().error ?? ""}</Text>
      </If>
    </View>
  );
  return (
    <ContextMenu trigger={row} popupClass="min-w-[240px] rounded-[14px] border border-border bg-glass p-1" popupStyle={BLUR} backdropClass="bg-backdrop">
      <RowMenu machine={m()} thread={t()} />
    </ContextMenu>
  );
}

/** A settled or snoozed row (T3's slim row): 44pt, one line, the mark at 40%, the time in mono. */
function SlimRow(props: { entry: Entry }) {
  const t = () => props.entry.thread;
  return (
    <ContextMenu
      trigger={
        <View role="button" testID={`thread-${t().id}`} onPress={() => openThread(props.entry.machine.id, t().id)} class="min-h-[44px] flex-row items-center gap-2 px-5 py-2 active:bg-line">
          <ProjectMark name={project(t())} dim />
          <Text class="flex-1 text-[16px] text-fg-2 line-clamp-1">{t().title || tr("Untitled")}</Text>
          <Text class="font-mono text-[14px] text-fg-3">{since(t().updatedAt, now())}</Text>
        </View>
      }
      popupClass="min-w-[240px] rounded-[14px] border border-border bg-glass p-1"
      popupStyle={BLUR}
      backdropClass="bg-backdrop"
    >
      <RowMenu machine={props.entry.machine} thread={t()} />
    </ContextMenu>
  );
}

/** The rows in progress: no head, as T3's. */
function ActiveRows(props: { rows: Entry[] }) {
  return (
    <View testID="shelf-block-active">
      <Each of={props.rows} key={(e) => `${e.machine.id}/${e.thread.id}`}>
        {(e, i) => <Row entry={e()} first={i() === 0} />}
      </Each>
    </View>
  );
}

/** A shelf under its head: snoozed (shut until opened) or settled (its first rows, then "Show more"). */
function ShelfBlock(props: { shelf: Shelf; label: string; rows: Entry[]; limit?: number }) {
  const listed = () => (props.limit === undefined ? props.rows : props.rows.slice(0, props.limit));
  const hidden = () => props.rows.length - listed().length;
  return (
    <If when={props.rows.length > 0}>
      <View testID={`shelf-block-${props.shelf}`}>
        <ShelfHead shelf={props.shelf} label={props.label} count={props.rows.length} />
        <If when={!collapsed()[props.shelf]}>
          <Each of={listed()} key={(e) => `${e.machine.id}/${e.thread.id}`}>
            {(e) => <SlimRow entry={e()} />}
          </Each>
          <If when={hidden() > 0}>
            <View
              role="button"
              testID={`shelf-more-${props.shelf}`}
              onPress={() => settledShown.set(settledShown() + SETTLED_PAGE)}
              class="mx-4 mt-2 items-center rounded-lg border border-dashed border-border py-[10px] active:opacity-60"
            >
              <Text class="text-[12px] font-medium text-fg-2">{tr("Show more (%d settled hidden)", hidden())}</Text>
            </View>
          </If>
        </If>
      </View>
    </If>
  );
}

function Empty(props: { title: string; text: string; button: string; onPress: () => void; testID: string }) {
  return (
    <View testID={props.testID} class="items-center gap-3 px-8 pt-24">
      <Text class="text-center text-[21px] font-medium leading-[28px] text-fg">{props.title}</Text>
      <Text class="text-center text-[14px] text-fg-2">{props.text}</Text>
      <Btn label={props.button} onPress={props.onPress} class="mt-2 self-center" testID={`${props.testID}-button`} />
    </View>
  );
}

export function ThreadsPage() {
  cleanup(tick());
  // a new scope lists the settled shelf from its first rows again, as T3's
  effect(() => {
    void [filter.machine(), filter.project(), filter.agent(), query()];
    settledShown.set(SETTLED_FIRST);
  });
  const refreshing = signal(false);
  const byShelf = (s: Shelf) => visible().filter((e) => shelfOf(e.thread) === s);
  const anyShell = () => machines().some((m) => shells()[m.id] !== undefined);
  return (
    <NavPage title="pansy" headerVisible={false}>
      <View class="flex-1 bg-screen pt-[env(safe-area-inset-top)]">
        <Header />
        <Scroll
          class="flex-1"
          refreshing={refreshing()}
          onRefresh={() => {
            refreshing.set(true);
            wakeAll();
            setTimeout(() => refreshing.set(false), 600);
          }}
        >
          <If when={ready()}>
            <If
              when={machines().length > 0}
              else={<Empty testID="empty-machines" title={tr("Connect a computer")} text={tr("pansy on your computer runs the agents; this phone drives them. Pair once, from the computer's Settings › Sharing.")} button={tr("Add machine")} onPress={() => pairOpen.set(true)} />}
            >
              <If when={searching()}>
                <View class="px-5 pb-2 pt-1">
                  <Field value={query()} onChange={query.set} placeholder={tr("Search threads")} testID="search-field" autoFocus />
                </View>
              </If>
              <Filters />
              <If
                when={entries().length > 0 || !anyShell()}
                else={<Empty testID="empty-threads" title={tr("Start a task on %s", machines()[0]?.name ?? "")} text={tr("Pick a project and an agent, and say what to do.")} button={tr("New task")} onPress={() => newTask.set({})} />}
              >
                <View class="pb-32" testID="thread-list">
                  <ActiveRows rows={byShelf("active")} />
                  <ShelfBlock shelf="snoozed" label={tr("Snoozed")} rows={byShelf("snoozed")} />
                  <ShelfBlock shelf="settled" label={tr("Settled")} rows={byShelf("settled")} limit={settledShown()} />
                </View>
              </If>
            </If>
          </If>
        </Scroll>
      </View>
      <FilterSheets />
    </NavPage>
  );
}
