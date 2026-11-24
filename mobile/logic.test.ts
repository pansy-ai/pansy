// The parts of the app that are plain functions: pairing links and
// addresses, the Markdown blocks, how events fold into a thread's feed, and
// the list's monograms, times and title as T3 has them, usage's counts, and
// the work log's lines and folds.
import { expect, test } from "nsh/test";
import { formatCode, normalizeAddress, parsePairUrl } from "./net/pair";
import { apply, fromHistory, type FeedState } from "./state/thread";
import { blocks, spans } from "./ui/Markdown";
import { monogram, monogramHue } from "./ui/parts";
import { answersOf, typedAnswer } from "./ui/Thread";
import { connectionTitle } from "./ui/Threads";
import { since } from "./ui/time";
import { compact, top } from "./ui/Usage";
import { duration, latestThought, program, rowsOf, summary as workSummary, thoughtLine, type WorkItem } from "./ui/work";
import type { Item } from "./state/thread";

test("reads a pairing link's address and secret from #pair=", () => {
  expect(parsePairUrl("http://192.168.1.20:7268/#pair=Zm9vYmFyLWJhego_-x")).toEqual({ base: "http://192.168.1.20:7268", secret: "Zm9vYmFyLWJhego_-x" });
  expect(parsePairUrl("https://mac.tail1234.ts.net/pair?x=1#v=1&pair=abc")).toEqual({ base: "https://mac.tail1234.ts.net", secret: "abc" });
  expect(parsePairUrl("http://192.168.1.20:7268/")).toBe(null);
  expect(parsePairUrl("http://192.168.1.20:7268/#pairing=abc")).toBe(null);
});

test("an address typed by hand gets its scheme and pansy's port", () => {
  expect(normalizeAddress("192.168.1.20")).toBe("http://192.168.1.20:7268");
  expect(normalizeAddress("192.168.1.20:9000")).toBe("http://192.168.1.20:9000");
  expect(normalizeAddress(" https://m-3f2a.pansy.dev/ ")).toBe("https://m-3f2a.pansy.dev:7268");
});

test("a typed pairing code keeps six digits, shown 123-456", () => {
  expect(formatCode("")).toBe("");
  expect(formatCode("123")).toBe("123");
  expect(formatCode("1234")).toBe("123-4");
  expect(formatCode("123-456")).toBe("123-456");
  expect(formatCode("12a3 45-6789")).toBe("123-456");
});

test("splits Markdown into blocks and spans", () => {
  const b = blocks("# Title\n\nSome `code` and **bold**.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n> quoted");
  expect(b.map((x) => x.kind)).toEqual(["h", "p", "list", "code", "quote"]);
  expect(b[3]).toEqual({ kind: "code", lang: "rust", text: "fn main() {}" });
  expect(spans("a `b` **c** [d](https://e)")).toEqual([{ text: "a " }, { text: "b", code: true }, { text: " " }, { text: "c", bold: true }, { text: " " }, { text: "d", link: "https://e" }]);
  // a fence still streaming runs to the end
  expect(blocks("```ts\nlet a").pop()).toEqual({ kind: "code", lang: "ts", text: "let a" });
});

test("folds a turn's events into the feed", () => {
  let s: FeedState = { items: fromHistory([{ role: "user", text: "hi" }]), approvals: [], running: false, startedAt: null };
  s = apply(s, { type: "turnStarted", turn: "t1" }, 1000);
  expect(s.running).toBe(true);
  s = apply(s, { type: "delta", item: "m1", text: "Hel" }, 1000);
  s = apply(s, { type: "delta", item: "m1", text: "lo" });
  expect(s.items[1]).toEqual({ kind: "assistant", id: "m1", text: "Hello", streaming: true, at: new Date(1000).toISOString() });
  s = apply(s, { type: "toolStarted", item: "c1", kind: "commandExecution", title: "cargo test" });
  s = apply(s, { type: "approval", request: "r1", kind: "commandExecution", title: "cargo test", detail: "cargo test" });
  expect(s.approvals.length).toBe(1);
  s = apply(s, { type: "resolved", request: "r1" });
  expect(s.approvals.length).toBe(0);
  s = apply(s, { type: "toolFinished", item: "c1", kind: "commandExecution", title: "ran cargo test", status: "completed", output: "ok" });
  s = apply(s, { type: "turnFinished", turn: "t1", status: "completed" });
  expect(s.running).toBe(false);
  expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "end"]);
  expect(s.items[2]).toMatchObject({ kind: "tool", id: "c1", tool: "commandExecution", title: "ran cargo test", status: "done", output: "ok" });
  // the turn's end keeps how long it took from its start
  expect(s.items[3]).toMatchObject({ kind: "end", stopped: false });
  expect(typeof (s.items[3] as { ms?: number }).ms).toBe("number");
});

test("a subagent is one whatever the agent calls it, and keeps what it reported", () => {
  let s: FeedState = { items: [], approvals: [], running: true, startedAt: 0 };
  s = apply(s, { type: "toolStarted", item: "a1", kind: "subagent", title: "Find the header · Explore" });
  // an older pansy passed Claude Code's own word through
  s = apply(s, { type: "toolStarted", item: "a2", kind: "Task", title: "Check the docs" });
  expect(s.items.map((i) => i.kind)).toEqual(["subagent", "subagent"]);
  expect(s.items[0]).toMatchObject({ name: "Find the header", detail: "Explore", status: "running" });
  s = apply(s, { type: "toolFinished", item: "a1", kind: "subagent", title: "Find the header · Explore", status: "completed", output: "In proxy.rs" });
  expect(s.items[0]).toMatchObject({ name: "Find the header", detail: "Explore", status: "done", output: "In proxy.rs" });
  // Codex's history names it spawn_agent
  expect(fromHistory([{ role: "tool", name: "spawn_agent", text: "Explore the gateway", output: "done" }])[0]).toMatchObject({ kind: "subagent", name: "Explore the gateway", output: "done" });
});

test("an agent that quits mid-turn ends the turn, its calls failed, saying so", () => {
  let s: FeedState = { items: fromHistory([{ role: "user", text: "go" }]), approvals: [], running: false, startedAt: null };
  s = apply(s, { type: "turnStarted", turn: "t1" }, 1000);
  s = apply(s, { type: "delta", item: "m1", text: "Look" }, 1000);
  s = apply(s, { type: "toolStarted", item: "c1", kind: "commandExecution", title: "cargo test" });
  s = apply(s, { type: "toolStarted", item: "a1", kind: "subagent", title: "Explore" });
  s = apply(s, { type: "approval", request: "r1", kind: "commandExecution", title: "cargo test", detail: "cargo test" });
  s = apply(s, { type: "exited", code: 137 }, 4000);
  expect(s.running).toBe(false);
  expect(s.approvals).toEqual([]);
  expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool", "subagent", "notice", "end"]);
  expect(s.items[1]).toMatchObject({ streaming: false });
  expect(s.items[2]).toMatchObject({ status: "failed" });
  expect(s.items[3]).toMatchObject({ status: "failed" });
  expect(s.items[4]).toMatchObject({ kind: "notice", tone: "danger", exited: { code: 137 } });
  expect(s.items[5]).toMatchObject({ kind: "end", ms: 3000, stopped: false });
  // quitting between turns adds nothing
  expect(apply(s, { type: "exited" }).items.length).toBe(s.items.length);
});

test("a typed line answers the first open question; all answered, it sends", () => {
  const qs = ["Which database?", "Where?"].map((question) => ({ question, header: "", multiSelect: false, options: [] }));
  const one = typedAnswer(qs, { "Where?": { picks: ["ops/"], other: "" } }, "Postgres");
  expect(one.answers["Which database?"]).toEqual({ picks: [], other: "Postgres" });
  expect(one.send).toEqual({ "Which database?": "Postgres", "Where?": "ops/" });
  const first = typedAnswer(qs, {}, "SQLite");
  expect(first.send).toBe(null);
  expect(typedAnswer(qs, first.answers, "scripts/").send).toEqual({ "Which database?": "SQLite", "Where?": "scripts/" });
  // a choice and a line of its own go together
  expect(answersOf(qs, { "Which database?": { picks: ["SQLite"], other: " in WAL mode " } })["Which database?"]).toBe("SQLite, in WAL mode");
});

test("a question waits like an approval, with its choices", () => {
  let s: FeedState = { items: [], approvals: [], running: true, startedAt: null };
  const questions = [{ question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] }];
  s = apply(s, { type: "question", request: "q1", questions });
  expect(s.approvals).toEqual([{ request: "q1", kind: "question", title: "Pick", detail: "", questions }]);
  s = apply(s, { type: "resolved", request: "q1" });
  expect(s.approvals.length).toBe(0);
});

test("a project's monogram and its color are T3's", () => {
  expect(monogram("pansy")).toBe("PY");
  expect(monogram("pansy-site")).toBe("PS");
  expect(monogram("nsh2")).toBe("N2");
  expect(monogram("")).toBe("PR");
  // fuchsia, as T3 draws pansy's
  expect(monogramHue("pansy")).toBe("#d946ef");
});

test("the list's times are T3's compact ones", () => {
  const at = Date.parse("2026-05-02T12:00:00Z");
  expect(since("2026-05-02T11:59:30Z", at)).toBe("<1m");
  expect(since("2026-05-02T11:55:00Z", at)).toBe("5m");
  expect(since("2026-05-02T09:00:00Z", at)).toBe("3h");
  expect(since("2026-04-30T11:00:00Z", at)).toBe("2d");
});

test("the title names the machines it reconnects to only while none is online", () => {
  expect(connectionTitle([])).toBe(null);
  expect(connectionTitle([{ name: "mac", state: "online" }, { name: "pc", state: "offline" }])).toBe(null);
  expect(connectionTitle([{ name: "mac", state: "connecting" }])).toEqual({ label: "Reconnecting to mac", spin: true });
  expect(connectionTitle([{ name: "mac", state: "offline" }, { name: "pc", state: "connecting" }])).toEqual({ label: "Reconnecting 2 machines", spin: true });
  expect(connectionTitle([{ name: "mac", state: "revoked" }])).toEqual({ label: "Not connected", spin: false });
});

test("usage counts are short, and who used most comes first", () => {
  expect(compact(1019)).toBe("1019");
  expect(compact(12_400)).toBe("12.4K");
  expect(compact(3_400_000)).toBe("3.4M");
  expect(compact(250_000_000)).toBe("250M");
  const t = (input: number, requests = 1) => ({ requests, errors: 0, input, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 });
  expect(top({ a: t(5), b: t(9), c: t(0, 0), d: t(7) }, 2).map(([k]) => k)).toEqual(["b", "d"]);
});

const tool = (id: string, name: string, title: string, status: "done" | "running" | "failed" = "done"): WorkItem => ({ kind: "tool", id, tool: name, title, status });

test("a run of calls says what it did, as T3's work log", () => {
  expect(workSummary([tool("a", "commandExecution", "cargo test"), tool("b", "commandExecution", "cargo fmt")])).toBe("Ran 2 commands");
  // an edit counts files, Codex's several in a call; reads come after commands and edits
  expect(workSummary([tool("a", "Read", "a.rs"), tool("b", "fileChange", "a.rs, b.rs"), tool("c", "Bash", "ls")])).toBe("Changed 2 files, ran 1 command, and performed 1 other action");
  expect(workSummary([tool("a", "Grep", "fn main"), tool("b", "WebSearch", "nsh")])).toBe("Searched code once and searched the web once");
  expect(workSummary([{ kind: "reasoning", id: "r", text: "" }, { kind: "reasoning", id: "s", text: "" }])).toBe("Thought (×2)");
});

test("a command's program, and T3's durations", () => {
  expect(program("bash -lc 'RUST_LOG=1 cargo test -p x'")).toBe("cargo");
  expect(program("sudo /usr/bin/pacman -S git")).toBe("pacman");
  expect(duration(820)).toBe("820ms");
  expect(duration(4_240)).toBe("4.2s");
  expect(duration(80_000)).toBe("1m 20s");
  expect(duration(3_723_000)).toBe("1h 2m 3s");
});

test("a settled turn folds behind how long it worked but for its answer; the one running doesn't", () => {
  const items: Item[] = [
    { kind: "user", id: "u1", text: "fix it", at: "2026-05-02T12:00:00Z" },
    tool("c1", "commandExecution", "cargo test"),
    { kind: "assistant", id: "m1", text: "Looking.", at: "2026-05-02T12:00:10Z" },
    tool("c2", "fileChange", "a.rs"),
    { kind: "assistant", id: "m2", text: "Fixed.", at: "2026-05-02T12:01:20Z" },
    { kind: "user", id: "u2", text: "and the docs" },
    tool("c3", "commandExecution", "rg docs", "running"),
  ];
  const rows = rowsOf(items, true, new Set());
  expect(rows.map((r) => (r.kind === "item" ? r.item.id : r.kind))).toEqual(["u1", "fold", "m2", "u2", "working", "work"]);
  expect(rows[1]).toMatchObject({ kind: "fold", label: "Worked for 1m 20s", open: false });
  expect(rows[5]).toMatchObject({ kind: "work", live: true });
  // opened, all of it in order; settled, the last turn folds too, and says it was stopped
  const open = rowsOf(items, true, new Set(["fu1"]));
  expect(open.map((r) => (r.kind === "item" ? r.item.id : r.kind)).slice(0, 6)).toEqual(["u1", "fold", "work", "m1", "work", "m2"]);
  const stopped = rowsOf([...items.slice(0, 6), { ...(items[6] as WorkItem), status: "done" } as Item, { kind: "end", id: "e", ms: 4000, stopped: true }], false, new Set());
  expect(stopped[stopped.length - 1]).toMatchObject({ kind: "fold", label: "You stopped after 4s" });
});

test("a thought in a line, as T3's: its heading, else its first sentence", () => {
  expect(thoughtLine("**Checking the usage file**\n\nIt has a project field.")).toBe("Checking the usage file");
  expect(thoughtLine("Look at `run.rs` first. Then the docs.")).toBe("Look at run.rs first.");
  expect(thoughtLine("先看 run.rs。再看文档。")).toBe("先看 run.rs。");
  expect(latestThought([{ kind: "reasoning", id: "a", text: "Old." }, tool("b", "Bash", "ls"), { kind: "reasoning", id: "c", text: "" }])).toBe("Old.");
});

test("a turn's time as its agent wrote it, and where the user stopped it", () => {
  const items = fromHistory([
    { role: "user", text: "go", at: "2026-05-02T12:00:00Z" },
    { role: "tool", name: "shell", text: "ls", status: "done", at: "2026-05-02T12:00:05Z" },
    { role: "assistant", text: "Done.", at: "2026-05-02T12:00:09Z" },
    { role: "end", text: "", ms: 12_000, at: "2026-05-02T12:00:12Z" },
    { role: "user", text: "more", at: "2026-05-02T12:01:00Z" },
    { role: "tool", name: "Bash", text: "cargo test", at: "2026-05-02T12:01:30Z" },
    { role: "end", text: "", status: "stopped", at: "2026-05-02T12:01:42Z" },
  ]);
  const folds = rowsOf(items, false, new Set()).filter((r) => r.kind === "fold");
  // Codex's own count; Claude Code's stop, from the prompt to it
  expect(folds.map((r) => (r as { label: string }).label)).toEqual(["Worked for 12s", "You stopped after 42s"]);
});

test("the tool calls setting hides calls, the failed kept, or keeps only the answers", () => {
  const items: Item[] = [
    { kind: "user", id: "u", text: "go" },
    tool("a", "Bash", "ls"),
    tool("b", "Bash", "cargo test", "failed"),
    { kind: "reasoning", id: "r", text: "Hm." },
    { kind: "assistant", id: "m1", text: "Trying." },
    { kind: "assistant", id: "m2", text: "Done." },
  ];
  const calls = (rows: ReturnType<typeof rowsOf>) => rows.flatMap((r) => (r.kind === "work" ? r.items.map((i) => i.id) : []));
  const open = new Set(["fu"]);
  expect(calls(rowsOf(items, false, open, false, "showAll"))).toEqual(["a", "b", "r"]);
  expect(calls(rowsOf(items, false, open, false, "hideSuccessful"))).toEqual(["b", "r"]);
  expect(calls(rowsOf(items, false, open, false, "hideAll"))).toEqual(["r"]);
  const zen = rowsOf(items, true, open, false, "zen");
  // what was said, and "Thinking" while nothing comes; never a fold
  expect(zen.map((r) => (r.kind === "item" ? r.item.id : r.kind))).toEqual(["u", "m1", "m2", "thinking"]);
  expect(rowsOf(items, false, open, false, "zen").map((r) => (r.kind === "item" ? r.item.id : r.kind))).toEqual(["u", "m1", "m2"]);
});
