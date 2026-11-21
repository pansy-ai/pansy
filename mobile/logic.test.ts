// The parts of the app that are plain functions: pairing links and
// addresses, the Markdown blocks, and how events fold into a thread's feed.
import { expect, test } from "nsh/test";
import { normalizeAddress, parsePairUrl } from "./net/pair";
import { apply, fromHistory, type FeedState } from "./state/thread";
import { blocks, spans } from "./ui/Markdown";

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
  expect(s.items.map((i) => i.kind)).toEqual(["user", "assistant", "tool"]);
  expect(s.items[2]).toEqual({ kind: "tool", id: "c1", tool: "commandExecution", title: "ran cargo test", status: "done", output: "ok" });
});

test("a question waits like an approval, with its choices", () => {
  let s: FeedState = { items: [], approvals: [], running: true, startedAt: null };
  const questions = [{ question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] }];
  s = apply(s, { type: "question", request: "q1", questions });
  expect(s.approvals).toEqual([{ request: "q1", kind: "question", title: "Pick", detail: "", questions }]);
  s = apply(s, { type: "resolved", request: "q1" });
  expect(s.approvals.length).toBe(0);
});
