// 线程 (phone-ui.md "2. 线程"): the user's lines in bubbles, the agent's
// answers flat in Markdown, its work folded into rows, subagents, the
// approval card docked over the composer, the status capsule; the header's
// Git, changes and more. History and live events come from state/thread.ts.
import { cleanup, Clipboard, effect, Scroll, signal, Text, token, View, type JSX, type ScrollRef } from "nsh";
import { Menu, MenuItem, NavPage } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Decision, GitStatus, Mode, ThreadSummary } from "../net/types";
import { connState, machine, summary } from "../state/store";
import { ThreadView, type Approval, type Item } from "../state/thread";
import { archive, copyLink, renaming } from "./actions";
import { Accessory, Composer, type SendMode } from "./Composer";
import { DiffView } from "./Diff";
import { CommitSheet, GitSheet, type Way } from "./Git";
import { tr, trn } from "./i18n";
import { Markdown } from "./Markdown";
import { ModelSheet } from "./ModelSheet";
import { ModeSheet } from "./ModeSheet";
import { chats } from "./nav";
import { AddDel, BLUR, Btn, GLASS, GlassButton, Icon, Loading, Note, SHADOW, Spinner, toast, type Color, type IconName } from "./parts";
import { clock, now, tick, timer } from "./time";

const project = (t: ThreadSummary | undefined | null) => (t ? (t.folder.split(/[\\/]/).filter(Boolean).pop() ?? t.folder) : "");

/** A tool's icon and color by what it did. */
function toolLook(tool: string, title: string, status: string): { icon: IconName; color: Color } {
  const k = `${tool} ${title}`.toLowerCase();
  if (status === "failed") return { icon: "circle-x", color: "danger" };
  if (/\brm\b|delete|remove|删/.test(k)) return { icon: "trash-2", color: "rose" };
  if (/quota|额度/.test(k)) return { icon: "circle-alert", color: "warning" };
  if (/command|exec|bash|shell|terminal|跑|运行/.test(k)) return { icon: "terminal", color: "fg-2" };
  if (/read|view|读/.test(k)) return { icon: "file-text", color: "fg-2" };
  if (/edit|write|patch|change|改/.test(k)) return { icon: "pencil", color: "fg-2" };
  if (/search|grep|find|glob|搜/.test(k)) return { icon: "search", color: "fg-2" };
  if (/web|fetch|http|网/.test(k)) return { icon: "globe", color: "fg-2" };
  return { icon: "wrench", color: "fg-2" };
}

/** A row of the work log: 28pt, its icon, a line in 14pt, a chevron; open, what went in and came out. */
function WorkRow(props: { icon: IconName; color: Color; text: string; live?: boolean; detail?: string; testID?: string; tall?: number }) {
  const open = signal(false);
  return (
    <View>
      <View
        role="button"
        testID={props.testID}
        onPress={() => {
          if (!props.detail) return;
          open.set(!open());
          Haptics.selection();
        }}
        class="min-h-7 flex-row items-center gap-1"
      >
        <View class="h-6 w-6 items-center justify-center">
          <If when={props.live} else={<Icon name={props.icon} color={props.color} size={15} />}>
            <Spinner size={14} />
          </If>
        </View>
        <Text class={`min-w-0 flex-1 text-[14px] leading-[19px] line-clamp-1 text-${props.color} ${props.live ? "animate-pulse" : ""}`}>{props.text}</Text>
        <If when={props.detail}>
          <View class="transition-transform duration-180" style={{ transform: [{ rotate: open() ? "90deg" : "0deg" }] }}>
            <Icon name="chevron-right" color="fg-3" size={11} />
          </View>
        </If>
      </View>
      <If when={open()}>
        <Scroll class="ml-7 rounded-lg bg-grouped" style={{ maxHeight: props.tall ?? 240 }}>
          <Text class="px-[10px] py-2 font-mono text-[12px] leading-[17px] text-fg-2" style={{ userSelect: "text" }}>
            {props.detail ?? ""}
          </Text>
        </Scroll>
      </If>
    </View>
  );
}

function ToolItem(props: { item: Extract<Item, { kind: "tool" }> }) {
  const look = () => toolLook(props.item.tool, props.item.title, props.item.status);
  const detail = () => [props.item.input, props.item.output].filter(Boolean).join("\n\n") || undefined;
  return (
    <WorkRow
      testID={`tool-${props.item.id}`}
      icon={look().icon}
      color={props.item.status === "declined" ? "fg-3" : look().color}
      text={props.item.status === "declined" ? tr("%s (declined)", props.item.title) : props.item.title}
      live={props.item.status === "running"}
      detail={detail()}
    />
  );
}

function SubagentItem(props: { item: Extract<Item, { kind: "subagent" }> }) {
  const dot = () => (props.item.status === "running" ? "bg-sky" : props.item.status === "failed" ? "bg-danger" : "bg-emerald");
  return (
    <View testID={`subagent-${props.item.id}`} class="flex-row items-center gap-2 py-[6px]">
      <View class={`h-[6px] w-[6px] rounded-full ${dot()}`} />
      <Text class="text-[14px] font-medium text-fg">{props.item.name}</Text>
      <Text class="flex-1 text-[13px] text-fg-2 line-clamp-1">{props.item.detail}</Text>
      <Icon name="chevron-right" color="fg-3" size={12} />
    </View>
  );
}

type WorkItem = Extract<Item, { kind: "tool" | "reasoning" | "subagent" }>;

function WorkItemView(props: { item: WorkItem }) {
  const it = props.item;
  if (it.kind === "tool") return <ToolItem item={it} />;
  if (it.kind === "subagent") return <SubagentItem item={it} />;
  return <WorkRow icon="brain" color="fg-2" text={it.seconds ? tr("Thought for %d s", it.seconds) : tr("Thought")} detail={it.text} tall={384} />;
}

/** Work rows in a run; four or more finished tools fold into "Used N tools". */
function Work(props: { items: WorkItem[] }) {
  const open = signal(false);
  const fold = () => props.items.length >= 4 && props.items.every((i) => i.kind !== "subagent" && (i.kind !== "tool" || i.status !== "running"));
  return (
    <View class="gap-px">
      <If
        when={fold() && !open()}
        else={
          <Each of={props.items} key={(i) => `${i.id}:${JSON.stringify(i)}`}>
            {(i) => <WorkItemView item={i()} />}
          </Each>
        }
      >
        <View role="button" testID="work-fold" onPress={() => open.set(true)} class="min-h-7 flex-row items-center gap-1">
          <View class="h-6 w-6 items-center justify-center">
            <Icon name="layers" color="fg-2" size={15} />
          </View>
          <Text class="flex-1 text-[14px] text-fg-2">{tr("Used %d tools", props.items.length)}</Text>
          <Icon name="chevron-right" color="fg-3" size={11} />
        </View>
      </If>
    </View>
  );
}

type Block = { kind: "item"; item: Item } | { kind: "work"; id: string; items: WorkItem[] };

/** The feed's items with each run of work gathered. */
function blocksOf(items: Item[]): Block[] {
  const out: Block[] = [];
  for (const item of items) {
    if (item.kind === "tool" || item.kind === "reasoning" || item.kind === "subagent") {
      const last = out[out.length - 1];
      if (last?.kind === "work") last.items.push(item);
      else out.push({ kind: "work", id: `w${item.id}`, items: [item] });
    } else out.push({ kind: "item", item });
  }
  return out;
}

async function copy(text: string) {
  try {
    await Clipboard.writeText(text);
    Haptics.selection();
    toast(tr("Copied"));
  } catch {
    toast(tr("Couldn't copy"), { tone: "error" });
  }
}

function UserBubble(props: { item: Extract<Item, { kind: "user" }> }) {
  return (
    <View class="items-end gap-1" style={{ opacity: props.item.queued ? 0.6 : 1 }}>
      <View testID="user-bubble" class="rounded-[20px] bg-bubble px-[14px] py-[10px]" style={{ maxWidth: "85%" }}>
        <Text class="text-[16px] leading-[23px] text-fg" style={{ userSelect: "text" }}>
          {props.item.text}
        </Text>
      </View>
      <View class="flex-row items-center gap-[10px]">
        <If when={props.item.queued} else={<Text class="text-[13px] font-medium text-fg-3">{clock(props.item.at)}</Text>}>
          <Text class="text-[13px] font-medium text-fg-3">{tr("Queued")}</Text>
        </If>
        <View role="button" accessibilityLabel={tr("Copy")} onPress={() => void copy(props.item.text)} hitSlop={8}>
          <Icon name="copy" color="fg-3" size={14} />
        </View>
      </View>
    </View>
  );
}

function Answer(props: { item: Extract<Item, { kind: "assistant" }> }) {
  return (
    <View class="gap-2">
      <Markdown text={props.item.text} testID="answer" />
      <If when={!props.item.streaming}>
        <View class="flex-row items-center gap-[14px] px-1">
          <View role="button" accessibilityLabel={tr("Copy")} onPress={() => void copy(props.item.text)} hitSlop={8}>
            <Icon name="copy" color="fg-3" size={15} />
          </View>
          <Text class="text-[13px] font-medium text-fg-3">{clock(props.item.at)}</Text>
        </View>
      </If>
    </View>
  );
}

/** A block of the feed; its kind stays for its key, its content changes (props are read where they show). */
function FeedBlock(props: { block: Block }) {
  if (props.block.kind === "work") return <Work items={(props.block as Extract<Block, { kind: "work" }>).items} />;
  const item = <K extends Item["kind"]>() => (props.block as Extract<Block, { kind: "item" }>).item as Extract<Item, { kind: K }>;
  const it = item();
  switch (it.kind) {
    case "user":
      return <UserBubble item={item<"user">()} />;
    case "assistant":
      return <Answer item={item<"assistant">()} />;
    case "notice":
      return (
        <View class="flex-row items-center gap-2 px-1">
          <Icon name={it.tone === "danger" ? "circle-alert" : "square"} color={it.tone === "danger" ? "danger" : "fg-3"} size={14} />
          <Text class={`flex-1 text-[14px] ${it.tone === "danger" ? "text-danger" : "text-fg-3"}`}>{it.text === "stopped" ? tr("Stopped") : it.text}</Text>
        </View>
      );
    case "compact":
      return (
        <View class="flex-row items-center gap-2 py-1">
          <View class="h-px flex-1 bg-line" />
          <Text class="text-[12px] text-fg-3">{tr("Earlier turns compacted")}</Text>
          <View class="h-px flex-1 bg-line" />
        </View>
      );
    default:
      return <View />;
  }
}

/** An approval's title by what it asks for. */
function approvalTitle(a: Approval): string {
  const k = a.kind.toLowerCase();
  if (/command|exec|shell/.test(k)) return tr("Run a command");
  if (/file|patch|edit/.test(k)) {
    const n = a.detail.split("\n").filter((l) => l.trim()).length;
    return trn(n, "Change 1 file", "Change %d files", n);
  }
  if (/network|web/.test(k)) return tr("Reach the network");
  return a.title;
}

/** 需要批准: docked over the composer, not in the feed; several go 1 / N. */
function ApprovalCard(props: { approvals: Approval[]; index: number; onIndex: (i: number) => void; onAnswer: (a: Approval, d: Decision) => void }) {
  const a = () => props.approvals[Math.min(props.index, props.approvals.length - 1)];
  const long = signal(false);
  const lines = () => a()?.detail.split("\n") ?? [];
  return (
    <View testID="approval-card" class="gap-[10px] rounded-[20px] border border-border bg-card p-4" style={{ boxShadow: SHADOW.glass() }}>
      <View class="flex-row items-center justify-between">
        <Text class="text-[12px] font-bold uppercase tracking-[1.1px] text-fg-2">{tr("Needs approval")}</Text>
        <If when={props.approvals.length > 1}>
          <View class="flex-row items-center gap-2">
            <View role="button" accessibilityLabel={tr("Previous")} hitSlop={8} onPress={() => props.onIndex((props.index + props.approvals.length - 1) % props.approvals.length)}>
              <Icon name="chevron-left" color="fg-2" size={14} />
            </View>
            <Text testID="approval-count" class="text-[12px] font-bold text-fg-2">{`${Math.min(props.index, props.approvals.length - 1) + 1} / ${props.approvals.length}`}</Text>
            <View role="button" accessibilityLabel={tr("Next")} hitSlop={8} onPress={() => props.onIndex((props.index + 1) % props.approvals.length)}>
              <Icon name="chevron-right" color="fg-2" size={14} />
            </View>
          </View>
        </If>
      </View>
      <Text class="text-[18px] font-bold leading-[23px] text-fg">{a() ? approvalTitle(a()!) : ""}</Text>
      <If when={a()?.detail}>
        <View role="button" onPress={() => long.set(!long())} class="rounded-lg bg-grouped px-[10px] py-2">
          <Text class="font-mono text-[12.5px] leading-[18px] text-fg" style={long() ? undefined : { numberOfLines: 4 }}>
            {a()?.detail ?? ""}
          </Text>
          <If when={!long() && lines().length > 4}>
            <Text class="text-[12px] text-fg-3">{tr("%d more lines", lines().length - 4)}</Text>
          </If>
        </View>
      </If>
      <If when={a()?.reason}>
        <Text class="text-[13px] text-warning">{a()?.reason ?? ""}</Text>
      </If>
      <View class="gap-2">
        <Btn label={tr("Allow once")} onPress={() => props.onAnswer(a()!, "accept")} testID="approve-once" />
        <Btn tone="quiet" label={tr("Allow for this session")} onPress={() => props.onAnswer(a()!, "acceptForSession")} testID="approve-session" />
        <Btn tone="danger" label={tr("Decline")} onPress={() => props.onAnswer(a()!, "decline")} testID="approve-decline" />
      </View>
    </View>
  );
}

/** The capsule over the composer: working and its timer, the queue, the subagents; and back to the bottom. */
function Capsule(props: { view: ThreadView; queue: number; onBottom: () => void; showBottom: boolean }) {
  const subs = () => props.view.items().filter((i) => i.kind === "subagent" && i.status === "running").length;
  const running = () => props.view.running();
  const show = () => running() || props.queue > 0 || subs() > 0;
  return (
    <View class="flex-row items-center justify-center gap-2" style={{ pointerEvents: "box-none" }}>
      <If when={show()}>
        <View testID="capsule" class={`h-11 flex-row items-center rounded-full ${GLASS}`} style={{ ...BLUR, boxShadow: SHADOW.glass() }}>
          <View class="h-full flex-row items-center gap-[6px] px-[14px]">
            <If when={running()} else={<Text class="text-[14px] text-fg-2">{tr("Stopped")}</Text>}>
              <Spinner size={15} />
              <Text class="text-[14px] font-bold text-sky" style={{ fontFamily: "system-ui", fontVariant: ["tabular-nums"] }}>
                {props.view.startedAt() ? timer(now() - props.view.startedAt()!) : tr("Working")}
              </Text>
            </If>
          </View>
          <If when={props.queue > 0}>
            <View class="h-full flex-row items-center gap-[6px] border-l border-border px-[14px]">
              <Icon name="list-ordered" color="fg" size={15} />
              <Text class="text-[14px] text-fg">{tr("Queue %d", props.queue)}</Text>
            </View>
          </If>
          <If when={subs() > 0}>
            <View class="h-full flex-row items-center gap-[6px] border-l border-border px-[14px]">
              <Icon name="users" color="fg" size={15} />
              <Text class="text-[14px] text-fg">{String(subs())}</Text>
            </View>
          </If>
        </View>
      </If>
      <If when={props.showBottom}>
        <GlassButton icon="arrow-down" label={tr("To the bottom")} onPress={props.onBottom} size={44} testID="to-bottom" />
      </If>
    </View>
  );
}

function HeaderButtons(props: { t: ThreadSummary | null; onGit: () => void; onDiff: () => void; menu: JSX.Element }) {
  return (
    <View class={`flex-row items-center rounded-full ${GLASS}`} style={{ ...BLUR, boxShadow: SHADOW.small() }}>
      <View role="button" testID="open-git" accessibilityLabel={tr("Git")} onPress={props.onGit} class="h-9 w-[38px] items-center justify-center active:opacity-50">
        <Icon name="git-branch" size={18} />
      </View>
      <View role="button" testID="open-diff" accessibilityLabel={tr("Changes")} onPress={props.onDiff} class="h-9 min-w-[38px] items-center justify-center px-2 active:opacity-50">
        <If when={props.t?.diff && (props.t.diff.add || props.t.diff.del)} else={<Icon name="file-diff" size={18} />}>
          <AddDel add={props.t?.diff?.add ?? 0} del={props.t?.diff?.del ?? 0} size={12} />
        </If>
      </View>
      {props.menu}
    </View>
  );
}

export function ThreadPage(props: { machine: string; thread: string }) {
  const view = new ThreadView(props.machine, props.thread);
  view.open();
  cleanup(() => view.close());
  cleanup(tick());

  const t = (): ThreadSummary | null => summary(props.machine, props.thread) ?? view.local();
  const m = () => machine(props.machine);
  const draft = signal("");
  const approvalAt = signal(0);
  const sheet = signal<"model" | "mode" | "git" | "commit" | null>(null);
  const diffOpen = signal(false);
  const commitWay = signal<Way>("commit_push");
  const commitStatus = signal<GitStatus | null>(null);
  const atBottom = signal(true);
  let scroller: ScrollRef | undefined;
  let contentHeight = 0;

  const reconnecting = () => connState(props.machine) !== "online";
  const subtitle = () => (reconnecting() ? tr("Reconnecting…") : `${project(t())} · ${m()?.name ?? ""}`);
  const placeholder = () => (view.approvals().length ? tr("Decline with a line telling the agent what to do") : view.running() ? tr("Steer it…") : tr("Ask it to do something"));

  const toBottom = (animated = true) => scroller?.scrollTo({ y: Math.max(0, contentHeight), animated });
  // new content follows while the user is at the bottom
  effect(() => {
    view.items();
    view.approvals();
    if (atBottom()) setTimeout(() => toBottom(false), 0);
  });

  const call = (method: string, params: Record<string, unknown>) => view.conn?.call(method, { threadId: props.thread, ...params }) ?? Promise.reject(new Error("offline"));

  const answer = async (a: Approval, decision: Decision, feedback?: string) => {
    try {
      await call("runtime-request.respond", { requestId: a.request, decision, ...(feedback ? { feedback } : {}) });
      view.approvals.set(view.approvals().filter((x) => x.request !== a.request));
      approvalAt.set(0);
      Haptics.notification(decision === "decline" ? "warning" : "success");
    } catch (e) {
      toast(tr("Couldn't answer: %s", String((e as Error).message ?? e)), { tone: "error" });
    }
  };

  const send = async (mode: SendMode | "start") => {
    const text = draft().trim();
    if (!text) return;
    const pending = view.approvals()[approvalAt()];
    draft.set("");
    if (pending) {
      await answer(pending, "decline", text);
      return;
    }
    view.echo(text, mode === "queue");
    atBottom.set(true);
    try {
      await call("message.dispatch", { text, dispatchMode: mode });
    } catch (e) {
      const code = (e as { code?: string }).code;
      toast(code === "conflict" ? tr("This session may be running elsewhere: it was written to a minute ago") : tr("Couldn't send: %s", String((e as Error).message ?? e)), { tone: "error" });
      view.items.set(view.items().filter((i) => !(i.kind === "user" && i.id.startsWith("local") && i.text === text)));
      draft.set(text);
    }
  };

  const update = async (patch: Record<string, unknown>) => {
    try {
      const r = await call("thread.update", patch);
      const next = (r as { thread?: ThreadSummary })?.thread;
      if (next) view.put(next);
    } catch (e) {
      toast(tr("Couldn't change it: %s", String((e as Error).message ?? e)), { tone: "error" });
    }
  };

  const moreMenu = () => (
    <Menu
      placement="bottom"
      align="end"
      trigger={
        <View role="button" testID="thread-more" accessibilityLabel={tr("More")} class="h-9 w-[38px] items-center justify-center active:opacity-50">
          <Icon name="ellipsis" size={18} />
        </View>
      }
      popupClass="min-w-[240px] rounded-[14px] border border-border bg-glass p-1"
      popupStyle={BLUR}
    >
      <Each
        of={[
          { id: "rename", label: tr("Rename"), icon: "pencil" as IconName, run: () => t() && renaming.set({ machine: props.machine, thread: t()! }) },
          { id: "settings", label: tr("Thread settings"), icon: "sliders-horizontal" as IconName, run: () => sheet.set("mode") },
          { id: "copy", label: tr("Copy link"), icon: "link" as IconName, run: () => t() && void copyLink(props.machine, t()!) },
          { id: "archive", label: tr("Archive"), icon: "archive" as IconName, run: () => t() && void archive(props.machine, t()!).then(() => chats.pop()) },
          { id: "computer", label: tr("Open on the computer"), icon: "laptop" as IconName, run: () => toast(tr("Opening on the computer comes in phase 2")) },
        ]}
        key={(x) => x.id}
      >
        {(x) => (
          <MenuItem onSelect={x().run} testID={`more-${x().id}`} class="min-h-11 flex-row items-center gap-3 rounded-[10px] px-4 focus:bg-line active:bg-line">
            <Text class="flex-1 text-[16px] text-fg">{x().label}</Text>
            <Icon name={x().icon} size={18} />
          </MenuItem>
        )}
      </Each>
    </Menu>
  );

  const header = (
    <View class="h-[52px] flex-row items-center gap-2 bg-screen px-4">
      <GlassButton icon="chevron-left" label={tr("Back")} onPress={() => chats.pop()} testID="thread-back" />
      <View class="min-w-0 flex-1 items-center">
        <Text testID="thread-title" class="text-[18px] font-bold leading-[23px] text-fg line-clamp-1">
          {t()?.title || tr("Untitled")}
        </Text>
        <Text testID="thread-subtitle" class="text-[12px] leading-[15px] text-fg-2 line-clamp-1">
          {subtitle()}
        </Text>
      </View>
      <HeaderButtons t={t()} onGit={() => sheet.set("git")} onDiff={() => diffOpen.set(true)} menu={moreMenu()} />
    </View>
  );

  return (
    <NavPage
      title={t()?.title ?? ""}
      header={header}
      headerRight={<HeaderButtons t={t()} onGit={() => sheet.set("git")} onDiff={() => diffOpen.set(true)} menu={moreMenu()} />}
      hidesBottomBarWhenPushed
    >
      <View class="flex-1 bg-screen">
        <Scroll
          class="flex-1"
          ref={(r: ScrollRef) => (scroller = r)}
          scrollEventThrottle={64}
          onScroll={(e) => {
            contentHeight = e.contentSize.height;
            atBottom.set(e.contentSize.height - (e.offset.y + e.size.height) < 80);
          }}
          keyboardDismissMode="interactive"
        >
          <View
            testID="feed"
            class="gap-[14px] px-4 pb-[300px] pt-2"
            onLayout={({ bounds }) => {
              contentHeight = bounds.size.height;
              if (atBottom()) toBottom(false);
            }}
          >
            <If when={!view.loading()} else={<Loading text={tr("Loading the thread…")} />}>
              <If when={view.error() && !view.items().length}>
                <Note tone="danger" text={reconnecting() ? tr("Offline: it shows again once %s is back", m()?.name ?? "") : (view.error() ?? "")} />
              </If>
              <If when={t()?.source === "terminal" && !view.items().some((i) => i.kind === "user" && i.id.startsWith("local"))}>
                <Note tone="warning" icon="terminal" text={tr("Opened in a terminal: what you send first takes the session over")} />
              </If>
              <Each of={blocksOf(view.items())} key={(b) => (b.kind === "work" ? b.id : b.item.id)}>
                {(b) => <FeedBlock block={b()} />}
              </Each>
            </If>
          </View>
        </Scroll>
        <View
          class="absolute bottom-0 left-0 right-0 gap-[10px] px-3 pb-[calc(env(safe-area-inset-bottom)+12px)] pt-6"
          style={{ backgroundImage: `linear-gradient(to bottom, transparent, ${token("screen")} 34%)`, pointerEvents: "box-none" }}
        >
          <Capsule view={view} queue={t()?.queue ?? 0} showBottom={!atBottom()} onBottom={() => { atBottom.set(true); toBottom(); }} />
          <If when={view.approvals().length > 0}>
            <ApprovalCard approvals={view.approvals()} index={approvalAt()} onIndex={approvalAt.set} onAnswer={(a, d) => void answer(a, d)} />
          </If>
          <Composer
            text={draft()}
            onText={draft.set}
            placeholder={placeholder()}
            agent={t()?.agent ?? ""}
            model={t()?.model ?? ""}
            running={view.running()}
            onSend={(mode) => void send(mode)}
            onStop={() => void call("run.interrupt", {}).catch(() => {})}
            onModel={() => sheet.set("model")}
            accessory={<Accessory mode={t()?.mode ?? "full"} onMode={() => sheet.set("mode")} />}
          />
        </View>
      </View>
      <ModelSheet
        open={sheet() === "model"}
        onClose={() => sheet.set(null)}
        machine={props.machine}
        agent={t()?.agent ?? ""}
        model={t()?.model ?? ""}
        effort={t()?.effort}
        thread
        onPick={(model, effort) => void update({ model, ...(effort ? { effort } : {}) })}
      />
      <ModeSheet
        open={sheet() === "mode"}
        onClose={() => sheet.set(null)}
        agent={t()?.agent ?? ""}
        mode={t()?.mode ?? "full"}
        onMode={(mode: Mode) => void update({ mode })}
        workspace={t()?.workdir}
      />
      <GitSheet
        open={sheet() === "git"}
        onClose={() => sheet.set(null)}
        machine={props.machine}
        thread={props.thread}
        model={t()?.model ?? ""}
        onDiff={() => {
          sheet.set(null);
          diffOpen.set(true);
        }}
        onCommit={(way, status) => {
          commitWay.set(way);
          commitStatus.set(status);
          sheet.set("commit");
        }}
      />
      <CommitSheet open={sheet() === "commit"} onClose={() => sheet.set(null)} machine={props.machine} thread={props.thread} model={t()?.model ?? ""} way={commitWay()} status={commitStatus()} />
      <DiffView
        open={diffOpen()}
        onClose={() => diffOpen.set(false)}
        machine={props.machine}
        thread={props.thread}
        onComment={(text) => draft.set(text + draft())}
      />
    </NavPage>
  );
}
