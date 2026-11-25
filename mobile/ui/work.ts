// How a thread's work reads, as T3's work log (client-runtime's
// `work-log/presentation.ts`, the app's `threadActivity.ts`): the turns of
// the feed, each settled one folded behind "Worked for 1m 20s" but for its
// answer; within a turn, adjacent tool calls under one line that says what
// they did ("Ran 2 commands and changed 3 files"), the running one's line
// in the present ("Running cargo") under the latest thought; and T3's
// setting for which calls show. Plain functions, so the tests read them.
import type { ToolCalls } from "../state/prefs";
import type { Change, Item } from "../state/thread";
import { tr, trn } from "./i18n";

export type WorkItem = Extract<Item, { kind: "tool" | "reasoning" }>;
export type Action = "command" | "edit" | "read" | "code-search" | "search" | "other";

/** What a call did, by the agent's name for it (Codex's item types and function names, Claude Code's tools). */
export function actionOf(tool: string): Action {
  const t = tool.toLowerCase();
  if (/^(command|commandexecution|bash|shell|local_shell|exec|exec_command|run_shell_command|terminal)$/.test(t)) return "command";
  if (/^(filechange|edit|write|multiedit|notebookedit|apply_patch|write_file|replace|str_replace_editor|create_file)$/.test(t)) return "edit";
  if (/^(read|read_file|readfile|view|view_image|notebookread)$/.test(t)) return "read";
  if (/^(grep|glob|ls|list_dir|list_files|search|search_files|find|codebase_search)$/.test(t)) return "code-search";
  if (/^(websearch|web_search|webfetch|web_fetch|fetch)$/.test(t)) return "search";
  return "other";
}

/** Commands and edits say the most; reads and searches next; the rest last (T3's `summaryActionPriority`). */
const PRIORITY: Record<Action, number> = { command: 0, edit: 0, read: 1, "code-search": 1, search: 1, other: 2 };

function actionLabel(action: Action, n: number): string {
  switch (action) {
    case "command":
      return trn(n, "Ran 1 command", "Ran %d commands", n);
    case "edit":
      return trn(n, "Changed 1 file", "Changed %d files", n);
    case "read":
      return trn(n, "Read 1 file", "Read %d files", n);
    case "code-search":
      return trn(n, "Searched code once", "Searched code %d times", n);
    case "search":
      return trn(n, "Searched the web once", "Searched the web %d times", n);
    default:
      return trn(n, "Used 1 tool", "Used %d tools", n);
  }
}

/** Two or three clauses as one sentence (a group says at most three). */
function join(labels: string[]): string {
  const said = labels.map((l, i) => (i === 0 ? l : l.charAt(0).toLowerCase() + l.slice(1)));
  if (said.length < 2) return said[0] ?? "";
  if (said.length === 2) return tr("%s and %s", said[0], said[1]);
  return tr("%s, %s, and %s", said[0], said[1], said[2]);
}

/** The action a run's icon shows: the one that says most, or thinking alone. */
export function leadAction(items: WorkItem[]): Action | "thought" {
  const tools = items.filter((i): i is Extract<WorkItem, { kind: "tool" }> => i.kind === "tool");
  if (!tools.length) return "thought";
  return tools.map((t) => actionOf(t.tool)).sort((a, b) => PRIORITY[a] - PRIORITY[b])[0];
}

/** A group's line: the two actions that say most, then how many others (T3's `summarizeToolGroup`). */
export function summary(items: WorkItem[]): string {
  const tools = items.filter((i): i is Extract<WorkItem, { kind: "tool" }> => i.kind === "tool");
  if (!tools.length) return items.length === 1 ? tr("Thought") : tr("Thought (×%d)", items.length);
  const groups = new Map<Action, string[]>();
  for (const t of tools) {
    const a = actionOf(t.tool);
    groups.set(a, [...(groups.get(a) ?? []), t.title]);
  }
  const all = [...groups].map(([action, titles], index) => ({
    action,
    index,
    calls: titles.length,
    // an edit counts its files: Codex names several in one call
    count: action === "edit" ? new Set(titles.flatMap((t) => t.split(", "))).size : titles.length,
  }));
  const top = [...all].sort((a, b) => PRIORITY[a.action] - PRIORITY[b.action] || a.index - b.index).slice(0, 2).sort((a, b) => a.index - b.index);
  const labels = top.map((g) => actionLabel(g.action, g.count));
  const rest = tools.length - top.reduce((n, g) => n + g.calls, 0);
  if (rest > 0) labels.push(trn(rest, "Performed 1 other action", "Performed %d other actions", rest));
  return join(labels);
}

/** The program a command runs: past `bash -lc '…'`, variables and `sudo`, without its folder. */
export function program(command: string): string | null {
  let s = command.trim();
  const wrapped = /^(?:\S*\/)?(?:bash|sh|zsh)\s+-l?c\s+(['"])([\s\S]*)\1$/.exec(s);
  if (wrapped) s = wrapped[2].trim();
  for (const word of s.split(/\s+/)) {
    if (/^\w+=/.test(word) || ["sudo", "env", "time", "nohup", "exec"].includes(word)) continue;
    const name = word.replace(/^.*\//, "").replace(/^['"]|['"]$/g, "");
    return name || null;
  }
  return null;
}

/** A running call's line, in the present: "Running cargo"; else what it works on. */
export function liveLabel(item: WorkItem): string {
  if (item.kind === "reasoning") return thoughtLine(item.text) || tr("Thinking");
  if (actionOf(item.tool) === "command") {
    const p = program(item.title);
    return p ? tr("Running %s", p) : tr("Running command");
  }
  return item.title;
}

/** A thought in a line (T3's `liveThoughtLine`): its bold heading, else its first sentence, without Markdown. */
export function thoughtLine(markdown: string): string {
  const heading = /^\s*\*\*([^*\r\n]+)\*\*[ \t]*\r?(?:\n|$)/.exec(markdown)?.[1];
  const text = (heading ?? markdown)
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^[ \t]*(?:#{1,6}|[-*+]|\d+\.)[ \t]+/gm, "")
    .replace(/`+|\*\*|~~/g, "")
    .replace(/(^|[^\w*])[*_]([^*_\n]+)[*_](?![\w*])/g, "$1$2")
    .replace(/\s+/g, " ")
    .trim();
  if (heading !== undefined) return text;
  // CJK sentences end in full-width marks with no space after them
  const end = /[.?!]["'”’)]?(?=\s)|[。！？]["'”’）」』]?/.exec(text);
  return end ? text.slice(0, end.index + end[0].length) : text;
}

/** The latest thought of a run going on, to show over its line; none when nothing was thought in words. */
export function latestThought(items: WorkItem[]): string {
  const t = [...items].reverse().find((i): i is Extract<WorkItem, { kind: "reasoning" }> => i.kind === "reasoning" && !!i.text.trim());
  return t ? thoughtLine(t.text) : "";
}

/** T3's durations: "820ms", "4.2s", "42s", "1m 20s", "1h 2m 3s". */
export function duration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return tr("%dms", 0);
  if (ms < 1000) return tr("%dms", Math.max(1, Math.round(ms)));
  if (ms < 10_000) return tr("%ss", String(Math.round(ms / 100) / 10));
  const total = Math.round(ms / 1000);
  if (total < 60) return tr("%ss", String(total));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h === 0) return tr("%dm %ds", m, s);
  if (m > 0) return tr("%dh %dm %ds", h, m, s);
  return s > 0 ? tr("%dh %ds", h, s) : tr("%dh", h);
}

/** What the feed shows: an item, a run of work, a turn's fold, the live "Working for" line, the "Thinking" row at the end. */
export type Row =
  | { kind: "item"; item: Item }
  | { kind: "work"; id: string; items: WorkItem[]; live: boolean }
  | { kind: "fold"; id: string; label: string; open: boolean; foldable: boolean }
  | { kind: "working"; id: string }
  | { kind: "thinking"; id: string }
  | { kind: "changes"; id: string; change: Change };

/**
 * Each change by its turn's prompt: the one it came under live, else the
 * last prompt before it finished (the history's times and the change's are
 * both the computer's). A later change of the same prompt wins.
 */
export function changesByPrompt(items: Item[], changes: readonly Change[]): Map<string, Change> {
  const prompts = items.filter((i): i is Extract<Item, { kind: "user" }> => i.kind === "user");
  const out = new Map<string, Change>();
  for (const c of [...changes].sort((a, b) => a.turn - b.turn)) {
    let id = c.prompt && prompts.some((p) => p.id === c.prompt) ? c.prompt : undefined;
    if (!id) {
      const done = Date.parse(c.at);
      id = [...prompts].reverse().find((p) => p.at && Date.parse(p.at) <= done)?.id;
    }
    if (id) out.set(id, c);
  }
  return out;
}

const isWork = (i: Item): i is WorkItem => i.kind === "tool" || i.kind === "reasoning";

/** How long a settled turn took: as it was seen to end or its agent wrote it, else from its prompt to where it ended or its last entry. */
function took(prompt: Item | undefined, turn: Item[]): number | undefined {
  const end = turn.find((i) => i.kind === "end") as Extract<Item, { kind: "end" }> | undefined;
  if (end?.ms !== undefined) return end.ms;
  const from = prompt && "at" in prompt && prompt.at ? Date.parse(prompt.at) : NaN;
  const last = [...turn].reverse().find((i) => "at" in i && i.at) as { at: string } | undefined;
  const to = last ? Date.parse(last.at) : NaN;
  return Number.isFinite(from) && Number.isFinite(to) && to > from ? to - from : undefined;
}

/** Whether `visibility` leaves a call out: T3 hides commands, edits and searches, the failed kept by "Hide successful". */
function left(item: Item, visibility: ToolCalls): boolean {
  if (item.kind !== "tool" || visibility === "showAll") return false;
  return visibility === "hideSuccessful" ? item.status === "done" : true;
}

/**
 * `items` as rows: work gathered into runs, each settled turn folded unless
 * `opened` has its fold; while an approval `waiting`, no "Thinking". With
 * `visibility` the calls it hides are left out; "zen" keeps what was said
 * and what went wrong alone, and never folds (T3 hides no message).
 */
export function rowsOf(items: Item[], running: boolean, opened: ReadonlySet<string>, waiting = false, visibility: ToolCalls = "showAll", changes: ReadonlyMap<string, Change> = new Map()): Row[] {
  // the turns: each from a prompt to the next
  const turns: { prompt?: Item; items: Item[] }[] = [];
  for (const item of items) {
    if (item.kind === "user") turns.push({ prompt: item, items: [] });
    else if (turns.length) turns[turns.length - 1].items.push(item);
    else turns.push({ items: [item] });
  }
  const out: Row[] = [];
  const shown = (list: Item[], live: boolean) => {
    for (const item of list) {
      if (item.kind === "end" || left(item, visibility)) continue;
      if (isWork(item)) {
        const last = out[out.length - 1];
        if (last?.kind === "work") last.items.push(item);
        else out.push({ kind: "work", id: `w${item.id}`, items: [item], live: false });
      } else out.push({ kind: "item", item });
    }
    // only the turn's last run of work is the one going on
    const last = out[out.length - 1];
    if (live && last?.kind === "work") last.live = true;
  };
  // under a settled turn's answer: what it changed (T3's changed-files card)
  const changed = (prompt: Item | undefined) => {
    const change = prompt && changes.get(prompt.id);
    if (change) out.push({ kind: "changes", id: `c${prompt.id}`, change });
  };
  turns.forEach((turn, n) => {
    if (turn.prompt) out.push({ kind: "item", item: turn.prompt });
    const key = `f${turn.prompt?.id ?? "start"}`;
    const live = running && n === turns.length - 1;
    const last = turn.items.filter((i) => i.kind !== "end").pop();
    if (visibility === "zen") {
      // what it said and what went wrong; none of its work, nothing folded
      shown(
        turn.items.filter((i) => i.kind === "assistant" || i.kind === "notice" || i.kind === "compact"),
        false,
      );
      const streaming = last?.kind === "assistant" && !!last.streaming;
      if (live && !waiting && !streaming) out.push({ kind: "thinking", id: `t${key}` });
      else if (!live) changed(turn.prompt);
      return;
    }
    if (live) {
      out.push({ kind: "working", id: `r${key}` });
      shown(turn.items, true);
      const busy = waiting || turn.items.some((i) => (i.kind === "tool" || i.kind === "subagent") && i.status === "running") || (last?.kind === "assistant" && !!last.streaming);
      if (!busy) out.push({ kind: "thinking", id: `t${key}` });
      return;
    }
    // settled: its last answer and what went wrong stay; the rest folds
    const answer = [...turn.items].reverse().find((i) => i.kind === "assistant");
    const stays = (i: Item) => i === answer || i.kind === "notice" || i.kind === "compact";
    const hidden = turn.items.filter((i) => !stays(i) && i.kind !== "end" && !left(i, visibility));
    const end = turn.items.find((i) => i.kind === "end") as Extract<Item, { kind: "end" }> | undefined;
    if (hidden.length || end?.stopped) {
      const ms = took(turn.prompt, turn.items);
      const label = end?.stopped
        ? ms !== undefined
          ? tr("You stopped after %s", duration(ms))
          : tr("You stopped this response")
        : ms !== undefined
          ? tr("Worked for %s", duration(ms))
          : tr("Worked");
      const open = opened.has(key);
      out.push({ kind: "fold", id: key, label, open, foldable: hidden.length > 0 });
      shown(open ? turn.items : turn.items.filter(stays), false);
    } else shown(turn.items, false);
    changed(turn.prompt);
  });
  return out;
}
