// Routing's lists under the card: the requests (the last the gateway keeps,
// or a day's from the usage records), a press puts one on the stage; and
// each account and key's tries over today's requests.
import { Text, View } from "nsh";
import { locale, tr, trn } from "./i18n";
import { gateway, go, groups, hideAccounts, providers, routeDay, routeDays, routePicked, showDay, tallies, type Provider, type Route } from "./model";
import { agentLogo, Label, Logo, providerLogo, TextButton } from "./parts";
import { cancelled, clock, failed, hueOf, keysOn, listed, metaOf, onStage, short, who } from "./routeStory";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "today", "yesterday", "Nov 8", "11月8日". */
function dayName(day: string): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const name = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  if (day === name(now)) return tr("today");
  if (day === name(new Date(now.getTime() - 86_400_000))) return tr("yesterday");
  const [, m, d] = day.split("-").map(Number);
  if (!m || !d) return day;
  return locale() === "zh-Hans" ? `${m}月${d}日` : `${MONTHS[m - 1]} ${d}`;
}

/** A pill of the filters: Live, today 2. */
function DayPill(props: { label: string; count?: number; on: boolean; onPress: () => void; testID?: string }) {
  return (
    <View
      role="button"
      testID={props.testID}
      onPress={props.onPress}
      class={`flex-row items-center gap-[5px] rounded-full border px-[9px] py-[3px] ${props.on ? "border-foreground" : "border-border hover:bg-pill"}`}
    >
      <Text class={`text-[12px] ${props.on ? "text-foreground" : "text-muted-foreground"}`}>{props.label}</Text>
      <If when={props.count !== undefined}>
        <Text class="text-[11px] text-faint">{String(props.count ?? "")}</Text>
      </If>
    </View>
  );
}

/** Where a request went: the account that answered, or how it ended. */
function Outcome(props: { route: Route }) {
  const r = () => props.route;
  const last = () => r().tries[r().tries.length - 1];
  const slot = () => (last() ? r().lineup[last()!.slot] : undefined);
  const dot = () => {
    if (!r().done) return "bg-muted-foreground";
    if (cancelled(r())) return "bg-muted-foreground";
    if (failed(r())) return "bg-destructive";
    return r().tries.length > 1 ? "bg-primary" : "bg-success";
  };
  const said = () => {
    const s = slot();
    if (!r().done) return s ? tr("asking %s…", who(s)) : tr("routing…");
    if (cancelled(r())) return tr("499 · canceled");
    if (failed(r())) return [String(r().status), short(r().error, s?.provider ?? "")].filter(Boolean).join(" · ");
    return s ? `${who(s)} · ${s.model}` : String(r().status);
  };
  return (
    <View class="min-w-0 flex-1 flex-row items-center gap-[6px]">
      <View class={`h-[6px] w-[6px] rounded-full ${dot()}`} />
      <If when={slot() && r().done && !failed(r()) && !cancelled(r())}>
        <Logo name={providerLogo(providers().find((p) => p.id === slot()!.provider)?.preset)} size={14} />
      </If>
      <Text class={`shrink text-[12px] line-clamp-1 ${failed(r()) ? "text-destructive" : "text-foreground"}`}>{said()}</Text>
    </View>
  );
}

function RequestRow(props: { route: Route }) {
  const r = () => props.route;
  const on = () => onStage()?.id === r().id;
  return (
    <View
      role="button"
      testID={`route-${r().id}`}
      onPress={() => routePicked.set(r().id)}
      class={`gap-[2px] rounded-[8px] border px-2 py-[6px] ${on() ? "border-border bg-card-2" : "border-transparent hover:bg-card-2"}`}
    >
      <View class="flex-row items-center gap-3">
        <Text class="w-[78px] text-[12px] text-muted-foreground">{clock(r().at)}</Text>
        <View class="w-[190px] flex-row items-center gap-[6px]">
          <View class="h-[8px] w-[8px] rounded-full" style={{ backgroundColor: hueOf(r().agent) }} />
          <Logo name={agentLogo(r().agent)} size={14} />
          <Text class="shrink text-[12px] text-fg-2 line-clamp-1">{r().requested}</Text>
        </View>
        <Outcome route={r()} />
      </View>
      <Text class="pl-[90px] text-[12px] text-muted-foreground line-clamp-1">{metaOf(r())}</Text>
    </View>
  );
}

/** The requests: Live's or a day's, newest first. */
export function RoutingRequests() {
  const list = () => [...listed()].reverse();
  const note = () => {
    const day = routeDay();
    const n = listed().length;
    return day === null ? tr("the last %d the gateway keeps", n) : trn(n, "%d request on %s", "%d requests on %s", n, dayName(day));
  };
  return (
    <View class="mt-[10px] gap-[6px]">
      <View class="min-h-[26px] flex-row items-center px-[2px]">
        <Label text={tr("Requests")} />
        <View class="flex-1" />
        <Text class="text-[11px] text-faint">{note()}</Text>
      </View>
      <View class="flex-row flex-wrap items-center gap-2">
        <DayPill label={tr("Live")} on={routeDay() === null} onPress={() => showDay(null)} testID="routing-live-pill" />
        <Each of={routeDays()} key={(d) => d.day}>
          {(d) => <DayPill label={dayName(d().day)} count={d().requests} on={routeDay() === d().day} onPress={() => showDay(d().day)} testID={`routing-day-${d().day}`} />}
        </Each>
      </View>
      <View testID="routing-list" class="rounded-[12px] border border-border bg-card p-1">
        <If
          when={list().length > 0}
          else={
            <View class="gap-1 px-3 pb-[18px] pt-[14px]">
              <Text class="text-[13px] font-medium text-foreground">{tr("No request yet")}</Text>
              <Text class="text-[12px] leading-[19px] text-muted-foreground">
                {routeDay() === null ? tr("Requests through pansy show here as they happen, the last 60 of them.") : tr("Nothing went through pansy this day.")}
              </Text>
            </View>
          }
        >
          <Each of={list()} key={(r) => r.id}>
            {(r) => <RequestRow route={r()} />}
          </Each>
        </If>
      </View>
    </View>
  );
}

/** One account or model under its provider, with its tries today. */
interface Fared {
  key: string;
  name: string;
  kind?: string;
  tried: number;
  answered: number;
  failed: number;
  last?: string;
  resting: number;
}

/** A provider's accounts (when it has several keys on) or models, as today's requests fared on them. */
function faredOf(p: Provider): Fared[] {
  const mine = tallies().filter((t) => t.provider === p.id);
  const resting = new Map((gateway()?.resting ?? []).map((r) => [r.key, r.secs]));
  const sum = (rows: typeof mine, key: string, name: string, kind?: string, rest = 0): Fared => ({
    key,
    name,
    kind,
    tried: rows.reduce((n, t) => n + t.tried, 0),
    answered: rows.reduce((n, t) => n + t.answered, 0),
    failed: rows.reduce((n, t) => n + t.failed, 0),
    last: rows.map((t) => t.last).filter((l): l is string => !!l).sort().pop(),
    resting: rest,
  });
  const keys = keysOn(p.id);
  if (keys.length >= 2) {
    return keys.map((k, i) =>
      sum(
        mine.filter((t) => t.account === k.display),
        k.id,
        hideAccounts() ? tr("key %d", i + 1) : k.display,
        tr("API key"),
        resting.get(`${p.id}:${k.id}`) ?? 0,
      ),
    );
  }
  const models = new Set(mine.map((t) => t.model));
  for (const g of groups()) for (const m of g.members) if (m.startsWith(`${p.id}/`)) models.add(m.slice(p.id.length + 1));
  const rest = keys[0] ? (resting.get(`${p.id}:${keys[0].id}`) ?? 0) : 0;
  return [...models].sort().map((m) => sum(mine.filter((t) => t.model === m), m, m, undefined, rest));
}

function FaredRow(props: { row: Fared }) {
  const f = () => props.row;
  const dot = () => (f().resting ? "bg-warning" : f().failed && !f().answered ? "bg-destructive" : f().answered ? "bg-success" : "bg-faint");
  return (
    <View class="flex-row pb-[9px] pr-3 pt-[6px]">
      <View class="w-[33px] pl-[19px] pt-[6px]">
        <View class={`h-[6px] w-[6px] rounded-full ${dot()}`} />
      </View>
      <View class="min-w-0 flex-1 gap-px">
        <View class="flex-row items-center gap-[6px]">
          <Text class="shrink text-[12px] font-medium text-foreground line-clamp-1">{f().name}</Text>
          <If when={f().kind}>
            <Text class="text-[12px] text-muted-foreground">{f().kind ?? ""}</Text>
          </If>
        </View>
        <If when={f().resting > 0}>
          <Text class="text-[12px] text-muted-foreground">{tr("resting · %d s left", f().resting)}</Text>
        </If>
        <View class="flex-row flex-wrap gap-x-[10px]">
          <Text class="text-[11px] text-muted-foreground">{tr("tried %d", f().tried)}</Text>
          <Text class="text-[11px] text-muted-foreground">{tr("answered %d", f().answered)}</Text>
          <If when={f().failed > 0}>
            <Text class="text-[11px] text-destructive">{tr("%d failed", f().failed)}</Text>
          </If>
          <If when={f().last}>
            <Text class="text-[11px] text-muted-foreground">{tr("last answered %s", clock(f().last ?? ""))}</Text>
          </If>
        </View>
      </View>
    </View>
  );
}

function ProviderFared(props: { provider: Provider; rows: Fared[] }) {
  return (
    <View testID={`routing-acts-${props.provider.id}`}>
      <View class="flex-row items-center gap-[7px] px-3 pb-1 pt-2">
        <Logo name={providerLogo(props.provider.preset)} size={14} />
        <Text class="shrink text-[12px] font-medium text-foreground line-clamp-1">{props.provider.name}</Text>
      </View>
      <Each of={props.rows} key={(r) => r.key}>
        {(r) => <FaredRow row={r()} />}
      </Each>
    </View>
  );
}

/** Accounts and keys: the providers routing uses, in up to three columns. */
export function RoutingAccounts() {
  const shown = () => {
    const used = new Set<string>(tallies().map((t) => t.provider));
    for (const g of groups()) for (const m of g.members) used.add(m.split("/")[0] ?? m);
    for (const r of listed()) for (const s of r.lineup) used.add(s.provider);
    return providers()
      .filter((p) => p.enabled && used.has(p.id))
      .map((p) => ({ provider: p, rows: faredOf(p) }))
      .filter((g) => g.rows.length > 0)
      .sort((a, b) => a.provider.name.localeCompare(b.provider.name));
  };
  const columns = () => {
    const all = shown();
    const cols: (typeof all)[] = Array.from({ length: Math.min(3, Math.max(1, all.length)) }, () => []);
    const heights = cols.map(() => 0);
    for (const g of all) {
      const at = heights.indexOf(Math.min(...heights));
      cols[at]!.push(g);
      heights[at]! += g.rows.length + 1;
    }
    return cols;
  };
  return (
    <If when={shown().length > 0}>
      <View class="mt-[10px] gap-[6px]">
        <View class="min-h-[26px] flex-row items-center px-[2px]">
          <Label text={tr("Accounts and keys")} />
          <View class="flex-1" />
          <Text class="text-[11px] text-faint">{tr("over today's requests")}</Text>
        </View>
        <View testID="routing-acts" class="flex-row rounded-[12px] border border-border bg-card py-1">
          <Each of={columns()} key={(c, i) => `${i}${c.map((g) => g.provider.id).join()}`}>
            {(c) => (
              <View class="min-w-0 flex-1">
                <Each of={c()} key={(g) => g.provider.id}>
                  {(g) => <ProviderFared provider={g().provider} rows={g().rows} />}
                </Each>
              </View>
            )}
          </Each>
        </View>
        <View class="flex-row justify-end" style={{ marginTop: -2 }}>
          <TextButton label={tr("View usage")} onPress={() => go("usage")} />
        </View>
      </View>
    </If>
  );
}
