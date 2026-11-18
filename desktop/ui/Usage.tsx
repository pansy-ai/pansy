// Usage, as magpie shows it: the range on a segmented control, four counts
// in one card (tokens, cache reads, reasoning, calls), the tokens a day as
// stacked bars, then who used them (by agent, model and provider) and the
// latest requests. Counted on this computer; prompts and replies are never
// kept. With a subscription signed in to, its accounts' allowances first.
import { effect, signal, Text, View } from "nsh";
import { AllowanceList } from "./Allowances";
import { locale, tr, trn } from "./i18n";
import { agents, allowances, compact, loadAllowances, loadUsage, providers, usage, usageDays, type Totals } from "./model";
import { agentLogo, Badge, Icon, IconButton, List, Logo, Mono, Page, providerLogo, Row, RowHead, Segs, type LogoName } from "./parts";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "2026-11-08" → "Nov 8", "11月8日". */
export function dayLabel(day: string): string {
  const [, m, d] = day.split("-").map(Number);
  if (!m || !d) return day;
  return locale() === "zh-Hans" ? `${m}月${d}日` : `${MONTHS[m - 1]} ${d}`;
}

function Kpi(props: { value: string; label: string; sub: string; testID?: string; last?: boolean }) {
  return (
    <View class={`flex-1 gap-[2px] bg-card px-[14px] pb-[11px] pt-3 ${props.last ? "" : "border-r border-border"}`}>
      <Text testID={props.testID} class="text-[20px] font-semibold text-foreground">
        {props.value}
      </Text>
      <Text class="text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground">{props.label}</Text>
      <Text class="text-[11px] text-muted-foreground line-clamp-1">{props.sub}</Text>
    </View>
  );
}

/** Tokens a day as bars: output on top in the accent, input under it lighter. */
function Chart() {
  const days = () => Object.entries(usage()?.byDay ?? {});
  const top = () => Math.max(1, ...days().map(([, t]) => t.input + t.output));
  return (
    <List class="px-[14px] pb-[10px] pt-3">
      <Text class="self-end text-[10.5px] text-faint">{compact(top())}</Text>
      <View class="mt-1 h-[96px] flex-row items-end gap-[3px] border-b border-line-2">
        <Each of={days()} key={(d) => d[0]}>
          {(d) => (
            <View class="flex-1 justify-end" style={{ height: `${Math.max(1, Math.round(((d()[1].input + d()[1].output) / top()) * 100))}%` }}>
              <View class="w-full rounded-t-[2px] bg-primary" style={{ flexGrow: d()[1].output || 0.0001 }} />
              <View class="w-full bg-primary/35" style={{ flexGrow: d()[1].input || 0.0001 }} />
            </View>
          )}
        </Each>
      </View>
      <View class="mt-[5px] flex-row gap-[3px]">
        <Each of={days()} key={(d) => d[0]}>
          {(d, i) => (
            <Text class={`flex-1 text-[10.5px] text-muted-foreground ${i() === 0 ? "" : i() === days().length - 1 ? "text-right" : "text-center"}`}>
              {days().length <= 10 || i() % Math.ceil(days().length / 8) === 0 ? dayLabel(d()[0]) : ""}
            </Text>
          )}
        </Each>
      </View>
    </List>
  );
}

/** Who used the tokens: one row each, the share of the whole as a bar. */
function Breakdown(props: { title: string; rows: Record<string, Totals>; logo: (key: string) => LogoName; name: (key: string) => string; testID?: string }) {
  const open = signal(true);
  const rows = () => Object.entries(props.rows).sort((a, b) => b[1].input + b[1].output - (a[1].input + a[1].output));
  const whole = () => Math.max(1, rows().reduce((n, [, t]) => n + t.input + t.output, 0));
  return (
    <View class="gap-[6px]">
      <RowHead>
        <View role="button" onPress={() => open.set(!open())} class="h-[22px] flex-row items-center gap-[5px] rounded-[6px] pl-1 pr-[6px] hover:bg-pill">
          <Icon name={open() ? "chev-down" : "chev-right"} color="muted-foreground" size={10} />
          <Text class="text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground">{props.title}</Text>
          <Text class="text-[11px] text-faint">{String(rows().length)}</Text>
        </View>
      </RowHead>
      <If when={open() && rows().length > 0}>
        <List testID={props.testID}>
          <Each of={rows()} key={(r) => r[0]}>
            {(r, i) => {
              const t = () => r()[1];
              const tokens = () => t().input + t().output;
              return (
                <Row first={i() === 0} class="min-h-[52px]">
                  <Logo name={props.logo(r()[0])} size={18} />
                  <View class="w-[200px] gap-[1px]">
                    <Text class="text-[13px] font-medium text-foreground line-clamp-1">{props.name(r()[0])}</Text>
                    <Text class="text-[11.5px] text-muted-foreground line-clamp-1">
                      {[trn(t().requests, "%d call", "%d calls", t().requests), ...(t().errors ? [tr("%d failed", t().errors)] : [])].join(" · ")}
                    </Text>
                  </View>
                  <View class="mx-[6px] h-1 flex-1 overflow-hidden rounded-[2px] bg-pill">
                    <View class="h-full rounded-[2px] bg-primary/70" style={{ width: `${Math.round((tokens() / whole()) * 1000) / 10}%` }} />
                  </View>
                  <View class="min-w-[150px] items-end">
                    <Text class="text-[13px] font-semibold text-foreground">{compact(tokens())}</Text>
                    <Text class="text-[10.5px] text-muted-foreground">{tr("%s in · %s out · %s cached", compact(t().input), compact(t().output), compact(t().cacheRead))}</Text>
                  </View>
                </Row>
              );
            }}
          </Each>
        </List>
      </If>
    </View>
  );
}

function Requests() {
  const time = (at: string) => {
    const d = new Date(at);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  };
  return (
    <View class="gap-[6px]">
      <RowHead label={tr("Latest requests")} />
      <List>
        <Each of={usage()?.latest ?? []} key={(r) => `${r.at}${r.requested}${r.ms}`}>
          {(r, i) => (
            <Row first={i() === 0} class="min-h-[36px] py-[5px]">
              <Text class="w-[58px] text-[11.5px] text-muted-foreground">{time(r().at)}</Text>
              <Logo name={agentLogo(r().agent)} size={14} />
              <View class="min-w-0 flex-1">
                <Mono text={`${r().provider}/${r().model}`} class="text-[11.5px] line-clamp-1" />
                <If when={r().error}>
                  <Text class="text-[11px] text-destructive line-clamp-1">{r().error ?? ""}</Text>
                </If>
              </View>
              <If when={r().failovers > 0}>
                <Badge label={trn(r().failovers, "+%d try", "+%d tries", r().failovers)} tone="warning" />
              </If>
              <Badge label={String(r().status)} tone={r().status >= 400 ? "danger" : "success"} />
              <Text class="w-[90px] text-right text-[11.5px] text-muted-foreground">{`${compact(r().input)} → ${compact(r().output)}`}</Text>
              <Text class="w-[44px] text-right text-[11.5px] text-muted-foreground">{`${(r().ms / 1000).toFixed(1)} s`}</Text>
            </Row>
          )}
        </Each>
      </List>
    </View>
  );
}

export function UsagePage() {
  const subscribed = () => providers().some((p) => p.subscription);
  // Read when the page first shows with a subscription; kept a minute.
  effect(() => {
    if (subscribed() && allowances() === null) void loadAllowances();
  });
  const total = () => usage()?.total;
  const hit = () => {
    const t = total();
    return t && t.input + t.cacheRead > 0 ? Math.round((t.cacheRead / (t.input + t.cacheRead)) * 100) : 0;
  };
  const agentName = (id: string) => agents().find((a) => a.id === id)?.name ?? id;
  const providerName = (id: string) => providers().find((p) => p.id === id)?.name ?? id;
  const providerPreset = (id: string) => providerLogo(providers().find((p) => p.id === id)?.preset ?? id);
  return (
    <Page testID="page-usage">
      <View class="h-[34px] flex-row items-center gap-[10px]">
        <Segs
          testID="usage-days"
          value={String(usageDays()) as "1" | "7" | "30"}
          onChange={(v) => {
            usageDays.set(Number(v));
            void loadUsage();
          }}
          options={[
            { value: "1", label: tr("Today") },
            { value: "7", label: tr("7 days") },
            { value: "30", label: tr("30 days") },
          ]}
        />
        <View class="flex-1" />
        <IconButton icon="sync" label={tr("Refresh")} onPress={loadUsage} />
        <Text class="text-[12px] text-muted-foreground">{trn(total()?.requests ?? 0, "%d call", "%d calls", total()?.requests ?? 0)}</Text>
      </View>
      <If when={subscribed()}>
        <RowHead label={tr("Allowances")} note={tr("what each subscription account has used, as its vendor says")}>
          <IconButton icon="sync" label={tr("Read again")} testID="allowances-refresh" onPress={() => void loadAllowances(true)} />
        </RowHead>
        <If when={allowances()} else={<Text class="px-1 text-[12px] text-muted-foreground">{tr("Reading the allowances…")}</Text>}>
          <AllowanceList />
        </If>
      </If>
      <If
        when={(total()?.requests ?? 0) > 0}
        else={
          <View class="items-center gap-2 rounded-[12px] border border-dashed border-border px-6 py-8">
            <Text class="text-[13px] font-medium text-foreground">{tr("No requests yet")}</Text>
            <Text class="text-center text-[12px] text-muted-foreground">{tr("Connect an agent and use it: its requests show here.")}</Text>
          </View>
        }
      >
        <Text class="px-1 text-[11px] text-faint">{tr("Gateway calls only, counted on this computer. Prompts and replies are never kept.")}</Text>
        <View class="flex-row overflow-hidden rounded-[12px] border border-border">
          <Kpi value={compact((total()?.input ?? 0) + (total()?.output ?? 0))} label={tr("Tokens")} sub={tr("%s in · %s out", compact(total()?.input ?? 0), compact(total()?.output ?? 0))} testID="usage-tokens" />
          <Kpi value={compact(total()?.cacheRead ?? 0)} label={tr("Cache read")} sub={tr("hit rate %d%", hit())} />
          <Kpi value={compact(total()?.reasoning ?? 0)} label={tr("Reasoning")} sub={tr("inside output")} />
          <Kpi value={compact(total()?.requests ?? 0)} label={tr("Calls")} sub={tr("%d failed", total()?.errors ?? 0)} testID="usage-requests" last />
        </View>
        <Chart />
        <Breakdown title={tr("By agent")} rows={usage()?.byAgent ?? {}} logo={agentLogo} name={agentName} testID="usage-agents" />
        <Breakdown title={tr("By model")} rows={usage()?.byModel ?? {}} logo={(k) => providerPreset(k.split("/")[0] ?? k)} name={(k) => k} testID="usage-models" />
        <Breakdown title={tr("By provider")} rows={usage()?.byProvider ?? {}} logo={providerPreset} name={providerName} />
        <If when={Object.keys(usage()?.byProject ?? {}).length > 0}>
          <Breakdown title={tr("By project")} rows={usage()?.byProject ?? {}} logo={() => "generic"} name={(k) => k} testID="usage-projects" />
          <Text class="px-1 text-[11px] text-faint">{tr("A project is the folder an agent was started in with pansy run; agents started another way are under none.")}</Text>
        </If>
        <Requests />
      </If>
    </Page>
  );
}
