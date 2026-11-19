// The subscriptions' allowances and the API keys' balances, as magpie shows
// them. On the Usage page, each account with its plan and, for each window
// its vendor counts (ChatGPT's 5 hours and week, Copilot's month, Code
// Assist's day of each model), a bar of what is used and when it renews,
// then each key's balance. In the quick panel's Allowances tab, magpie's
// cards: a vendor's accounts under its name, each with a ring a window and
// when they start again, the rings saying what is used or what is left;
// then the balances as figures, two to a row. An account its vendor holds
// says when it is back; the gateway sends the next one meanwhile.
import { Canvas, signal, Text, token, View } from "nsh";
import { Button as BaseButton } from "nsh/base";
import { tr } from "./i18n";
import { allowances, balances, providers, type Allowance, type AllowanceWindow, type Balance } from "./model";
import { Badge, Hint, List, Logo, logoNamed, MOTION, providerLogo, Row } from "./parts";

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
export function windowName(name: string): string {
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

function WindowLine(props: { window: AllowanceWindow }) {
  const w = () => props.window;
  return (
    <View class="h-[20px] flex-row items-center gap-[10px]">
      <Text class="w-[150px] text-[11.5px] text-fg-2 line-clamp-1">{windowName(w().name)}</Text>
      <If
        when={!w().unlimited}
        else={<Text class="flex-1 text-[11.5px] text-muted-foreground">{tr("Unlimited")}</Text>}
      >
        <View class="h-[6px] flex-1 overflow-hidden rounded-full bg-pill">
          <View class={`h-full rounded-full ${ink(w().used)}`} style={{ width: `${Math.min(100, Math.max(w().used > 0 ? 2 : 0, w().used))}%` }} />
        </View>
        <Text class="w-[38px] text-right font-mono text-[11px] text-foreground">{`${Math.round(w().used)}%`}</Text>
        <Text class="w-[80px] text-right font-mono text-[11px] text-muted-foreground">{w().display ?? ""}</Text>
        <Text class="w-[120px] text-right text-[11px] text-muted-foreground line-clamp-1">
          {w().resetsAt ? tr("renews in %s", untilText(w().resetsAt!)) : ""}
        </Text>
      </If>
    </View>
  );
}

function AllowanceRow(props: { allowance: Allowance; first: boolean }) {
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
        {(w) => <WindowLine window={w()} />}
      </Each>
      <If when={a().note}>
        <Text class="text-[11.5px] text-muted-foreground">{a().note ?? ""}</Text>
      </If>
    </Row>
  );
}

/** Every subscription account's allowance, in the providers' order. */
export function AllowanceList() {
  const list = () => allowances() ?? [];
  return (
    <List testID="allowances">
      <Each of={list()} key={(a) => `${a.provider}:${a.account}`}>
        {(a, i) => <AllowanceRow allowance={a()} first={i() === 0} />}
      </Each>
    </List>
  );
}

/** A key's balance on the Usage page: its provider and key, what is left, and a relay key's windows. */
function BalanceRow(props: { balance: Balance; first: boolean }) {
  const b = () => props.balance;
  const preset = () => providers().find((p) => p.id === b().provider)?.preset;
  return (
    <Row first={props.first} testID={`balance-${b().provider}-${b().account}`} class="flex-col items-stretch gap-[6px] py-[9px]">
      <View class="flex-row items-center gap-2">
        <Logo name={providerLogo(preset())} size={15} />
        <Text class="min-w-0 shrink text-[12.5px] font-medium text-foreground line-clamp-1">{b().user ? `${b().name} · ${b().user}` : b().name}</Text>
        <View class="flex-1" />
        <If when={!b().error && b().amount}>
          <Text class="font-mono text-[12.5px] font-semibold text-foreground">{b().amount ?? ""}</Text>
        </If>
      </View>
      <If when={b().error}>
        <Text class="text-[11.5px] text-muted-foreground line-clamp-2">{tr("Couldn't be read: %s", b().error ?? "")}</Text>
      </If>
      <Each of={b().windows ?? []} key={(w) => w.name}>
        {(w) => <WindowLine window={w()} />}
      </Each>
    </Row>
  );
}

/** Every key's balance pansy can read, in the providers' order. */
export function BalanceList() {
  return (
    <List testID="balances">
      <Each of={balances() ?? []} key={(b) => `${b.provider}:${b.account}`}>
        {(b, i) => <BalanceRow balance={b()} first={i() === 0} />}
      </Each>
    </List>
  );
}

// --- The quick panel's Allowances tab --------------------------------------------

/** The rings say what is left of each window, else what is used (magpie's Used / Left). */
const left = signal(false);

/** `5h`, `7d`: a window's span on its ring; any other name as it is said. */
function ringName(name: string): string {
  const span = name.match(/^(\d+) (minutes|hours|days)$/);
  if (!span) return windowName(name);
  return `${span[1]}${({ minutes: "m", hours: "h", days: "d" } as Record<string, string>)[span[2]!]}`;
}

/** When `iso` is, as magpie says a reset: 14:26, tomorrow 09:00, 11/20 09:00. */
function resetClock(iso: string): string {
  const at = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(at) - day(new Date())) / 86_400_000);
  if (days <= 0) return time;
  if (days === 1) return tr("tomorrow %s", time);
  return tr("%d/%d %s", at.getMonth() + 1, at.getDate(), time);
}

/** When the windows begun start again: the first bare, the others by their ring's name; "Not used yet" when none is. */
function resetsSaid(ws: AllowanceWindow[]): string {
  const begun = ws.filter((w) => w.resetsAt && w.used > 0);
  if (begun.length === 0) return ws.some((w) => w.used > 0) ? "" : tr("Not used yet");
  return begun
    .map((w, i) => {
      if (Date.parse(w.resetsAt!) <= Date.now()) return tr("Reset time passed");
      return i === 0 ? tr("Resets %s", resetClock(w.resetsAt!)) : `${ringName(w.name)} ${resetClock(w.resetsAt!)}`;
    })
    .join(" · ");
}

/** A window's ring: what is used (or left) as a dial with its share in it, amber once 90% is used; a press turns every ring to the other. */
function Ring(props: { window: AllowanceWindow }) {
  const used = () => Math.round(Math.min(100, Math.max(0, props.window.used)));
  const shown = () => (left() ? 100 - used() : used());
  const full = () => used() >= 90;
  return (
    <Hint text={`${windowName(props.window.name)} · ${props.window.display ? `${props.window.display} · ` : ""}${tr(left() ? "%d% left" : "%d% used", shown())}`}>
      <View role="button" testID={`ring-${props.window.name}`} onPress={() => left.set(!left())} class="items-center gap-[3px]">
        <View style={{ width: 34, height: 34 }}>
          <Canvas
            style={{ width: 34, height: 34 }}
            draw={(cx) => {
              cx.lineWidth = 3;
              cx.strokeStyle = full() ? token("warning") : token("fg-2");
              cx.globalAlpha = full() ? 0.2 : 0.12;
              cx.beginPath();
              cx.arc(17, 17, 15.5, 0, Math.PI * 2);
              cx.stroke();
              cx.globalAlpha = 1;
              if (shown() <= 0) return;
              cx.beginPath();
              cx.arc(17, 17, 15.5, -Math.PI / 2, -Math.PI / 2 + (Math.PI * 2 * shown()) / 100);
              cx.stroke();
            }}
          />
          <View class="absolute inset-0 items-center justify-center">
            <Text class={`text-[9.5px] font-semibold tabular-nums ${full() ? "text-foreground" : "text-fg-2"}`}>{`${shown()}%`}</Text>
          </View>
        </View>
        <Text class="max-w-[44px] text-[9.5px] text-muted-foreground line-clamp-1">{ringName(props.window.name)}</Text>
      </View>
    </Hint>
  );
}

/** A card of the tab: who on the left with a line or two under, the rings on the right. */
function Card(props: { testID: string; edge: string; who: string; say: string; warn?: boolean; more?: string; windows: AllowanceWindow[] }) {
  return (
    <View testID={props.testID} class="flex-row items-center gap-3 rounded-[11px] bg-card px-3 py-[10px]" style={{ boxShadow: props.edge }}>
      <View class="min-w-0 flex-1 gap-1">
        <Text class="text-[12.5px] text-foreground line-clamp-1">{props.who}</Text>
        <If when={props.say}>
          <Text class={`text-[11px] line-clamp-2 tabular-nums ${props.warn ? "text-warning" : "text-muted-foreground"}`}>{props.say}</Text>
        </If>
        <If when={props.more}>
          <Text class="text-[11px] text-muted-foreground line-clamp-1 tabular-nums">{props.more ?? ""}</Text>
        </If>
      </View>
      <If when={props.windows.length > 0}>
        <View class="flex-row gap-3">
          <Each of={props.windows} key={(w) => w.name}>
            {(w) => <Ring window={w()} />}
          </Each>
        </View>
      </If>
    </View>
  );
}

/** Rings for a card: three at most, none for a window with no limit. */
const ringed = (ws: AllowanceWindow[]) => ws.filter((w) => !w.unlimited).slice(0, 3);

/** What a subscription account's card says under its name. */
function accountSaid(a: Allowance): { say: string; warn: boolean } {
  if (a.error) return { say: tr("Couldn't be read: %s", a.error), warn: false };
  if (a.spentUntil) return { say: tr("Held · back in %s", untilText(a.spentUntil)), warn: true };
  if (a.windows.length === 0) return { say: tr("Its vendor says no allowance"), warn: false };
  if (ringed(a.windows).length === 0) return { say: tr("Unlimited"), warn: false };
  return { say: resetsSaid(ringed(a.windows)), warn: false };
}

/** Used or Left, above a vendor's rings: a press turns them. */
function Mode() {
  return (
    <Hint text={left() ? tr("Show how much of each window is used") : tr("Show how much of each window is left")}>
      <BaseButton
        onPress={() => left.set(!left())}
        testID="allowances-mode"
        class={`group h-[15px] items-center justify-center rounded-full bg-pill px-[6px] hover:bg-pill-hover ${MOTION}`}
      >
        <Text class={`text-[9.5px] font-semibold uppercase tracking-[0.04em] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{left() ? tr("Left") : tr("Used")}</Text>
      </BaseButton>
    </Hint>
  );
}

/** A section's head: the vendor's logo and name in small capitals, how many accounts or the one's plan, and Used / Left. */
function Head(props: { name: string; logo?: string | null; count?: number; note?: string; mode?: boolean }) {
  return (
    <View class="min-h-[15px] flex-row items-center gap-2">
      <If when={props.logo !== undefined}>
        <Logo name={providerLogo(props.logo)} size={14} />
      </If>
      <Text class="min-w-0 shrink text-[10.5px] font-semibold uppercase tracking-[0.07em] text-faint line-clamp-1">{props.name}</Text>
      <If when={(props.count ?? 0) > 1}>
        <View class="h-[15px] min-w-[16px] items-center justify-center rounded-full bg-pill px-[5px]">
          <Text class="text-[9.5px] font-semibold text-muted-foreground tabular-nums">{String(props.count ?? "")}</Text>
        </View>
      </If>
      <View class="flex-1" />
      <If when={props.note}>
        <Text class="min-w-0 shrink text-[12px] text-muted-foreground line-clamp-1">{props.note ?? ""}</Text>
      </If>
      <If when={props.mode}>
        <Mode />
      </If>
    </View>
  );
}

/** A balance as a figure: whose, and what is left, or why it couldn't be read. */
function Figure(props: { balance: Balance; edge: string }) {
  const b = () => props.balance;
  const preset = () => providers().find((p) => p.id === b().provider)?.preset;
  return (
    <View testID={`panel-balance-${b().provider}-${b().account}`} class="min-w-0 gap-1 rounded-[11px] bg-card px-3 py-[10px]" style={{ boxShadow: props.edge }}>
      <View class="flex-row items-center gap-[6px]">
        <Logo name={providerLogo(preset())} size={14} />
        <Text class="min-w-0 shrink text-[11px] text-muted-foreground line-clamp-1">{b().user ? `${b().name} · ${b().user}` : b().name}</Text>
      </View>
      <If when={!b().error} else={<Text class="text-[11px] text-muted-foreground line-clamp-3">{tr("Couldn't be read: %s", b().error ?? "")}</Text>}>
        <Text class="text-[15px] font-semibold text-foreground tabular-nums line-clamp-1">{b().amount ?? ""}</Text>
      </If>
    </View>
  );
}

/** Whether the panel has an Allowances tab: a subscription signed in, or a key whose balance was read. */
export const allowancesShown = () => providers().some((p) => p.subscription) || (balances()?.length ?? 0) > 0;

/**
 * The quick panel's Allowances tab: each subscription's accounts under its
 * name, a relay key with spending limits as one of them, then the other
 * balances as figures, two to a row. `edge` is the panel's card edge.
 */
export function PanelAllowances(props: { edge: string }) {
  const subscribed = () => providers().some((p) => p.subscription);
  const loading = () => (subscribed() && allowances() === null) || balances() === null;
  /** The subscriptions' accounts by provider, in the providers' order. */
  const groups = () => {
    const out: { provider: string; accounts: Allowance[] }[] = [];
    for (const a of allowances() ?? []) {
      const g = out.find((x) => x.provider === a.provider);
      if (g) g.accounts.push(a);
      else out.push({ provider: a.provider, accounts: [a] });
    }
    return out;
  };
  const limited = () => (balances() ?? []).filter((b) => (b.windows?.length ?? 0) > 0);
  const figures = () => (balances() ?? []).filter((b) => !b.windows?.length);
  const providerOf = (id: string) => providers().find((p) => p.id === id);
  return (
    <View testID="panel-allowances" class="gap-3 px-[2px] pb-[6px] pt-[2px]">
      <If when={!loading()} else={<><View class="h-[58px] rounded-[11px] bg-pill" /><View class="h-[58px] rounded-[11px] bg-pill" /></>}>
        <Each of={groups()} key={(g) => g.provider}>
          {(g) => (
            <View class="gap-2">
              <Head
                name={providerOf(g().provider)?.name ?? g().provider}
                logo={providerOf(g().provider)?.preset ?? null}
                count={g().accounts.length}
                note={g().accounts.length === 1 ? g().accounts[0]!.plan : undefined}
                mode={g().accounts.some((a) => !a.error && ringed(a.windows).length > 0)}
              />
              <Each of={g().accounts} key={(a) => a.account}>
                {(a) => <Card testID={`panel-allowance-${a().provider}-${a().account}`} edge={props.edge} who={a().user} {...accountSaid(a())} more={a().note} windows={a().error ? [] : ringed(a().windows)} />}
              </Each>
            </View>
          )}
        </Each>
        <Each of={limited()} key={(b) => `${b.provider}:${b.account}`}>
          {(b) => (
            <View class="gap-2">
              <Head name={b().name} logo={providerOf(b().provider)?.preset ?? null} mode />
              <Card
                testID={`panel-balance-${b().provider}-${b().account}`}
                edge={props.edge}
                who={b().user || b().name}
                say={b().error ? tr("Couldn't be read: %s", b().error ?? "") : resetsSaid(ringed(b().windows ?? []))}
                more={b().amount ? tr("%s left", b().amount ?? "") : undefined}
                windows={b().error ? [] : ringed(b().windows ?? [])}
              />
            </View>
          )}
        </Each>
        <If when={figures().length > 0}>
          <View class="gap-2">
            <Head name={tr("Balances")} />
            <View class="grid grid-cols-2 gap-2">
              <Each of={figures()} key={(b) => `${b.provider}:${b.account}`}>
                {(b) => <Figure balance={b()} edge={props.edge} />}
              </Each>
            </View>
          </View>
        </If>
      </If>
    </View>
  );
}
