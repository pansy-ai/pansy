// The subscriptions' allowances, as magpie shows them: each account with
// its plan and, for each window its vendor counts (ChatGPT's 5 hours and
// week, Copilot's month, Code Assist's day of each model), a bar of what
// is used and when it renews. An account its vendor holds says when it is
// back; the gateway sends the next one meanwhile. On the Usage page and in
// the quick panel's Allowances tab.
import { Text, View } from "nsh";
import { tr } from "./i18n";
import { allowances, providers, type Allowance, type AllowanceWindow } from "./model";
import { Badge, List, Logo, logoNamed, Row } from "./parts";

/** `2h 5m`, `3d 4h`: how long until `iso`. */
export function untilText(iso: string): string {
  const secs = Math.max(0, Math.round((Date.parse(iso) - Date.now()) / 1000));
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return tr("%dd %dh", d, h);
  if (h > 0) return tr("%dh %dm", h, m);
  return tr("%dm", Math.max(1, m));
}

/** A window's name: the spans and Copilot's counts in the user's language; a model's id as it is. */
function windowName(name: string): string {
  const span = name.match(/^(\d+) (hours|days|minutes)$/);
  if (span) {
    const n = Number(span[1]);
    return span[2] === "hours" ? tr("%d hours", n) : span[2] === "days" ? tr("%d days", n) : tr("%d minutes", n);
  }
  switch (name) {
    case "Premium requests":
      return tr("Premium requests");
    case "Chat requests":
      return tr("Chat requests");
    case "Completions":
      return tr("Completions");
  }
  return name;
}

/** The bar's ink: spent is red, nearly spent amber. */
const ink = (used: number) => (used >= 100 ? "bg-destructive" : used >= 80 ? "bg-warning" : "bg-primary");

function WindowLine(props: { window: AllowanceWindow; compact?: boolean }) {
  const w = () => props.window;
  return (
    <View class="h-[20px] flex-row items-center gap-[10px]">
      <Text class={`${props.compact ? "w-[110px]" : "w-[150px]"} text-[11.5px] text-fg-2 line-clamp-1`}>{windowName(w().name)}</Text>
      <If
        when={!w().unlimited}
        else={<Text class="flex-1 text-[11.5px] text-muted-foreground">{tr("Unlimited")}</Text>}
      >
        <View class="h-[6px] flex-1 overflow-hidden rounded-full bg-pill">
          <View class={`h-full rounded-full ${ink(w().used)}`} style={{ width: `${Math.min(100, Math.max(w().used > 0 ? 2 : 0, w().used))}%` }} />
        </View>
        <Text class="w-[38px] text-right font-mono text-[11px] text-foreground">{`${Math.round(w().used)}%`}</Text>
        <If when={!props.compact}>
          <Text class="w-[80px] text-right font-mono text-[11px] text-muted-foreground">{w().display ?? ""}</Text>
        </If>
        <Text class={`${props.compact ? "w-[78px]" : "w-[120px]"} text-right text-[11px] text-muted-foreground line-clamp-1`}>
          {w().resetsAt ? tr("renews in %s", untilText(w().resetsAt!)) : ""}
        </Text>
      </If>
    </View>
  );
}

function AllowanceRow(props: { allowance: Allowance; first: boolean; compact?: boolean }) {
  const a = () => props.allowance;
  const provider = () => providers().find((p) => p.id === a().provider);
  return (
    <Row first={props.first} testID={`allowance-${a().provider}-${a().account}`} class="flex-col items-stretch gap-[6px] py-[9px]">
      <View class="flex-row items-center gap-2">
        <Logo name={logoNamed(provider()?.icon ?? a().provider)} size={15} />
        <Text class="min-w-0 shrink text-[12.5px] font-medium text-foreground line-clamp-1">{`${provider()?.name ?? a().provider} · ${a().user}`}</Text>
        <If when={a().plan}>
          <Badge label={a().plan ?? ""} />
        </If>
        <If when={!a().enabled}>
          <Badge label={tr("Switched off")} />
        </If>
        <View class="flex-1" />
        <If when={a().spentUntil}>
          <Badge tone="warning" testID={`allowance-held-${a().provider}-${a().account}`} label={tr("Held · back in %s", untilText(a().spentUntil ?? ""))} />
        </If>
      </View>
      <If when={a().error}>
        <Text class="text-[11.5px] text-muted-foreground line-clamp-2">{tr("Couldn't be read: %s", a().error ?? "")}</Text>
      </If>
      <If when={!a().error && a().windows.length === 0}>
        <Text class="text-[11.5px] text-muted-foreground">{tr("Its vendor says no allowance")}</Text>
      </If>
      <Each of={a().windows} key={(w) => w.name}>
        {(w) => <WindowLine window={w()} compact={props.compact} />}
      </Each>
      <If when={a().note}>
        <Text class="text-[11.5px] text-muted-foreground">{a().note ?? ""}</Text>
      </If>
    </Row>
  );
}

/** Every subscription account's allowance, in the providers' order. */
export function AllowanceList(props: { compact?: boolean }) {
  const list = () => allowances() ?? [];
  return (
    <List testID="allowances">
      <Each of={list()} key={(a) => `${a.provider}:${a.account}`}>
        {(a, i) => <AllowanceRow allowance={a()} first={i() === 0} compact={props.compact} />}
      </Each>
    </List>
  );
}
