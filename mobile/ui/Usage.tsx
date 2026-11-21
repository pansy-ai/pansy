// 用量: what pansy's gateway counted on a machine (usage.summary), the
// window's Usage page cut to what a phone needs: the range, three counts,
// the tokens a day as bars, and who used them, by model and by agent.
// Prompts and replies are never kept, here or on the computer.
import { resource, signal, Text, View } from "nsh";
import type { Totals, Usage } from "../net/types";
import { conn, machine, machines, online } from "../state/store";
import { tr, trn } from "./i18n";
import { pairOpen, tab } from "./nav";
import { agentLogo, agentName, Btn, Chip, Group, GroupHead, GroupRow, Loading, Logo, Note, providerLogo, Seg } from "./parts";
import { Page } from "./Settings";

type Range = "1" | "7" | "30";
const range = signal<Range>("7");
/** The machine chosen; the first online one until one is. */
const chosen = signal<string | null>(null);

/** 1019 → "1019", 12_400 → "12.4K", 3_400_000 → "3.4M". */
export function compact(n: number): string {
  const short = (v: number, unit: string) => `${v >= 100 ? Math.round(v) : Math.round(v * 10) / 10}${unit}`;
  if (n >= 1e9) return short(n / 1e9, "B");
  if (n >= 1e6) return short(n / 1e6, "M");
  if (n >= 1e4) return short(n / 1e3, "K");
  return String(n);
}

const tokens = (t: Totals) => t.input + t.output;

/** The rows of `by`, the most tokens first, the first `n`. */
export function top(by: Record<string, Totals>, n: number): [string, Totals][] {
  return Object.entries(by)
    .filter(([, t]) => t.requests > 0)
    .sort((a, b) => tokens(b[1]) - tokens(a[1]))
    .slice(0, n);
}

function Count(props: { value: string; label: string; danger?: boolean; testID: string; last?: boolean }) {
  return (
    <View class={`flex-1 gap-[2px] px-4 py-3 ${props.last ? "" : "border-r border-line"}`}>
      <Text testID={props.testID} class={`text-[22px] font-bold ${props.danger ? "text-danger" : "text-fg"}`}>
        {props.value}
      </Text>
      <Text class="text-[13px] text-fg-3">{props.label}</Text>
    </View>
  );
}

/** The tokens a day, oldest on the left, today on the right. */
function Bars(props: { by: Record<string, Totals> }) {
  const days = () => Object.entries(props.by).sort(([a], [b]) => a.localeCompare(b));
  const most = () => Math.max(1, ...days().map(([, t]) => tokens(t)));
  const label = (day: string) => day.slice(5).replace("-", "/").replace(/^0/, "").replace(/\/0/, "/");
  return (
    <Group class="px-4 pb-3 pt-4">
      <View class="h-[96px] flex-row items-end gap-[3px]" testID="usage-bars">
        <Each of={days()} key={([d]) => d}>
          {(d) => <View class="flex-1 rounded-t-[3px] bg-primary" style={{ height: Math.max(2, Math.round((tokens(d()[1]) / most()) * 96)), opacity: tokens(d()[1]) ? 1 : 0.25 }} />}
        </Each>
      </View>
      <View class="mt-[6px] flex-row justify-between">
        <Text class="font-mono text-[12px] text-fg-3">{label(days()[0]?.[0] ?? "")}</Text>
        <Text class="text-[12px] text-fg-3">{tr("Today")}</Text>
      </View>
    </Group>
  );
}

function Who(props: { title: string; rows: [string, Totals][]; kind: "model" | "agent" }) {
  const all = () => props.rows.reduce((n, [, t]) => n + tokens(t), 0) || 1;
  return (
    <If when={props.rows.length}>
      <GroupHead text={props.title} />
      <Group testID={`usage-by-${props.kind}`}>
        <Each of={props.rows} key={([k]) => k}>
          {(r, i) => {
            // a row stays while its key does: its counts are read anew from `r`
            const key = () => r()[0];
            const t = () => r()[1];
            const sub = () => [props.kind === "model" ? key().slice(0, key().indexOf("/")) : "", trn(t().requests, "1 request", "%d requests", t().requests), t().errors ? trn(t().errors, "1 failed", "%d failed", t().errors) : ""].filter(Boolean).join(" · ");
            return (
              <GroupRow
                first={i() === 0}
                lead={<Logo name={props.kind === "agent" ? agentLogo(key()) : providerLogo(key())} size={22} />}
                title={props.kind === "agent" ? agentName(key()) : key().slice(key().indexOf("/") + 1)}
                sub={sub()}
                trailing={
                  <View class="items-end gap-[2px]">
                    <Text class="font-mono text-[15px] text-fg">{compact(tokens(t()))}</Text>
                    <Text class="text-[12px] text-fg-3">{`${Math.round((tokens(t()) / all()) * 100)}%`}</Text>
                  </View>
                }
              />
            );
          }}
        </Each>
      </Group>
    </If>
  );
}

export function UsagePage() {
  const id = () => {
    const c = chosen();
    if (c && machine(c)) return c;
    return (machines().find((m) => online(m.id)) ?? machines()[0])?.id ?? null;
  };
  // read again each time the tab is opened, the range or the machine changes
  const usage = resource(
    () => (tab() === "usage" && id() && online(id()!) ? `${id()}|${range()}` : null),
    async (key) => {
      const [m, days] = key.split("|");
      return conn(m)!.call<Usage>("usage.summary", { days: Number(days) });
    },
  );
  return (
    <Page title={tr("Usage")} testID="usage">
      <If
        when={machines().length}
        else={
          <View class="items-center gap-3 px-4 pt-20" testID="usage-empty">
            <Text class="text-center text-[14px] text-fg-2">{tr("Pair a computer to see what its agents used through pansy.")}</Text>
            <Btn label={tr("Add machine")} onPress={() => pairOpen.set(true)} class="mt-2 self-center" />
          </View>
        }
      >
        <If when={machines().length > 1}>
          <View class="flex-row flex-wrap gap-2">
            <Each of={machines()} key={(m) => m.id}>
              {(m) => <Chip label={m().name} on={m().id === id()} onPress={() => chosen.set(m().id)} testID={`usage-machine-${m().id}`} />}
            </Each>
          </View>
        </If>
        <Seg<Range>
          testID="usage-range"
          value={range()}
          onChange={range.set}
          options={[
            { value: "1", label: tr("Today") },
            { value: "7", label: tr("7 days") },
            { value: "30", label: tr("30 days") },
          ]}
        />
        <If when={id() && online(id()!)} else={<Note tone="warning" icon="wifi-off" text={tr("%s is offline", machine(id() ?? "")?.name ?? "")} />}>
          <If
            when={usage()}
            else={
              <If when={usage.error()} else={<Loading text={tr("Reading the usage…")} />}>
                <Note tone="danger" text={tr("Couldn't read it: %s", String((usage.error() as Error)?.message ?? ""))} action={{ label: tr("Retry"), onPress: () => usage.refetch() }} />
              </If>
            }
          >
            <Group class="flex-row">
              <Count value={compact(tokens(usage()!.total))} label={tr("Tokens")} testID="usage-tokens" />
              <Count value={compact(usage()!.total.requests)} label={tr("Requests")} testID="usage-requests" />
              <Count value={compact(usage()!.total.errors)} label={tr("Failed")} danger={usage()!.total.errors > 0} testID="usage-errors" last />
            </Group>
            <If when={usage()!.days > 1}>
              <Bars by={usage()!.byDay} />
            </If>
            <If when={usage()!.total.requests} else={<Text class="px-4 text-center text-[14px] text-fg-2">{tr("Nothing went through the gateway in this time.")}</Text>}>
              <Who title={tr("By model")} rows={top(usage()!.byModel, 6)} kind="model" />
              <Who title={tr("By agent")} rows={top(usage()!.byAgent, 6)} kind="agent" />
            </If>
          </If>
        </If>
        <Text class="px-4 text-[13px] text-fg-3">{tr("Counted by pansy's gateway on the computer. Prompts and replies are never kept.")}</Text>
      </If>
    </Page>
  );
}
