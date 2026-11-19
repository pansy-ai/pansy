// What the Routing page says about a request: who each account is, what
// each try came to, why routing put the first one first, where the time
// went. Words only; ui/Routing.tsx draws them.
import { locale, tr, trn } from "./i18n";
import { agents, compact, dayRoutes, hideAccounts, providers, routeDay, routePicked, routes, type Route, type RouteSlot, type RouteTry, type Routing } from "./model";

/** The requests listed: Live's, or the day's; oldest first. */
export const listed = (): Route[] => (routeDay() === null ? routes() : dayRoutes());
/** The request on the stage: the one picked, else the latest. */
export const onStage = (): Route | undefined => {
  const list = listed();
  const id = routePicked();
  return (id !== null ? list.find((r) => r.id === id) : undefined) ?? list[list.length - 1];
};

/** magpie's twelve agent colors, one lightness around the wheel, none red. */
const PALETTE = ["#d97757", "#bd8638", "#a59234", "#849e4a", "#5aa66c", "#25a98f", "#00a6b0", "#2e9fc9", "#5e95d7", "#858ad7", "#a47fca", "#bb77b1"];
/** The place in it an agent asks for: its maker's color. */
const HUES: Record<string, number> = { "claude-code": 0, codex: 9, "gemini-cli": 7, "copilot-cli": 10, cursor: 5, opencode: 2, crush: 11, goose: 3, pi: 4 };
const given = new Map<string, string>();

/** The color an agent's requests are drawn in: the first from its place no other agent has. */
export function hueOf(agent: string): string {
  const had = given.get(agent);
  if (had) return had;
  let h = HUES[agent];
  if (h === undefined) {
    h = 0;
    for (const ch of agent) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  }
  const taken = new Set(given.values());
  const from = (k: number) => PALETTE[(h! + k) % PALETTE.length]!;
  const k = [...PALETTE.keys()].find((k) => !taken.has(from(k))) ?? 0;
  given.set(agent, from(k));
  return from(k);
}

export const providerById = (id: string) => providers().find((p) => p.id === id);
export const providerName = (id: string) => providerById(id)?.name ?? id;
export const keysOn = (id: string) => providerById(id)?.accounts.filter((a) => a.enabled) ?? [];
export const agentName = (id: string) => agents().find((a) => a.id === id)?.name ?? (id === "other" ? tr("An agent") : id);

/** An account by its key's name when its provider has several on, else by the provider's; "key 2" while accounts are hidden. */
export function who(s: { provider: string; account?: string }): string {
  const keys = keysOn(s.provider);
  if (!s.account || keys.length < 2) return providerName(s.provider);
  if (!hideAccounts()) return s.account;
  const at = keys.findIndex((k) => k.display === s.account);
  return tr("key %d", at < 0 ? 1 : at + 1);
}

/** "backup (claude-sonnet-4-6)". */
const whoModel = (s: RouteSlot) => tr("%s (%s)", who(s), s.model);

/** µs as magpie says a time: 0.3 ms, 487 ms, 1.2 s, 2 min 5 s. */
export function dur(us: number): string {
  const ms = us / 1000;
  if (ms < 10) return tr("%s ms", ms.toFixed(1));
  if (ms < 1000) return tr("%s ms", String(Math.round(ms)));
  const s = ms / 1000;
  if (s < 60) return tr("%s s", s < 10 ? s.toFixed(1) : String(Math.round(s)));
  return tr("%d min %d s", Math.floor(s / 60), Math.round(s % 60));
}

/** Seconds of rest: 30 s, 5 min, 1 h. */
export function secs(n: number): string {
  if (n < 60) return tr("%d s", n);
  if (n < 3600) return tr("%d min", Math.round(n / 60));
  return tr("%d h", Math.round(n / 3600));
}

/** A clock time: 07:03:16 PM, 19:03:16. */
export function clock(at: string): string {
  const d = new Date(at);
  const pad = (n: number) => String(n).padStart(2, "0");
  if (locale() === "zh-Hans") return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  const h = d.getHours() % 12 || 12;
  return `${pad(h)}:${pad(d.getMinutes())}:${pad(d.getSeconds())} ${d.getHours() < 12 ? "AM" : "PM"}`;
}

/** A vendor's error without the provider's name in front, and short. */
export function short(error: string | undefined, provider: string): string {
  let text = (error ?? "").trim();
  if (text.startsWith(`${provider}: `)) text = text.slice(provider.length + 2);
  return text.length > 90 ? `${text.slice(0, 89)}…` : text;
}

/** The try a slot had, if it had one. */
export const tryOf = (r: Route, slot: number): RouteTry | undefined => r.tries.find((t) => t.slot === slot);
/** The try that answered: the last, when it said yes. */
export const answering = (r: Route): RouteTry | undefined => {
  const t = r.tries[r.tries.length - 1];
  return t && t.status > 0 && t.status < 400 ? t : undefined;
};
export const cancelled = (r: Route) => r.done && r.status === 499;
export const failed = (r: Route) => r.done && r.status >= 400 && r.status !== 499;

/** How a slot looks on the stage: its dot and border, and its line. */
export type SlotKind = "idle" | "asking" | "on" | "failed" | "rest" | "spare";
export function slotState(r: Route, at: number): { kind: SlotKind; text: string } {
  const s = r.lineup[at]!;
  const t = tryOf(r, at);
  if (t) {
    if (t.status === 0) return { kind: "asking", text: tr("asking…") };
    if (t.status === 499) return { kind: "failed", text: tr("499 · canceled by %s", agentName(r.agent)) };
    if (t.status >= 400) {
      const why = [String(t.status), short(t.error, s.provider)].filter(Boolean).join(" · ");
      return { kind: "failed", text: t.rest ? tr("%s · rests %s", why, secs(t.rest)) : why };
    }
    if (cancelled(r)) return { kind: "on", text: tr("499 · canceled · passed to %s", agentName(r.agent)) };
    if (!r.done) return { kind: "asking", text: tr("answering…") };
    return { kind: "on", text: t.us ? tr("answered this request · %s", dur(t.us)) : tr("answered this request") };
  }
  if (s.resting) return { kind: "rest", text: tr("resting · %s left", secs(s.resting)) };
  if (s.spare) return { kind: "spare", text: tr("past the most tries a request makes") };
  if (!s.account) return { kind: "idle", text: tr("no key needed") };
  return { kind: "idle", text: keysOn(s.provider).length >= 2 ? tr("API key") : tr("one key") };
}

/** What the hub's chip says: how the group routes, or the keys' order. */
export function wayChip(r: Route): string | null {
  if (r.group) return ({ order: tr("In order"), rotate: tr("Take turns"), usage: tr("Least used") } as Record<Routing, string>)[r.group.routing];
  return r.lineup.length > 1 ? tr("In order") : null;
}

/** The card's head: what was asked for, and how it routes. */
export function topOf(r: Route): { name: string; rest: string; mode: string } {
  const g = r.group;
  if (g) {
    const mode = {
      order: tr("In order: member by member, the first model the group names until it can't answer, each over its own keys in their order; one resting after a failure goes last."),
      rotate: tr("In turn: each request starts at the next member, spreading the load; each member over its own keys in their order, one resting after a failure last."),
      usage: tr("Least used first: the member that has answered the fewest requests since pansy started goes first; each over its own keys in their order, one resting after a failure last."),
    }[g.routing];
    return { name: g.name, rest: trn(g.members.length, " · routing group · %d member", " · routing group · %d members", g.members.length), mode };
  }
  if (r.lineup.length === 0) return { name: r.requested, rest: tr(" · no route"), mode: r.error ?? "" };
  const providersIn = [...new Set(r.lineup.map((s) => s.provider))];
  if (providersIn.length > 1) {
    return {
      name: r.lineup[0]!.model,
      rest: trn(providersIn.length, " · offered by %d provider", " · offered by %d providers", providersIn.length),
      mode: tr("In order: the providers that offer it, in their order on the Providers page, each over its own keys; one resting after a failure goes last."),
    };
  }
  const s = r.lineup[0]!;
  if (!s.account) return { name: providerName(s.provider), rest: tr(" · %s · no key needed", s.model), mode: tr("It takes no key: it answers every request for this model.") };
  const n = keysOn(s.provider).length || r.lineup.length;
  return {
    name: providerName(s.provider),
    rest: trn(n, " · %s · %d key on", " · %s · %d keys on", s.model, n),
    mode: tr("In order: its first key until it can't answer, then the next; one resting after a failure goes last."),
  };
}

/**
 * How fast a request that answered wrote, as magpie's rows say it: its
 * output over the time from its first token to its last, else, for one
 * whose first token wasn't timed apart (a burst), at least its output over
 * its whole time, "≥", when that is 1,000 tokens or more over a second or
 * more; "" when it tells none, and for more than 10,000 a second.
 */
export function speedOf(r: Route): string {
  if (!r.done || r.status >= 400 || !r.output) return "";
  const writing = r.us - r.firstUs;
  if (r.firstUs > 0 && writing >= 100_000 && r.output * 1e6 <= 10_000 * writing && !(writing < 1e6 && r.firstUs > 4 * writing)) {
    return tr("%d tok/s", Math.round(r.output / (writing / 1e6)));
  }
  if (r.output >= 1000 && r.us >= 1e6 && r.output * 1e6 <= 10_000 * r.us) return `≥${tr("%d tok/s", Math.round(r.output / (r.us / 1e6)))}`;
  return "";
}

export type StepKind = "plain" | "why" | "aside" | "ok" | "bad" | "wait";
export interface Step {
  kind: StepKind;
  text: string;
}

/** "176 tokens · first token in 320 ms · 180 tok/s · request cache hit rate 0%". */
function answerFacts(r: Route): string {
  const facts: string[] = [];
  if (r.output || r.input) facts.push(tr("%s tokens", compact(r.input + r.output)));
  if (r.firstUs) facts.push(tr("first token in %s", dur(r.firstUs)));
  if (speedOf(r)) facts.push(speedOf(r));
  if (r.input) facts.push(tr("request cache hit rate %d%", Math.round((Math.min(r.cacheRead, r.input) / r.input) * 100)));
  return facts.length ? ` · ${facts.join(" · ")}` : "";
}

/** The story of a request: what was asked, why the first was first, each try. */
export function storyOf(r: Route): Step[] {
  const agent = agentName(r.agent);
  const steps: Step[] = [];
  steps.push({
    kind: "plain",
    text: r.group ? tr("%s asked for the routing group %s: %s", agent, r.group.name, r.group.members.join(", ")) : tr("%s asked for %s", agent, r.requested),
  });
  if (r.subagent) {
    steps.push({
      kind: "plain",
      text: r.asked
        ? tr("This is a subagent's request: it asked for %s, and %s's subagents go on this model instead.", r.asked, agent)
        : r.kept
          ? tr("This is a subagent's request. It stayed on the model it asked for: %s", r.kept)
          : tr("This is a subagent's request."),
    });
  }
  if (r.lineup.length === 0) {
    steps.push({ kind: "bad", text: tr("Nothing took this request: %s", r.error ?? "") });
    return steps;
  }
  steps.push({ kind: "why", text: whyOf(r) });
  if (r.tries.length === 0 && r.done && r.status >= 400) {
    steps.push({ kind: "bad", text: tr("Nothing took this request: %s", r.error ?? "") });
    return steps;
  }
  r.tries.forEach((t, i) => {
    const s = r.lineup[t.slot]!;
    const next = r.tries[i + 1];
    const last = i === r.tries.length - 1;
    if (t.status === 0) {
      steps.push({ kind: "wait", text: tr("Asking %s…", whoModel(s)) });
    } else if (t.status === 499) {
      steps.push({ kind: "bad", text: tr("%s canceled the request while %s was being asked: nobody failed, so nobody rests and nobody else is asked.", agent, whoModel(s)) });
    } else if (t.status >= 400) {
      const why = [String(t.status), short(t.error, s.provider)].filter(Boolean).join(" · ");
      const rest = again(t) + (t.rest ? tr(" It rests %s.", secs(t.rest)) : "");
      // A model that turned the request itself away is what the agent hears
      // when no model after it answered: the first that did, not the last.
      const heard = last && r.done && (t.status !== r.status || t.error !== r.error) ? r.tries.find((x) => x.status === r.status && x.error === r.error) : undefined;
      if (next) {
        steps.push({ kind: "bad", text: tr("%s answered %s; the request goes on to %s before any of the reply reaches %s.", whoModel(s), why, who(r.lineup[next.slot]!), agent) + rest });
      } else if (heard) {
        steps.push({ kind: "bad", text: tr("%s answered %s, and nobody is left to try, so %s gets what %s answered first: the request itself was turned away.", whoModel(s), why, agent, whoModel(r.lineup[heard.slot]!)) + rest });
      } else if (last && r.lineup.some((x, at) => at > t.slot && !x.spare && !tryOf(r, at))) {
        steps.push({ kind: "bad", text: tr("%s answered %s — an error another account wouldn't fix, so %s gets it.", whoModel(s), why, agent) + rest });
      } else {
        steps.push({ kind: "bad", text: tr("%s answered %s, and nobody is left to try, so %s gets the error.", whoModel(s), why, agent) + rest });
      }
    } else if (cancelled(r)) {
      steps.push({ kind: "bad", text: tr("%s canceled the request while %s was answering: nobody failed, so nobody rests and nobody else is asked.", agent, whoModel(s)) });
    } else if (!r.done) {
      steps.push({ kind: "wait", text: tr("%s is answering…", whoModel(s)) });
    } else {
      const fails = r.tries.length - 1;
      const answered = tr("%s answered in %s%s.", whoModel(s), dur(r.us), answerFacts(r));
      steps.push({ kind: "ok", text: (fails ? answered + tr(" %s got one clean reply and never saw the %d that failed first.", agent, fails) : answered) + again(t) });
    }
  });
  return steps;
}

/** What a try asked twice adds to its line. */
const again = (t: RouteTry) => (t.again ? tr(" Its connection broke the first time, so it was asked once more.") : "");

/** Why the first in line was first. */
function whyOf(r: Route): string {
  const first = r.lineup[0]!;
  const resting = r.lineup.filter((s) => s.resting);
  if (resting.length === r.lineup.length) return tr("Every one is resting after a failure, so %s, first in line, is tried all the same.", who(first));
  const rested = [...new Set(resting.map((s) => who(s)))].join(", ");
  if (r.group?.routing === "rotate") return tr("In turn: it's %s's turn — each request starts one further along.", who(first));
  if (r.group?.routing === "usage") return tr("Least used first: %s has answered the fewest requests — %d.", whoModel(first), first.used);
  if (r.lineup.length === 1) {
    return first.account ? tr("%s has one key on — nothing to choose between.", providerName(first.provider)) : tr("%s takes no key — nothing to choose between.", providerName(first.provider));
  }
  if (rested) return tr("In order: with %s resting after a failure, %s is the first that can answer.", rested, who(first));
  return tr("In order: %s is first, and answers everything while it can.", who(first));
}

/** The line under the stage: how the request ended, or where it is. */
export function captionOf(r: Route | undefined): string {
  if (!r) return "";
  const story = storyOf(r);
  return story[story.length - 1]?.text ?? "";
}

/** Where an answered request's time went (µs), when the gateway timed its tries. */
export interface Timing {
  masking: number;
  pansy: number;
  before: number;
  head: number;
  firstToken: number;
  streaming: number;
  vendor: number;
}

export function timingOf(r: Route): Timing | null {
  const a = answering(r);
  if (!a || !r.done || !a.headUs) return null;
  const before = r.tries.slice(0, -1).reduce((n, t) => n + t.us, 0);
  const pansy = Math.max(0, a.atUs - before);
  const firstToken = r.firstUs ? Math.max(0, r.firstUs - a.atUs - a.headUs) : 0;
  const streaming = Math.max(0, r.us - Math.max(r.firstUs, a.atUs + a.headUs));
  return { masking: Math.min(r.maskingUs, pansy), pansy, before, head: a.headUs, firstToken, streaming, vendor: a.headUs + firstToken + streaming };
}

/** A request's second line in the list: 520 ms · First token 353 ms · 176 tokens · Cache 0% · 180 tok/s. */
export function metaOf(r: Route): string {
  const parts: string[] = [];
  if (r.done) parts.push(dur(r.us));
  if (r.firstUs) parts.push(tr("First token %s", dur(r.firstUs)));
  if (r.input || r.output) parts.push(tr("%s tokens", compact(r.input + r.output)));
  if (r.input) parts.push(tr("Cache %d%", Math.round((Math.min(r.cacheRead, r.input) / r.input) * 100)));
  if (speedOf(r)) parts.push(speedOf(r));
  if (r.tries.length > 1) parts.push(trn(r.tries.length, "%d try", "%d tries", r.tries.length));
  return parts.join(" · ");
}
