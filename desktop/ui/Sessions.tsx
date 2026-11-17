// Work › Sessions, as magpie shows it: every session of Claude Code and
// Codex on this computer, by the folder it ran in, to find, pick up again
// or delete. A row says when, on which model, how much was said and spent;
// opened, its details (where, its id, the command that resumes it, its
// file) and its subagents; what was said in it opens on its own, with the
// top, the bottom and each prompt to jump between, or is saved as Markdown
// in Downloads. The
// filter finds a session by its title, folder or id, and a moment after
// the last keystroke by what was said in it. Delete asks first and moves
// the session's files into pansy's trash, which puts them back on Restore;
// erasing for good is only done when asked. A session written to in the
// last minute may still be running, and is left as it is.
import { Clipboard, Scroll, signal, Text, untracked, View, type JSX, type ScrollRef, type ViewRef } from "nsh";
import { Collapsible, TextView, type TextViewClasses } from "nsh/base";
import { Sessions } from "pansy/native";
import { locale, tr, trn } from "./i18n";
import { act, agents, ask, compact, isBusy, loadSessions, reason, sessionOpen, sessions, toast, type Session, type SessionTokens, type Subagent, type Trashed, type Turn } from "./model";
import { agentLogo, Badge, CopyButton, Empty, Icon, IconButton, Input, List, Logo, Page, Row, RowHead, SHADOW, TextButton } from "./parts";

/** Rows drawn at first in a folder, and conversation turns on a page: each "Show more" draws as many again. */
const PAGE = 50;
const TURNS = 120;

/** The agent whose sessions are shown; "" for the one with the most. */
const agentShown = signal("");
const query = signal("");
/** What was said in sessions, searched for `query`: by `agent/id`, the words around it. */
const found = signal<{ query: string; hits: Map<string, string>; busy: boolean } | null>(null);
/** The session opened to its details, by `agent/id`. */
const opened = signal("");
/** Folders folded or unfolded by hand, by their path; the first few start unfolded. */
const unfolded = signal<Record<string, boolean>>({});
/** Rows drawn in each folder, by its path. */
const drawn = signal<Record<string, number>>({});
/** Sessions whose subagents are unfolded, by `agent/id`. */
const subsOpen = signal<Record<string, boolean>>({});
const trashOn = signal(false);
const trashed = signal<Trashed[] | null>(null);

const keyOf = (s: { agent: string; id: string }) => `${s.agent}/${s.id}`;
const NAMES: Record<string, string> = { "claude-code": "Claude Code", codex: "Codex", opencode: "OpenCode" };
const agentName = (id: string) => agents().find((a) => a.id === id)?.name ?? NAMES[id] ?? id;
const totalOf = (t: SessionTokens) => t.input + t.output + t.cacheRead + t.cacheWrite;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number) => String(n).padStart(2, "0");

/** "Nov 8", "11月8日"; with its year when it isn't this one. */
function dateLabel(d: Date): string {
  const year = d.getFullYear() === new Date().getFullYear() ? "" : String(d.getFullYear());
  if (locale() === "zh-Hans") return `${year ? `${year}年` : ""}${d.getMonth() + 1}月${d.getDate()}日`;
  return `${MONTHS[d.getMonth()]} ${d.getDate()}${year ? `, ${year}` : ""}`;
}

/** When, from now: "just now", "5 min ago", "3 h ago", "Yesterday", "4 days ago", else the date. */
function ago(at: string | null): string {
  if (!at) return "";
  const d = new Date(at);
  const s = (Date.now() - d.getTime()) / 1000;
  if (s < 60) return tr("just now");
  if (s < 3600) return tr("%d min ago", Math.floor(s / 60));
  if (s < 86400) return tr("%d h ago", Math.floor(s / 3600));
  if (s < 2 * 86400) return tr("Yesterday");
  if (s < 7 * 86400) return tr("%d days ago", Math.floor(s / 86400));
  return dateLabel(d);
}

/** "Nov 8 14:02". */
function stamp(at: string | null): string {
  if (!at) return "";
  const d = new Date(at);
  return `${dateLabel(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 1536 → "1.5 KB". */
function bytes(n: number): string {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)} GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  if (n >= 1 << 10) return `${Math.round(n / (1 << 10))} KB`;
  return `${n} B`;
}

/** Where a folder is, as its row says it: the folder it is in, the home folder as ~. */
function whereOf(path: string): string {
  const home = sessions()?.home ?? "";
  const up = path.replace(/[\\/]+$/, "");
  const cut = Math.max(up.lastIndexOf("/"), up.lastIndexOf("\\"));
  const parent = cut < 0 ? "" : cut === 0 ? "/" : up.slice(0, cut);
  if (!home || !parent) return parent;
  if (parent === home) return "~";
  return parent.startsWith(`${home}/`) || parent.startsWith(`${home}\\`) ? `~${parent.slice(home.length)}` : parent;
}

const baseName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;

/** The agents with sessions, the one with the most first. */
function agentCounts(): { agent: string; count: number }[] {
  const n = new Map<string, number>();
  for (const s of sessions()?.sessions ?? []) n.set(s.agent, (n.get(s.agent) ?? 0) + 1);
  return [...n].map(([agent, count]) => ({ agent, count })).sort((a, b) => b.count - a.count);
}

const currentAgent = () => {
  const all = agentCounts();
  return all.find((a) => a.agent === agentShown())?.agent ?? all[0]?.agent ?? "";
};

const needle = () => query().trim().toLowerCase();

/** What was said in it that the filter found, if it was found that way. */
const hitOf = (s: Session) => {
  const f = found();
  return f && f.query === needle() ? f.hits.get(keyOf(s)) : undefined;
};

function matches(s: Session): boolean {
  const q = needle();
  if (!q) return true;
  return s.title.toLowerCase().includes(q) || s.cwd.toLowerCase().includes(q) || s.id.toLowerCase().startsWith(q) || hitOf(s) !== undefined;
}

/** The sessions shown, by folder: the folder of the latest first. */
function folders(): { path: string; sessions: Session[] }[] {
  const agent = currentAgent();
  const out = new Map<string, Session[]>();
  for (const s of sessions()?.sessions ?? []) {
    if (s.agent !== agent || !matches(s)) continue;
    const list = out.get(s.cwd);
    if (list) list.push(s);
    else out.set(s.cwd, [s]);
  }
  return [...out].map(([path, list]) => ({ path, sessions: list }));
}

let searchTimer: ReturnType<typeof setTimeout> | undefined;

/** Looks through what was said a moment after the last keystroke; a newer one wins. */
function setQuery(text: string) {
  query.set(text);
  drawn.set({});
  clearTimeout(searchTimer);
  const q = text.trim().toLowerCase();
  if (q.length < 2) {
    found.set(null);
    return;
  }
  found.set({ query: q, hits: found()?.query === q ? found()!.hits : new Map(), busy: true });
  searchTimer = setTimeout(() => {
    void Sessions.search(q)
      .then((got) => {
        if (needle() !== q) return;
        const hits = new Map<string, string>();
        for (const h of JSON.parse(got) as { agent: string; id: string; text: string }[]) hits.set(keyOf(h), h.text);
        found.set({ query: q, hits, busy: false });
      })
      .catch((error: unknown) => {
        if (needle() === q) found.set({ query: q, hits: new Map(), busy: false });
        toast(reason(error), "error");
      });
  }, 300);
}

async function loadTrash() {
  try {
    trashed.set(JSON.parse(await Sessions.trash()) as Trashed[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

function copy(text: string) {
  return Clipboard.writeText(text).then(() => toast(tr("Copied")));
}

async function remove(s: Session) {
  const yes = await ask({
    title: tr("Delete “%s”?", s.title || tr("(no prompt)")),
    description: tr("Its files move into pansy's trash, and %s can't resume it until you restore it there.", agentName(s.agent)),
    confirm: tr("Move to trash"),
    danger: true,
  });
  if (!yes) return;
  if (await act(`session:${keyOf(s)}`, () => Sessions.remove(s.agent, s.id), tr("Moved to the trash"))) await loadSessions();
}

// --- The list -------------------------------------------------------------------

/** The agents with sessions, as magpie's tabs: each one's logo, name and count. */
function AgentTabs() {
  return (
    <View testID="sessions-agents" class="flex-row self-start rounded-[7px] bg-pill p-[2px]">
      <Each of={agentCounts()} key={(a) => a.agent}>
        {(a) => {
          const on = () => currentAgent() === a().agent;
          return (
            <View
              role="button"
              testID={`sessions-agent-${a().agent}`}
              onPress={() => {
                agentShown.set(a().agent);
                opened.set("");
                drawn.set({});
              }}
              class={`h-[22px] flex-row items-center gap-[6px] rounded-[5px] px-[9px] ${on() ? "bg-seg-thumb" : "hover:bg-pill-hover"}`}
              style={on() ? { boxShadow: SHADOW.thumb() } : undefined}
            >
              <Logo name={agentLogo(a().agent)} size={13} />
              <Text class={`text-[12px] ${on() ? "text-foreground" : "text-muted-foreground"}`}>{agentName(a().agent)}</Text>
              <Text class="text-[11px] text-faint">{String(a().count)}</Text>
            </View>
          );
        }}
      </Each>
    </View>
  );
}

/** One line of a session's details: what, and its value, with a copy button for some. */
function Line(props: { k: string; v?: string; copy?: string; mono?: boolean; children?: JSX.Element; testID?: string }) {
  return (
    <View class="min-h-[22px] flex-row items-center gap-3">
      <Text class="w-[92px] text-[11.5px] text-muted-foreground">{props.k}</Text>
      <If when={props.v !== undefined}>
        <Text testID={props.testID} class={`min-w-0 flex-1 text-[12px] text-foreground ${props.mono ? "font-mono text-[11.5px]" : ""}`}>
          {props.v ?? ""}
        </Text>
      </If>
      {props.children}
      <If when={props.copy}>
        <CopyButton label={tr("Copy %s", props.k)} onCopy={() => copy(props.copy ?? "")} />
      </If>
    </View>
  );
}

/** The tokens spent, in a line: "12K in · 3.4K out · 1.2M cache read". */
function spent(t: SessionTokens): string {
  const parts = [tr("%s in", compact(t.input)), tr("%s out", compact(t.output))];
  if (t.cacheRead) parts.push(tr("%s cache read", compact(t.cacheRead)));
  if (t.cacheWrite) parts.push(tr("%s cache write", compact(t.cacheWrite)));
  if (t.reasoning) parts.push(tr("%s reasoning", compact(t.reasoning)));
  return parts.join(" · ");
}

/** A path as rows show it: the home folder as ~. */
function tilde(path: string): string {
  const home = sessions()?.home ?? "";
  return home && (path.startsWith(`${home}/`) || path.startsWith(`${home}\\`)) ? `~${path.slice(home.length)}` : path;
}

/** Saves the whole conversation as Markdown in Downloads, and says where. */
function exportTalk(s: Session) {
  return act(`export:${keyOf(s)}`, async () => {
    const path = await Sessions.export(s.agent, s.id);
    toast(tr("Saved to %s", tilde(path)));
  });
}

function openTalk(s: Session, sub?: Subagent) {
  sessionOpen.set({ agent: s.agent, id: s.id, sub: sub?.id ?? "", title: sub ? sub.title : s.title || tr("(no prompt)") });
}

function Details(props: { s: Session }) {
  const s = () => props.s;
  const subs = () => subsOpen()[keyOf(s())] === true;
  return (
    <View testID={`session-details-${s().id}`} class="gap-[3px] border-t border-line-2 bg-card-2 py-[9px] pl-[14px] pr-[12px]">
      <Line k={tr("Time")} v={`${stamp(s().start)} – ${stamp(s().last)}`} />
      <If when={s().cwd}>
        <Line k={tr("Folder")} v={s().cwd} copy={s().cwd} />
      </If>
      <Line k={tr("Session ID")} v={s().id} copy={s().id} mono />
      <If when={s().resume}>
        <Line k={tr("Resume")} v={s().resume} copy={s().resume} mono testID={`session-resume-${s().id}`} />
      </If>
      <If when={s().models.length > 0}>
        <Line k={tr("Models")} v={s().models.join(", ")} />
      </If>
      <Line k={tr("Tokens")} v={spent(s().tokens)} />
      <Line k={tr("File")} v={s().path} copy={s().path} mono />
      <Line k={tr("Conversation")}>
        <TextButton label={tr("Show conversation")} icon="chevron-right" onPress={() => openTalk(s())} testID={`session-talk-${s().id}`} />
        <TextButton
          label={tr("Export Markdown")}
          icon="save"
          onPress={() => void exportTalk(s())}
          disabled={isBusy(`export:${keyOf(s())}`)}
          testID={`session-export-${s().id}`}
        />
      </Line>
      <If when={s().subagents.length > 0}>
        <View
          role="button"
          testID={`session-subs-${s().id}`}
          onPress={() => subsOpen.update((o) => ({ ...o, [keyOf(s())]: !subs() }))}
          class="mt-1 h-[22px] flex-row items-center gap-[5px] self-start rounded-[6px] pl-1 pr-[6px] hover:bg-pill"
        >
          <Icon name={subs() ? "chev-down" : "chev-right"} color="muted-foreground" size={10} />
          <Text class="text-[12px] text-fg-2">{trn(s().subagents.length, "%d subagent", "%d subagents", s().subagents.length)}</Text>
        </View>
        <If when={subs()}>
          <List class="ml-4">
            <Each of={s().subagents} key={(a) => a.id}>
              {(a, i) => (
                <Row first={i() === 0} class="min-h-[38px] py-[5px]" onPress={() => openTalk(s(), a())} testID={`subagent-${a().id}`}>
                  <View class="min-w-0 flex-1 gap-[1px]">
                    <Text class="text-[12.5px] text-foreground line-clamp-1">{a().title || a().id}</Text>
                    <Text class="text-[11px] text-muted-foreground line-clamp-1">
                      {[a().kind, ago(a().last), trn(a().messages, "%d message", "%d messages", a().messages), compact(totalOf(a().tokens))].filter(Boolean).join(" · ")}
                    </Text>
                  </View>
                  <Icon name="chev-right" color="faint" size={10} />
                </Row>
              )}
            </Each>
          </List>
        </If>
      </If>
    </View>
  );
}

function SessionRow(props: { s: Session; first: boolean }) {
  const s = () => props.s;
  const open = () => opened() === keyOf(s());
  const sub = () =>
    [ago(s().last), s().models[0] ?? "", trn(s().messages, "%d message", "%d messages", s().messages), bytes(s().size), s().title === s().id ? "" : s().id.slice(0, 8)]
      .filter(Boolean)
      .join(" · ");
  return (
    <View testID={`session-${s().id}`} class={props.first ? "" : "border-t border-line-2"}>
      <View class="min-h-[52px] flex-row items-center gap-3 py-[7px] pl-[14px] pr-[12px]">
        <View role="button" onPress={() => opened.set(open() ? "" : keyOf(s()))} testID={`session-open-${s().id}`} class="min-w-0 flex-1 flex-row items-center gap-[9px]">
          <Icon name={open() ? "chev-down" : "chev-right"} color="faint" size={10} />
          <View class="min-w-0 flex-1 gap-[1px]">
            <View class="flex-row items-center gap-[7px]">
              <Text testID={`session-title-${s().id}`} class="shrink text-[13px] font-medium text-foreground line-clamp-1">
                {s().title || tr("(no prompt)")}
              </Text>
              <If when={s().active}>
                <Badge label={tr("Running")} tone="success" />
              </If>
            </View>
            <Text class="text-[11.5px] text-muted-foreground line-clamp-1">{sub()}</Text>
            <If when={hitOf(s())}>
              <Text class="text-[11.5px] text-fg-2 line-clamp-2">{hitOf(s()) ?? ""}</Text>
            </If>
          </View>
        </View>
        <View class="w-[78px] items-end">
          <Text class="text-[13px] font-semibold text-foreground">{compact(totalOf(s().tokens))}</Text>
          <Text class="text-[10.5px] text-muted-foreground">{tr("tokens")}</Text>
        </View>
        <If when={s().resume}>
          <TextButton label={tr("Resume")} onPress={() => void copy(s().resume)} testID={`session-copy-resume-${s().id}`} />
        </If>
        <IconButton
          icon="trash-2"
          label={
            !s().deletable
              ? tr("%s keeps its sessions in its own database: pansy lists them, and doesn't delete them", agentName(s().agent))
              : s().active
                ? tr("Written to in the last minute: it may still be running")
                : tr("Delete")
          }
          disabled={!s().deletable || s().active || isBusy(`session:${keyOf(s())}`)}
          onPress={() => void remove(s())}
          testID={`session-delete-${s().id}`}
        />
      </View>
      <If when={open()}>
        <Details s={s()} />
      </If>
    </View>
  );
}

function Folder(props: { path: string; list: Session[]; index: number }) {
  const key = () => props.path;
  const open = () => needle() !== "" || (unfolded()[key()] ?? props.index < 3);
  const n = () => drawn()[key()] ?? PAGE;
  return (
    <View testID={`sessions-folder-${baseName(props.path) || "none"}`} class="gap-[6px]">
      <View
        role="button"
        onPress={() => unfolded.update((u) => ({ ...u, [key()]: !open() }))}
        class="h-[26px] flex-row items-center gap-[7px] self-stretch rounded-[6px] px-[4px] hover:bg-pill"
      >
        <Icon name={open() ? "chev-down" : "chev-right"} color="muted-foreground" size={10} />
        <Icon name="folder" color="fg-2" size={13} />
        <Text class="text-[12.5px] font-medium text-foreground line-clamp-1">{props.path ? baseName(props.path) : tr("No folder")}</Text>
        <Text class="shrink text-[11.5px] text-faint line-clamp-1">{props.path ? whereOf(props.path) : ""}</Text>
        <View class="flex-1" />
        <Text class="text-[11.5px] text-muted-foreground">{String(props.list.length)}</Text>
      </View>
      <If when={open()}>
        <List>
          <Each of={props.list.slice(0, n())} key={(s) => keyOf(s)}>
            {(s, i) => <SessionRow s={s()} first={i() === 0} />}
          </Each>
        </List>
        <If when={props.list.length > n()}>
          <TextButton label={tr("Show %d more", Math.min(PAGE, props.list.length - n()))} onPress={() => drawn.update((d) => ({ ...d, [key()]: n() + PAGE }))} />
        </If>
      </If>
    </View>
  );
}

function SessionList() {
  const all = () => folders();
  return (
    <>
      <Text testID="sessions-note" class="px-1 text-[11px] text-faint">
        {currentAgent() === "opencode"
          ? tr("Read from OpenCode's database on this computer, which OpenCode keeps open: its sessions are listed and resumed here, not deleted.")
          : tr("Read from the agents' own files on this computer. Delete moves a session's files into pansy's trash.")}
      </Text>
      <If when={found()?.busy && found()?.query === needle()}>
        <Text testID="sessions-searching" class="px-1 text-[11.5px] text-muted-foreground">
          {tr("Looking through what was said…")}
        </Text>
      </If>
      <If
        when={all().length > 0}
        else={<Empty testID="sessions-none" title={tr("No session matches")} description={tr("The filter looks in titles, folders, ids and what was said.")} />}
      >
        <Each of={all()} key={(f) => f.path}>
          {(f, i) => <Folder path={f().path} list={f().sessions} index={i()} />}
        </Each>
      </If>
    </>
  );
}

// --- The trash ------------------------------------------------------------------

function Trash() {
  const list = () => trashed() ?? [];
  const restore = async (t: Trashed) => {
    if (await act(`trash:${t.key}`, () => Sessions.restore(t.key), tr("Put back"))) await Promise.all([loadTrash(), loadSessions()]);
  };
  const purge = async (t: Trashed) => {
    const yes = await ask({
      title: tr("Delete “%s” forever?", t.title || tr("(no prompt)")),
      description: tr("Its files are erased from this computer, and it can't be restored."),
      confirm: tr("Delete forever"),
      danger: true,
    });
    if (yes && (await act(`trash:${t.key}`, () => Sessions.purge(t.key)))) await Promise.all([loadTrash(), loadSessions()]);
  };
  const emptyAll = async () => {
    const yes = await ask({
      title: tr("Empty the trash?"),
      description: trn(list().length, "The %d session in it is erased from this computer, and can't be restored.", "The %d sessions in it are erased from this computer, and can't be restored.", list().length),
      confirm: tr("Empty the trash"),
      danger: true,
    });
    if (yes && (await act("trash:empty", () => Sessions.emptyTrash()))) await Promise.all([loadTrash(), loadSessions()]);
  };
  return (
    <>
      <RowHead label={tr("Trash")} note={trn(list().length, "%d session", "%d sessions", list().length)}>
        <If when={list().length > 0}>
          <TextButton label={tr("Empty the trash")} tone="danger" onPress={() => void emptyAll()} testID="trash-empty" />
        </If>
      </RowHead>
      <If
        when={list().length > 0}
        else={<Empty testID="trash-none" title={tr("The trash is empty")} description={tr("Deleted sessions wait here, their files untouched, until you restore them or delete them forever.")} />}
      >
        <List testID="trash-list">
          <Each of={list()} key={(t) => t.key}>
            {(t, i) => (
              <Row first={i() === 0} class="min-h-[50px]" testID={`trashed-${t().id}`}>
                <Logo name={agentLogo(t().agent)} size={16} />
                <View class="min-w-0 flex-1 gap-[1px]">
                  <Text class="text-[13px] font-medium text-foreground line-clamp-1">{t().title || tr("(no prompt)")}</Text>
                  <Text class="text-[11.5px] text-muted-foreground line-clamp-1">
                    {[tr("Deleted %s", ago(t().deleted)), t().cwd ? `${whereOf(t().cwd) === "~" ? "~/" : `${whereOf(t().cwd)}/`}${baseName(t().cwd)}` : "", bytes(t().size)].filter(Boolean).join(" · ")}
                  </Text>
                </View>
                <TextButton label={tr("Restore")} onPress={() => void restore(t())} disabled={isBusy(`trash:${t().key}`)} testID={`trash-restore-${t().id}`} />
                <TextButton label={tr("Delete forever")} tone="danger" onPress={() => void purge(t())} disabled={isBusy(`trash:${t().key}`)} testID={`trash-purge-${t().id}`} />
              </Row>
            )}
          </Each>
        </List>
      </If>
    </>
  );
}

// --- A conversation -------------------------------------------------------------
// Drawn as AI Elements draws one (elements.ai-sdk.dev): a prompt in a bubble
// at the right, a reply as Markdown across, what the model thought folded
// under Reasoning, and each tool call a card that opens to what it was given
// and what it gave back.

const timeOf = (at: string | null) => {
  if (!at) return "";
  const d = new Date(at);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

/** Markdown at the window's size: TextView reads at 15pt, the window at 13. */
const PROSE: TextViewClasses = {
  root: "text-[13px] leading-[20px] text-foreground",
  heading1: "text-[17px] leading-[24px]",
  heading2: "text-[15px] leading-[22px]",
  heading3: "text-[13.5px] leading-[20px]",
  code: "text-[11.5px]",
  codeBlockCode: "text-[11.5px] leading-[17px]",
};

/** What the model thought, quieter than what it said. */
const THOUGHT: TextViewClasses = { ...PROSE, root: "text-[12px] leading-[18px] text-muted-foreground" };

/** A tool's input or output: only its code block. */
const CODE: TextViewClasses = {
  root: "text-[11.5px] leading-[17px] text-foreground",
  codeBlock: "rounded-[6px] border-line-2 bg-card-2",
  codeBlockCode: "text-[11.5px] leading-[17px]",
};

const COPY = () => [{ id: "copy", label: tr("Copy") }];

/** `code` as a Markdown code block in `language`, its fence longer than any run of backticks in it. */
function fenced(code: string, language: string): string {
  const longest = Math.max(2, ...(code.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(longest + 1);
  return `${fence}${language}\n${code}\n${fence}`;
}

/** The language a tool's input or output is written in, as far as its start tells. */
const languageOf = (text: string) => (/^\s*[[{]/.test(text) ? "json" : text.startsWith("*** Begin Patch") ? "diff" : "");

/** How a tool call ended, as AI Elements' badge says it. */
const STATES = () =>
  ({
    done: { label: tr("Completed"), icon: "circle-check", color: "success" },
    error: { label: tr("Error"), icon: "circle-x", color: "destructive" },
    none: { label: tr("No result"), icon: "circle", color: "muted-foreground" },
  }) as const;

function ToolPart(props: { label: string; children: JSX.Element }) {
  return (
    <View class="gap-[6px]">
      <Text class="text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground">{props.label}</Text>
      {props.children}
    </View>
  );
}

/** A tool call: its name, what it was given in a line and how it ended; opened, what it was given and gave back. */
function ToolCard(props: { turn: Turn; index: number }) {
  const t = () => props.turn;
  const open = signal(false);
  const state = () => STATES()[t().status ?? "none"];
  const opens = () => Boolean(t().input || t().output);
  return (
    <Collapsible
      open={open()}
      reveal
      class="self-stretch rounded-[8px] border border-border bg-card"
      content={
        <View class="gap-[12px] border-t border-line-2 px-[12px] pb-[12px] pt-[10px]">
          <If when={t().input}>
            <ToolPart label={tr("Parameters")}>
              <TextView markdown={fenced(t().input ?? "", languageOf(t().input ?? ""))} classes={CODE} codeBlockActions={COPY()} />
            </ToolPart>
          </If>
          <If when={t().output}>
            <ToolPart label={t().status === "error" ? tr("Error") : tr("Result")}>
              <If
                when={t().status === "error"}
                else={<TextView markdown={fenced(t().output ?? "", languageOf(t().output ?? ""))} classes={CODE} codeBlockActions={COPY()} />}
              >
                <View class="rounded-[6px] bg-destructive-soft px-[10px] py-[8px]">
                  <Text class="font-mono text-[11.5px] leading-[17px] text-destructive">{t().output ?? ""}</Text>
                </View>
              </If>
            </ToolPart>
          </If>
        </View>
      }
    >
      <View
        role="button"
        testID={`session-tool-${props.index}`}
        onPress={() => opens() && open.set(!open())}
        accessibilityState={{ expanded: open() }}
        class={`min-h-[38px] flex-row items-center gap-[8px] rounded-[8px] px-[12px] py-[8px] ${opens() ? "hover:bg-card-2" : ""}`}
      >
        <Icon name="wrench" color="muted-foreground" size={13} />
        <Text class="shrink-0 text-[12.5px] font-medium text-foreground">{t().name || tr("Tool")}</Text>
        <Text class="min-w-0 flex-1 font-mono text-[11.5px] text-muted-foreground line-clamp-1">{t().text}</Text>
        <View class="h-[20px] flex-row items-center gap-[5px] rounded-full bg-pill px-[8px]">
          <Icon name={state().icon} color={state().color} size={11} />
          <Text class="text-[11px] text-fg-2">{state().label}</Text>
        </View>
        <If when={opens()}>
          <Icon name={open() ? "chev-up" : "chev-down"} color="muted-foreground" size={10} />
        </If>
      </View>
    </Collapsible>
  );
}

/** What the model thought: folded under Reasoning. */
function Reasoning(props: { turn: Turn }) {
  const open = signal(false);
  return (
    <Collapsible
      open={open()}
      reveal
      class="self-stretch"
      content={
        <View class="ml-[7px] mt-[6px] border-l-2 border-line-2 pl-[12px]">
          <TextView markdown={props.turn.text} classes={THOUGHT} />
        </View>
      }
    >
      <View
        role="button"
        onPress={() => open.set(!open())}
        accessibilityState={{ expanded: open() }}
        class="h-[22px] flex-row items-center gap-[6px] self-start rounded-[6px] pl-1 pr-[6px] hover:bg-pill"
      >
        <Icon name="brain" color="muted-foreground" size={12} />
        <Text class="text-[12px] text-muted-foreground">{tr("Reasoning")}</Text>
        <Icon name={open() ? "chev-up" : "chev-down"} color="muted-foreground" size={10} />
      </View>
    </Collapsible>
  );
}

/** A prompt in a bubble at the right, or a reply as Markdown across. */
function Message(props: { turn: Turn; agent: string; place?: Place }) {
  const t = () => props.turn;
  const mine = () => t().role === "user";
  return (
    <View
      ref={props.place?.ref}
      onLayout={props.place ? (e) => props.place?.top(e.bounds.origin.y) : undefined}
      class={`gap-[4px] ${mine() ? "self-end" : "self-stretch"}`}
      style={mine() ? { maxWidth: "80%" } : undefined}
    >
      <View class={`flex-row items-center gap-[6px] ${mine() ? "self-end" : ""}`}>
        <If when={!mine()}>
          <Logo name={agentLogo(props.agent)} size={12} />
        </If>
        <Text class="text-[11px] font-medium text-muted-foreground">{mine() ? tr("You") : agentName(props.agent)}</Text>
        <Text class="text-[11px] text-faint">{timeOf(t().at)}</Text>
      </View>
      <If when={mine()} else={<TextView markdown={t().text} classes={PROSE} codeBlockActions={COPY()} />}>
        <View class="rounded-[12px] bg-pill px-[14px] py-[9px]">
          <Text class="text-[13px] leading-[20px] text-foreground">{t().text}</Text>
        </View>
      </If>
    </View>
  );
}

/** Where a prompt is: its view, to scroll to, and where it starts in the conversation, as it is laid out. */
type Place = { ref: (r: ViewRef) => void; top: (y: number) => void };

function TurnView(props: { turn: Turn; agent: string; index: number; place?: Place }) {
  const t = () => props.turn;
  return (
    <If
      when={t().role !== "compact"}
      else={
        <View class="flex-row items-center gap-[10px] py-1">
          <View class="h-px flex-1 bg-line-2" />
          <Text class="text-[11px] text-faint">{tr("The conversation was compacted here")}</Text>
          <View class="h-px flex-1 bg-line-2" />
        </View>
      }
    >
      <If when={t().role !== "tool"} else={<ToolCard turn={t()} index={props.index} />}>
        <If when={t().role !== "reasoning"} else={<Reasoning turn={t()} />}>
          <Message turn={t()} agent={props.agent} place={t().role === "user" ? props.place : undefined} />
        </If>
      </If>
    </If>
  );
}

/** How near the top of the view a prompt counts as the one being read. */
const SLACK = 8;

/**
 * An open conversation, as magpie's sheet of one: its head stays above it,
 * with where it is among the prompts and the top, the previous and the next
 * prompt and the bottom to jump to, at once as a key pressed many times would.
 */
function Conversation() {
  const at = untracked(() => sessionOpen());
  const turns = signal<Turn[] | null>(null);
  const failed = signal("");
  const shown = signal(TURNS);
  const session = () => sessions()?.sessions.find((s) => s.agent === at?.agent && s.id === at?.id);
  if (at) {
    void Sessions.transcript(at.agent, at.id, at.sub)
      .then((got) => turns.set(JSON.parse(got) as Turn[]))
      .catch((error: unknown) => failed.set(reason(error)));
  }
  const items = () => turns() ?? [];

  let scroller: ScrollRef | undefined;
  let end: ViewRef | undefined;
  /** Each prompt's view, by its turn's index. */
  const anchors = new Map<number, ViewRef>();
  /** Where each prompt drawn starts among the turns, by its turn's index. */
  const tops = signal<Record<number, number>>({});
  /** Where the turns start in what scrolls. */
  const talkY = signal(0);
  /** How far it is scrolled, how tall the view is and what it shows. */
  const offset = signal(0);
  const viewH = signal(0);
  const contentH = signal(0);

  const prompts = () => items().flatMap((t, i) => (t.role === "user" ? [i] : []));
  /** Where prompt `i` starts in what scrolls; none until it is drawn. */
  const topOf = (i: number) => (tops()[i] === undefined ? undefined : talkY() + tops()[i]);
  const allShown = () => shown() >= items().length;
  const atEnd = () => allShown() && offset() + viewH() >= contentH() - 1;
  const before = () => prompts().filter((i) => (topOf(i) ?? Infinity) < offset() - SLACK);
  const after = () => prompts().filter((i) => (topOf(i) ?? Infinity) > offset() + SLACK);
  /** The prompt being read: the last one whose top has passed the top of the view (the last of all at the bottom). */
  const here = () => {
    const all = prompts();
    if (all.length === 0) return 0;
    if (atEnd()) return all.length;
    return Math.max(1, all.filter((i) => (topOf(i) ?? Infinity) <= offset() + SLACK).length);
  };

  /** Draws the turns up to `i` if they aren't yet, then scrolls to prompt `i`. */
  const goTo = (i: number) => {
    if (i >= shown()) shown.set(Math.min(items().length, i + TURNS));
    queueMicrotask(() => anchors.get(i)?.reveal({ vertical: "start", animated: false }));
  };
  const top = () => scroller?.scrollTo({ y: 0, animated: false });
  const bottom = () => {
    shown.set(items().length);
    queueMicrotask(() => end?.reveal({ vertical: "end", animated: false }));
  };
  const place = (i: number): Place => ({
    ref: (r) => anchors.set(i, r),
    top: (y) => tops.update((t) => (t[i] === y ? t : { ...t, [i]: y })),
  });

  return (
    <View testID="page-sessions" class="flex-1">
      <View class="min-h-[34px] flex-row items-center gap-[8px] pb-[4px] pl-5 pr-4">
        <Logo name={agentLogo(at?.agent ?? "")} size={14} />
        <Text class="min-w-0 shrink text-[12px] text-muted-foreground line-clamp-1">
          {[agentName(at?.agent ?? ""), at?.sub ? tr("Subagent") : "", session()?.cwd ?? "", stamp(session()?.start ?? null)].filter(Boolean).join(" · ")}
        </Text>
        <View class="flex-1" />
        <If when={!at?.sub && session()?.resume}>
          <TextButton label={tr("Copy the resume command")} icon="copy-s" onPress={() => void copy(session()?.resume ?? "")} />
        </If>
        <If when={prompts().length > 0}>
          <Text testID="session-nav-count" class="ml-1 text-[11.5px] text-muted-foreground">{`${here()} / ${prompts().length}`}</Text>
        </If>
        <View class="flex-row items-center">
          <IconButton icon="top" label={tr("Top")} onPress={top} disabled={offset() <= 0} testID="session-nav-top" />
          <IconButton icon="up" label={tr("Previous prompt")} onPress={() => goTo(before().at(-1) ?? 0)} disabled={before().length === 0} testID="session-nav-prev" />
          <IconButton icon="down" label={tr("Next prompt")} onPress={() => goTo(after()[0] ?? 0)} disabled={after().length === 0} testID="session-nav-next" />
          <IconButton icon="bottom" label={tr("Bottom")} onPress={bottom} disabled={items().length === 0 || atEnd()} testID="session-nav-bottom" />
        </View>
      </View>
      <Scroll
        ref={(r) => (scroller = r)}
        class="flex-1"
        onScroll={(e) => {
          offset.set(e.offset.y);
          viewH.set(e.size.height);
          contentH.set(e.contentSize.height);
        }}
        onLayout={(e) => viewH.set(e.bounds.size.height)}
      >
        <View class="gap-[10px] pb-6 pl-4 pr-4 pt-[2px]" onLayout={(e) => contentH.set(e.bounds.size.height)}>
          <If when={failed()}>
            <Empty title={tr("Could not read the conversation")} description={failed()} />
          </If>
          <If when={turns() === null && !failed()}>
            <Text class="px-1 text-[12px] text-muted-foreground">{tr("Reading the conversation…")}</Text>
          </If>
          <If when={turns() !== null && items().length === 0}>
            <Empty title={tr("Nothing was said in it")} />
          </If>
          <View selectable testID="session-talk" class="gap-[14px]" onLayout={(e) => talkY.set(e.bounds.origin.y)}>
            <Each of={items().slice(0, shown())} key="index">
              {(turn, i) => <TurnView turn={turn()} agent={at?.agent ?? ""} index={i()} place={place(i())} />}
            </Each>
          </View>
          <If when={items().length > shown()}>
            <TextButton label={tr("Show %d more", Math.min(TURNS, items().length - shown()))} onPress={() => shown.set(shown() + TURNS)} testID="session-talk-more" />
          </If>
          <View ref={(r) => (end = r)} class="h-px" />
        </View>
      </Scroll>
    </View>
  );
}

/** What the title bar says of an open conversation. */
export function sessionCrumb(): string {
  return sessionOpen()?.title ?? "";
}

export function SessionsPage() {
  const loaded = () => sessions() !== null;
  const none = () => loaded() && (sessions()?.sessions.length ?? 0) === 0;
  const trashCount = () => sessions()?.trash ?? 0;
  return (
    <If when={sessionOpen() === null} else={<Conversation />}>
      <Page testID="page-sessions">
        <View class="h-[34px] flex-row items-center gap-[10px]">
          <If when={!trashOn() && agentCounts().length > 0}>
            <AgentTabs />
          </If>
          <If when={trashOn()}>
            <TextButton label={tr("Sessions")} icon="chevron-left" onPress={() => trashOn.set(false)} testID="trash-back" />
          </If>
          <View class="flex-1" />
          <If when={!trashOn() && !none()}>
            <Input value={query()} onChange={setQuery} placeholder={tr("Search sessions")} testID="sessions-search" class="w-[220px]" />
          </If>
          <TextButton
            label={trashCount() > 0 ? tr("Trash (%d)", trashCount()) : tr("Trash")}
            icon="trash-2"
            onPress={() => {
              trashOn.set(!trashOn());
              if (trashOn()) void loadTrash();
            }}
            testID="sessions-trash"
          />
        </View>
        <If when={trashOn()}>
          <Trash />
        </If>
        <If when={!trashOn()}>
          <If when={loaded()} else={<Text class="px-1 text-[12px] text-muted-foreground">{tr("Reading the agents' sessions…")}</Text>}>
            <If
              when={!none()}
              else={<Empty testID="sessions-empty" title={tr("No sessions yet")} description={tr("Claude Code's, Codex's and OpenCode's sessions on this computer show up here, by the folder they ran in.")} />}
            >
              <SessionList />
            </If>
          </If>
        </If>
      </Page>
    </If>
  );
}
