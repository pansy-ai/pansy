// 线程 (phone-ui.md "2. 线程"): the user's lines in bubbles, the agent's
// answers flat in Markdown, its work folded into rows, subagents, the
// approval card docked over the composer, the status capsule; the header's
// Git, changes and more. History and live events come from state/thread.ts.
import { cleanup, Clipboard, createAnimatedValue, effect, Scroll, signal, Text, token, View, type JSX, type ScrollRef } from "nsh";
import { Menu, MenuItem, NavPage } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Decision, GitStatus, Mode, Question, ThreadSummary } from "../net/types";
import { connState, machine, summary } from "../state/store";
import { toolCalls } from "../state/prefs";
import { ThreadView, type Approval, type Change, type Item } from "../state/thread";
import { archive, copyLink, renaming } from "./actions";
import { Accessory, Composer, type SendMode } from "./Composer";
import { DiffView } from "./Diff";
import { CommitSheet, GitSheet, type Way } from "./Git";
import { tr, trn } from "./i18n";
import { Markdown } from "./Markdown";
import { ModelSheet } from "./ModelSheet";
import { ModeSheet } from "./ModeSheet";
import { chats } from "./nav";
import { AddDel, BLUR, Btn, Field, GLASS, GlassButton, Icon, Loading, Note, SHADOW, Spinner, toast, type Color, type IconName } from "./parts";
import { clock, now, tick, timer } from "./time";
import { actionOf, changesByPrompt, duration, latestThought, leadAction, liveLabel, rowsOf, thoughtLine, summary as workSummary, type Action, type Row, type WorkItem } from "./work";

const project = (t: ThreadSummary | undefined | null) => (t ? (t.folder.split(/[\\/]/).filter(Boolean).pop() ?? t.folder) : "");

/** A tool's icon and color by what it did. */
function toolLook(tool: string, title: string, status: string): { icon: IconName; color: Color } {
  const k = `${tool} ${title}`.toLowerCase();
  if (status === "failed") return { icon: "circle-x", color: "danger" };
  if (/\brm\b|delete|remove|删/.test(k)) return { icon: "trash-2", color: "rose" };
  if (/quota|额度/.test(k)) return { icon: "circle-alert", color: "warning" };
  return { icon: ACTION_ICONS[actionOf(tool)], color: "fg-2" };
}

const ACTION_ICONS: Record<Action, IconName> = { command: "terminal", edit: "pencil", read: "eye", "code-search": "search", search: "globe", other: "wrench" };

/** `#rrggbb` or `#rrggbbaa` at `alpha`. */
const rgba = (hex: string, alpha: number) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
};

/** T3's shimmer over a line of work going on: a band sweeping across in 1350 ms, then a pause of 1450. */
function Shimmer() {
  const x = createAnimatedValue(-72);
  void x.keyframes(
    [
      { offset: 0, value: -72 },
      { offset: 0.48, value: 420 },
      { offset: 1, value: 420 },
    ],
    { duration: 2800, iterations: "infinite", easing: "linear" },
  );
  cleanup(() => void x.stop());
  const glow = (alpha: number) => rgba(token("screen"), alpha);
  return <View class="absolute bottom-0 top-0 w-[72px]" style={{ transform: [{ translateX: x }], backgroundImage: `linear-gradient(to right, ${glow(0)}, ${glow(0.85)}, ${glow(0)})`, pointerEvents: "none" }} />;
}

/** A row of the work log: 28pt, its icon, a line in 14pt, a chevron; open, what went in and came out. */
function WorkRow(props: { icon: IconName; color: Color; text: string; live?: boolean; detail?: string; exit?: number; testID?: string; tall?: number; onPress?: () => void; open?: boolean; children?: JSX.Element }) {
  const own = signal(false);
  const open = () => props.open ?? own();
  const opens = () => !!props.detail || !!props.onPress;
  return (
    <View>
      <View
        role="button"
        testID={props.testID}
        onPress={() => {
          if (!opens()) return;
          if (props.onPress) props.onPress();
          else own.set(!own());
          Haptics.selection();
        }}
        class="min-h-7 flex-row items-center gap-1"
      >
        <View class="h-6 w-6 items-center justify-center">
          <If when={props.live} else={<Icon name={props.icon} color={props.color} size={14} />}>
            <Spinner size={13} />
          </If>
        </View>
        <View class="min-w-0 flex-1 overflow-hidden">
          <Text class={`text-[14px] leading-[19px] line-clamp-1 text-${props.color}`}>{props.text}</Text>
          <If when={props.live}>
            <Shimmer />
          </If>
        </View>
        <If when={opens()}>
          <View class="transition-transform duration-180" style={{ transform: [{ rotate: open() ? "90deg" : "0deg" }] }}>
            <Icon name="chevron-right" color="fg-3" size={11} />
          </View>
        </If>
      </View>
      <If when={open() && props.detail}>
        <Scroll class="ml-7 rounded-lg bg-grouped" style={{ maxHeight: props.tall ?? 240 }}>
          <Text class="px-[10px] pt-2 font-mono text-[12px] leading-[17px] text-fg-2" style={{ userSelect: "text" }}>
            {props.detail ?? ""}
          </Text>
          <Text class={`px-[10px] pb-2 font-mono text-[12px] leading-[17px] ${props.exit ? "text-danger" : "text-fg-3"}`}>{props.exit !== undefined ? `exit ${props.exit}` : ""}</Text>
        </Scroll>
      </If>
      {props.children}
    </View>
  );
}

function ToolItem(props: { item: Extract<Item, { kind: "tool" }> }) {
  const look = () => toolLook(props.item.tool, props.item.title, props.item.status);
  const running = () => props.item.status === "running";
  const detail = () => [props.item.input, props.item.output].filter(Boolean).join("\n\n") || undefined;
  return (
    <WorkRow
      testID={`tool-${props.item.id}`}
      icon={look().icon}
      color={props.item.status === "declined" ? "fg-3" : look().color}
      text={props.item.status === "declined" ? tr("%s (declined)", props.item.title) : running() ? liveLabel(props.item) : props.item.title}
      live={running()}
      detail={detail()}
      exit={props.item.exit}
    />
  );
}

/** A subagent: its dot, what it was told, `agent · model`; opens on what it reported back, once it has. */
function SubagentItem(props: { item: Extract<Item, { kind: "subagent" }> }) {
  const open = signal(false);
  const dot = () => (props.item.status === "running" ? "bg-sky" : props.item.status === "failed" ? "bg-danger" : "bg-emerald");
  const report = () => props.item.output?.trim() ?? "";
  return (
    <View>
      <View
        role="button"
        testID={`subagent-${props.item.id}`}
        onPress={() => {
          if (!report()) return;
          open.set(!open());
          Haptics.selection();
        }}
        class="flex-row items-center gap-2 py-[6px]"
      >
        <View class={`h-[6px] w-[6px] rounded-full ${dot()}`} />
        <Text class="shrink text-[14px] font-medium text-fg line-clamp-1">{props.item.name}</Text>
        <Text class="flex-1 text-[13px] text-fg-2 line-clamp-1">{props.item.detail}</Text>
        <If when={report()}>
          <View class="transition-transform duration-180" style={{ transform: [{ rotate: open() ? "90deg" : "0deg" }] }}>
            <Icon name="chevron-right" color="fg-3" size={12} />
          </View>
        </If>
      </View>
      <If when={open() && report()}>
        <Scroll testID={`subagent-report-${props.item.id}`} class="ml-[14px] rounded-lg bg-grouped px-[10px] py-2" style={{ maxHeight: 240 }}>
          <Markdown text={report()} />
        </Scroll>
      </If>
    </View>
  );
}

/** A call or a thought; read from `props` where it shows, as the run it is in changes under it. */
function WorkItemView(props: { item: WorkItem; live?: boolean }) {
  const thought = () => props.item as Extract<WorkItem, { kind: "reasoning" }>;
  return (
    <If
      when={props.item.kind === "tool"}
      else={<WorkRow icon="brain" color="fg-2" text={props.live ? liveLabel(thought()) : thoughtLine(thought().text) || tr("Thought")} live={props.live} detail={thought().text || undefined} tall={384} />}
    >
      <ToolItem item={props.item as Extract<WorkItem, { kind: "tool" }>} />
    </If>
  );
}

/** A run of work: one call as its row; more under one line saying what they did, the running one's in the present. */
function Work(props: { items: WorkItem[]; live: boolean }) {
  const open = signal(false);
  const latest = () => props.items[props.items.length - 1];
  const failed = () => props.items.some((i) => i.kind === "tool" && i.status === "failed");
  const icon = (): IconName => {
    const lead = leadAction(props.items);
    return lead === "thought" ? "brain" : ACTION_ICONS[lead];
  };
  // going on and folded: the latest thought over the line, as T3's
  const thought = () => (props.live && !open() ? latestThought(props.items) : "");
  const line = () => (!props.live || open() ? workSummary(props.items) : thought() && latest().kind === "reasoning" ? tr("Thinking") : liveLabel(latest()));
  return (
    <If when={props.items.length > 1} else={<WorkItemView item={props.items[0]} live={props.live && latest().kind === "reasoning"} />}>
      <If when={thought()}>
        <View testID="work-thought" class="flex-row gap-1 py-[2px]">
          <View class="h-6 w-6 items-center justify-center">
            <Icon name="brain" color="fg-3" size={14} />
          </View>
          <Text class="min-w-0 flex-1 py-[2px] text-[14px] leading-[19px] text-fg-3 line-clamp-4">{thought()}</Text>
        </View>
      </If>
      <WorkRow
        testID="work-fold"
        icon={failed() ? "circle-x" : icon()}
        color={failed() ? "danger" : "fg-2"}
        text={line()}
        live={props.live && !open()}
        open={open()}
        onPress={() => open.set(!open())}
      >
        <If when={open()}>
          <Scroll style={{ maxHeight: 256 }}>
            <View class="gap-px">
              {/* kept by id: a call that changes updates its row, which stays open if it was */}
              <Each of={props.items} key={(i) => i.id}>
                {(i) => <WorkItemView item={i()} live={props.live && i() === latest() && i().kind === "reasoning"} />}
              </Each>
            </View>
          </Scroll>
        </If>
      </WorkRow>
    </If>
  );
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

/** A row of the feed; its kind stays for its key, its content changes (props are read where they show). */
/** Files a changes card lists before "Show all". */
const CARD_FILES = 5;

/** What a settled turn changed, under its answer (T3's changed-files card): the files with their counts; each opens the turn's diff. */
function ChangesCard(props: { change: Change; onOpen: (turn: number) => void }) {
  const all = signal(false);
  const files = () => (all() ? props.change.files : props.change.files.slice(0, CARD_FILES));
  const n = () => props.change.files.length;
  const open = () => {
    Haptics.selection();
    props.onOpen(props.change.turn);
  };
  return (
    <View testID={`changes-${props.change.turn}`} class="overflow-hidden rounded-2xl border border-border">
      <View role="button" accessibilityLabel={tr("Open this turn's changes")} onPress={open} class="min-h-11 flex-row items-center gap-2 bg-grouped px-3 active:opacity-70">
        <Icon name="file-diff" color="fg-2" size={15} />
        <Text class="flex-1 text-[14px] font-medium text-fg">{trn(n(), "Changed 1 file", "Changed %d files", n())}</Text>
        <AddDel add={props.change.add} del={props.change.del} size={13} mono />
        <Icon name="chevron-right" color="fg-3" size={12} />
      </View>
      <Each of={files()} key={(f) => f.path}>
        {(f) => (
          <View role="button" accessibilityLabel={f().path} onPress={open} class="min-h-9 flex-row items-center gap-2 border-t border-line px-3 active:bg-line">
            <Text class={`min-w-0 flex-1 font-mono text-[12px] line-clamp-1 ${f().status === "deleted" ? "text-fg-3 line-through" : "text-fg"}`}>{f().path}</Text>
            <AddDel add={f().add} del={f().del} size={12} mono />
          </View>
        )}
      </Each>
      <If when={!all() && n() > CARD_FILES}>
        <View role="button" testID={`changes-more-${props.change.turn}`} onPress={() => all.set(true)} class="min-h-9 justify-center border-t border-line px-3 active:bg-line">
          <Text class="text-[13px] text-fg-2">{tr("Show all %d files", n())}</Text>
        </View>
      </If>
    </View>
  );
}

function FeedRow(props: { row: Row; startedAt: number | null; onFold: (id: string) => void; onTurnDiff: (turn: number) => void }) {
  const row = <K extends Row["kind"]>() => props.row as Extract<Row, { kind: K }>;
  switch (props.row.kind) {
    case "changes":
      return <ChangesCard change={row<"changes">().change} onOpen={props.onTurnDiff} />;
    case "work":
      return <Work items={row<"work">().items} live={row<"work">().live} />;
    case "fold":
      return (
        <WorkRow
          testID="turn-fold"
          icon="clock"
          color="fg-2"
          text={row<"fold">().label}
          open={row<"fold">().open}
          onPress={row<"fold">().foldable ? () => props.onFold(row<"fold">().id) : undefined}
        />
      );
    case "working":
      return <WorkRow testID="turn-working" icon="loader-circle" color="fg-2" text={props.startedAt ? tr("Working for %s", duration(now() - props.startedAt)) : tr("Working")} live />;
    case "thinking":
      return <WorkRow testID="turn-thinking" icon="brain" color="fg-2" text={tr("Thinking")} live />;
  }
  const item = <K extends Item["kind"]>() => row<"item">().item as Extract<Item, { kind: K }>;
  const it = item();
  switch (it.kind) {
    case "user":
      return <UserBubble item={item<"user">()} />;
    case "assistant":
      return <Answer item={item<"assistant">()} />;
    case "subagent":
      return <SubagentItem item={item<"subagent">()} />;
    case "notice":
      return (
        <View class="flex-row items-center gap-2 px-1">
          <Icon name="circle-alert" color="danger" size={14} />
          <Text class="flex-1 text-[14px] text-danger">
            {it.exited ? (it.exited.code !== undefined && it.exited.code !== null ? tr("The agent quit (exit code %d)", it.exited.code) : tr("The agent quit")) : it.text}
          </Text>
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

/** A question's answer so far: the options picked, and a line of the user's own. */
export type Picked = { picks: string[]; other: string };
/** Each question's answer so far, by the question. */
export type Answers = Record<string, Picked>;

const answered = (p: Picked | undefined) => !!p && (p.picks.length > 0 || !!p.other.trim());

/** What is sent for each question: its picks, then its own line. */
export function answersOf(questions: Question[], answers: Answers): Record<string, string> {
  return Object.fromEntries(questions.map((q) => [q.question, [...(answers[q.question]?.picks ?? []), answers[q.question]?.other.trim() ?? ""].filter(Boolean).join(", ")]));
}

/** A line typed in the composer while questions wait: the first unanswered one takes it as its own (the first one, when all are answered); `send` once every one is. */
export function typedAnswer(questions: Question[], answers: Answers, text: string): { answers: Answers; send: Record<string, string> | null } {
  const q = questions.find((q) => !answered(answers[q.question])) ?? questions[0];
  if (!q) return { answers, send: null };
  const next = { ...answers, [q.question]: { picks: answers[q.question]?.picks ?? [], other: text } };
  return { answers: next, send: questions.every((q) => answered(next[q.question])) ? answersOf(questions, next) : null };
}

/** A question's choices: one, or several when it says so, and a line of the user's own; Send once each has an answer. */
function QuestionCard(props: { approval: Approval; answers: Answers; onAnswers: (a: Answers) => void; onChoose: (a: Approval, answers: Record<string, string>) => void; onAnswer: (a: Approval, d: Decision) => void }) {
  const qs = (): Question[] => props.approval.questions ?? [];
  const at = (q: Question): Picked => props.answers[q.question] ?? { picks: [], other: "" };
  const toggle = (q: Question, label: string) => {
    const now = at(q).picks;
    const next = q.multiSelect ? (now.includes(label) ? now.filter((l) => l !== label) : [...now, label]) : [label];
    props.onAnswers({ ...props.answers, [q.question]: { ...at(q), picks: next } });
    Haptics.selection();
  };
  const ready = () => qs().every((q) => answered(props.answers[q.question]));
  const send = () => props.onChoose(props.approval, answersOf(qs(), props.answers));
  return (
    <View class="gap-[12px]">
      <Each of={qs()} key={(q) => q.question}>
        {(q, n) => (
          <View class="gap-[6px]">
            <If when={q().header}>
              <Text class="text-[12px] font-bold uppercase tracking-[1.1px] text-fg-2">{q().header}</Text>
            </If>
            <Text class="text-[16px] font-bold leading-[21px] text-fg">{q().question}</Text>
            <View class="overflow-hidden rounded-xl bg-grouped">
              <Each of={q().options} key={(o) => o.label}>
                {(o, i) => {
                  const on = () => at(q()).picks.includes(o().label);
                  return (
                    <View
                      role="button"
                      accessibilityLabel={o().label}
                      testID={`question-option-${o().label}`}
                      onPress={() => toggle(q(), o().label)}
                      class={`min-h-11 flex-row items-center gap-[10px] px-3 py-2 active:bg-line ${i() > 0 ? "border-t border-line" : ""}`}
                    >
                      <Icon name={on() ? (q().multiSelect ? "square-check" : "circle-check") : q().multiSelect ? "square" : "circle"} color={on() ? "primary" : "fg-3"} size={18} />
                      <View class="flex-1 gap-[1px]">
                        <Text class="text-[15px] text-fg">{o().label}</Text>
                        <If when={o().description}>
                          <Text class="text-[13px] text-fg-3">{o().description}</Text>
                        </If>
                      </View>
                    </View>
                  );
                }}
              </Each>
            </View>
            {/* each question its own "Other", as T3's */}
            <Field
              value={at(q()).other}
              onChange={(other) => props.onAnswers({ ...props.answers, [q().question]: { ...at(q()), other } })}
              placeholder={tr("Other answer")}
              testID={`question-other-${n()}`}
            />
          </View>
        )}
      </Each>
      <View class="gap-2">
        <Btn label={tr("Send answer")} onPress={send} disabled={!ready()} testID="question-send" />
        <Btn tone="danger" label={tr("Don't answer")} onPress={() => props.onAnswer(props.approval, "decline")} testID="question-decline" />
      </View>
    </View>
  );
}

/** 需要批准: docked over the composer, not in the feed; several go 1 / N. */
function ApprovalCard(props: {
  approvals: Approval[];
  index: number;
  onIndex: (i: number) => void;
  onAnswer: (a: Approval, d: Decision) => void;
  onChoose: (a: Approval, answers: Record<string, string>) => void;
  answers: (a: Approval) => Answers;
  onAnswers: (a: Approval, answers: Answers) => void;
}) {
  const a = () => props.approvals[Math.min(props.index, props.approvals.length - 1)];
  const long = signal(false);
  const lines = () => a()?.detail.split("\n") ?? [];
  return (
    <View testID="approval-card" class="gap-[10px] rounded-[20px] border border-border bg-card p-4" style={{ boxShadow: SHADOW.glass() }}>
      <View class="flex-row items-center justify-between">
        <Text class="text-[12px] font-bold uppercase tracking-[1.1px] text-fg-2">{a()?.questions ? tr("A question for you") : tr("Needs approval")}</Text>
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
      <If when={a()?.questions}>
        <QuestionCard approval={a()!} answers={props.answers(a()!)} onAnswers={(x) => props.onAnswers(a()!, x)} onChoose={props.onChoose} onAnswer={props.onAnswer} />
      </If>
      <If when={!a()?.questions}>
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
      </If>
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
  // the questions' answers so far, by request: the card and the composer both fill them
  const questionAnswers = signal<Record<string, Answers>>({});
  const answersFor = (a: Approval): Answers => questionAnswers()[a.request] ?? {};
  const setAnswers = (a: Approval, answers: Answers) => questionAnswers.set({ ...questionAnswers(), [a.request]: answers });
  const sheet = signal<"model" | "mode" | "git" | "commit" | null>(null);
  const diffOpen = signal(false);
  // the turn a changes card opened the diff on (else the diff's own scopes)
  const diffTurn = signal<number | undefined>(undefined);
  const openTurn = (turn: number) => {
    diffTurn.set(turn);
    diffOpen.set(true);
  };
  const commitWay = signal<Way>("commit_push");
  const commitStatus = signal<GitStatus | null>(null);
  const atBottom = signal(true);
  // the settled turns opened past their "Worked for" line
  const opened = signal<ReadonlySet<string>>(new Set());
  const fold = (id: string) => {
    const next = new Set(opened());
    if (!next.delete(id)) next.add(id);
    opened.set(next);
  };
  let scroller: ScrollRef | undefined;
  let contentHeight = 0;

  const reconnecting = () => connState(props.machine) !== "online";
  const subtitle = () => (reconnecting() ? tr("Reconnecting…") : `${project(t())} · ${m()?.name ?? ""}`);
  const placeholder = () =>
    view.approvals()[approvalAt()]?.questions
      ? tr("Or type your own answer")
      : view.approvals().length
        ? tr("Decline with a line telling the agent what to do")
        : view.running()
          ? tr("Steer it…")
          : tr("Ask it to do something");

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

  const choose = async (a: Approval, answers: Record<string, string>) => {
    try {
      await call("runtime-request.respond", { requestId: a.request, answers });
      view.approvals.set(view.approvals().filter((x) => x.request !== a.request));
      approvalAt.set(0);
      Haptics.notification("success");
    } catch (e) {
      toast(tr("Couldn't answer: %s", String((e as Error).message ?? e)), { tone: "error" });
    }
  };

  const send = async (mode: SendMode | "start") => {
    const text = draft().trim();
    if (!text) return;
    const pending = view.approvals()[approvalAt()];
    draft.set("");
    if (pending?.questions) {
      // typed: the answer to the first question still open; sent once each has one
      const typed = typedAnswer(pending.questions, answersFor(pending), text);
      setAnswers(pending, typed.answers);
      if (typed.send) await choose(pending, typed.send);
      return;
    }
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
          { id: "archive", label: tr("Settle"), icon: "archive" as IconName, run: () => t() && void archive(props.machine, t()!).then(() => chats.pop()) },
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
              <Each of={rowsOf(view.items(), view.running(), opened(), view.approvals().length > 0, toolCalls(), changesByPrompt(view.items(), view.changes()))} key={(r) => (r.kind === "item" ? r.item.id : r.id)}>
                {(r) => <FeedRow row={r()} startedAt={view.startedAt()} onFold={fold} onTurnDiff={openTurn} />}
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
            <ApprovalCard approvals={view.approvals()} index={approvalAt()} onIndex={approvalAt.set} onAnswer={(a, d) => void answer(a, d)} onChoose={(a, c) => void choose(a, c)} answers={answersFor} onAnswers={setAnswers} />
          </If>
          <Composer
            text={draft()}
            onText={draft.set}
            placeholder={placeholder()}
            agent={t()?.agent ?? ""}
            model={t()?.model ?? ""}
            running={view.running()}
            onSend={(mode) => void send(mode)}
            onStop={(force) => void call("run.interrupt", force ? { force: true } : {}).catch(() => {})}
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
        onClose={() => {
          diffOpen.set(false);
          diffTurn.set(undefined);
        }}
        machine={props.machine}
        thread={props.thread}
        worktree={!!t()?.worktree}
        turn={diffTurn()}
        onComment={(text) => draft.set(text + draft())}
      />
    </NavPage>
  );
}
