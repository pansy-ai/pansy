// Insights › Masking, as magpie shows it: what masking kept from vendors in
// the last days (a count, a bar a day, which agents' requests to which
// vendors), then each request masking changed, by day, or each value; a
// request opens on its own with the steps it went through, its text before
// and after (while it is among the latest, in memory), and how each value
// was masked. Records never hold the values themselves.
import { effect, signal, Text, token, View } from "nsh";
import { Masking } from "pansy/native";
import { locale, tr, trn } from "./i18n";
import {
  agents,
  ask,
  compact,
  go,
  kindLabel,
  loadMasking,
  maskDays,
  masking,
  maskingOn,
  maskOpen,
  PERSONAL_KINDS,
  providers,
  reason,
  setMasking,
  settings,
  settingsTab,
  toast,
  type MaskChange,
  type MaskRecord,
  type MaskValue,
} from "./model";
import { agentLogo, Badge, Icon, IconButton, List, Logo, Mono, Page, PrimaryButton, providerLogo, Row, RowHead, Segs, TextButton } from "./parts";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const pad = (n: number) => String(n).padStart(2, "0");
const dayOf = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** "2026-11-08" → "Nov 8", "11月8日"; today and yesterday by name. */
function dayLabel(day: string): string {
  const now = new Date();
  if (day === dayOf(now)) return tr("Today");
  if (day === dayOf(new Date(now.getTime() - 86400000))) return tr("Yesterday");
  const [, m, d] = day.split("-").map(Number);
  if (!m || !d) return day;
  return locale() === "zh-Hans" ? `${m}月${d}日` : `${MONTHS[m - 1]} ${d}`;
}

const timeOf = (t: string) => {
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};

/** What a record's crumb says: "Today 11:32:05". */
export function maskingCrumb(): string {
  const r = masking()?.records.find((x) => x.key === maskOpen());
  return r ? `${dayLabel(dayOf(new Date(r.t)))} ${timeOf(r.t)}` : "";
}

const agentName = (id: string) => agents().find((a) => a.id === id)?.name ?? (id === "other" ? tr("Other apps") : id);
const providerName = (id: string) => providers().find((p) => p.id === id)?.name ?? id;
const providerLogoOf = (id: string) => providerLogo(providers().find((p) => p.id === id)?.preset ?? id);

/** The group of a kind, for the filters: secrets, personal data, the user's words, the user's rules. */
type Filter = "all" | "secrets" | "personal" | "words" | "rules";
function groupOf(v: MaskValue): Filter {
  if (v.b === "word" || v.k === "TERM") return "words";
  if (v.b === "yours") return "rules";
  if (PERSONAL_KINDS.includes(v.k) || ["SSN", "PASSPORT", "IBAN", "BIRTHDAY", "MAC", "SERIAL", "USERNAME", "HOSTNAME", "IP", "BUCKET"].includes(v.k)) return "personal";
  return "secrets";
}

const foundBy = (b: string) => {
  switch (b) {
    case "word":
      return tr("Your words");
    case "yours":
      return tr("Your rules");
    case "known":
      return tr("Masked before");
  }
  return tr("Rules");
};

/** What a value looked like, without it: its first characters, then dots for the rest. */
const shapeOf = (v: MaskValue) => `${v.h ?? ""}${"•".repeat(Math.max(3, Math.min(v.l - (v.h?.length ?? 0), 10)))}`;

/** Places masked in a record (those warned of went as written). */
const maskedIn = (r: MaskRecord) => r.values.filter((v) => !v.a).reduce((n, v) => n + v.n, 0);

function Summary() {
  const sum = () => masking();
  const days = () => Object.entries(sum()?.byDay ?? {});
  const top = () => Math.max(1, ...days().map(([, n]) => n));
  const agentsRows = () => Object.entries(sum()?.byAgent ?? {}).sort((a, b) => b[1] - a[1]).slice(0, 4);
  const vendorRows = () => Object.entries(sum()?.byProvider ?? {}).sort((a, b) => b[1] - a[1]).slice(0, 4);
  return (
    <List testID="masking-summary">
      <View class="flex-row items-start gap-3 px-[16px] pt-[14px]">
        <View class="flex-1 gap-[2px]">
          <Text class="text-[13.5px] font-semibold text-foreground">{tr("Kept from vendors")}</Text>
          <Text class="text-[11.5px] text-muted-foreground">{tr("Masked on this computer before each request left it, and put back in each answer")}</Text>
        </View>
        <TextButton
          label={tr("Settings › Privacy")}
          onPress={() => {
            settingsTab.set("privacy");
            go("settings");
          }}
        />
        <TextButton label={tr("Turn off masking")} tone="muted" testID="masking-off" onPress={() => void setMasking({ secrets: false, personal: false }).then(() => loadMasking())} />
        <Segs
          small
          testID="masking-days"
          value={String(maskDays()) as "7" | "30"}
          onChange={(v) => {
            maskDays.set(Number(v));
            void loadMasking();
          }}
          options={[
            { value: "7", label: tr("7 days") },
            { value: "30", label: tr("30 days") },
          ]}
        />
      </View>
      <View class="flex-row items-end gap-6 px-[16px] pb-[14px] pt-3">
        <View class="gap-[2px]">
          <Text testID="masking-values" class="text-[30px] font-semibold text-foreground">
            {compact(sum()?.values ?? 0)}
          </Text>
          <Text class="text-[11.5px] text-muted-foreground">{trn(sum()?.values ?? 0, "value masked", "values masked")}</Text>
        </View>
        <View class="gap-[2px] pb-[2px]">
          <Text class="text-[15px] font-semibold text-foreground">{compact(sum()?.distinct ?? 0)}</Text>
          <Text class="text-[11.5px] text-muted-foreground">{tr("Different values")}</Text>
        </View>
        <View class="gap-[2px] pb-[2px]">
          <Text testID="masking-requests" class="text-[15px] font-semibold text-foreground">
            {compact(sum()?.requests ?? 0)}
          </Text>
          <Text class="text-[11.5px] text-muted-foreground">{tr("Requests")}</Text>
        </View>
        <View class="flex-1 gap-1">
          <View class="h-[44px] flex-row items-end gap-[3px]">
            <Each of={days()} key={(d) => d[0]}>
              {(d) => <View class={`flex-1 rounded-t-[2px] ${d()[1] ? "bg-primary" : "bg-pill"}`} style={{ height: `${d()[1] ? Math.max(8, Math.round((d()[1] / top()) * 100)) : 6}%` }} />}
            </Each>
          </View>
          <View class="flex-row">
            <Text class="flex-1 text-[10.5px] text-muted-foreground">{days()[0] ? dayLabel(days()[0]![0]) : ""}</Text>
            <Text class="text-[10.5px] text-muted-foreground">{tr("Today")}</Text>
          </View>
        </View>
      </View>
      <If when={agentsRows().length > 0}>
        <View class="flex-row items-center gap-3 border-t border-line-2 px-[16px] py-[12px]">
          <View class="flex-1 gap-[5px]">
            <Each of={agentsRows()} key={(a) => a[0]}>
              {(a) => (
                <View class="h-[22px] flex-row items-center gap-[7px] self-start rounded-[6px] bg-pill px-[7px]">
                  <Logo name={agentLogo(a()[0])} size={13} />
                  <Text class="text-[12px] text-foreground">{agentName(a()[0])}</Text>
                  <Text class="text-[11px] text-muted-foreground">{String(a()[1])}</Text>
                </View>
              )}
            </Each>
          </View>
          <View class="items-center gap-1">
            <View class="flex-row items-center gap-[6px]">
              <View class="h-px w-[38px] bg-border" />
              <View class="h-[30px] w-[30px] items-center justify-center rounded-full bg-accent">
                <Icon name="shield" color="primary" size={15} />
              </View>
              <View class="h-px w-[38px] bg-border" />
            </View>
            <Text class="text-[10.5px] text-muted-foreground">{tr("pansy masked these on this computer")}</Text>
          </View>
          <View class="flex-1 items-end gap-[5px]">
            <Each of={vendorRows()} key={(p) => p[0]}>
              {(p) => (
                <View class="h-[22px] flex-row items-center gap-[7px] rounded-[6px] bg-pill px-[7px]">
                  <Logo name={providerLogoOf(p()[0])} size={13} />
                  <Text class="text-[12px] text-foreground">{providerName(p()[0])}</Text>
                  <Text class="text-[11px] text-muted-foreground">{String(p()[1])}</Text>
                </View>
              )}
            </Each>
          </View>
        </View>
      </If>
    </List>
  );
}

/** Masking is off: what that means, and the switch. */
function Off() {
  return (
    <View testID="masking-empty" class="items-center gap-[10px] rounded-[12px] border border-border bg-card px-6 py-10">
      <View class="h-[40px] w-[40px] items-center justify-center rounded-full bg-pill">
        <Icon name="shield" color="muted-foreground" size={20} />
      </View>
      <Text class="text-[14px] font-semibold text-foreground">{tr("Requests go to vendors as they are")}</Text>
      <Text class="max-w-[420px] text-center text-[12px] text-muted-foreground">
        {tr("Turn on masking and API keys, tokens and passwords in what your agents send leave this computer as placeholders, and come back as they were in the answers.")}
      </Text>
      <View class="flex-row items-center gap-2 pt-1">
        <PrimaryButton label={tr("Turn on masking")} testID="masking-on" onPress={() => void setMasking({ secrets: true }).then(() => loadMasking())} />
        <TextButton
          label={tr("Settings › Privacy")}
          onPress={() => {
            settingsTab.set("privacy");
            go("settings");
          }}
        />
      </View>
    </View>
  );
}

function KindBadges(props: { values: MaskValue[] }) {
  const kinds = () => [...new Set(props.values.map((v) => v.k))].slice(0, 3);
  return (
    <View class="flex-row gap-1">
      <Each of={kinds()} key={(k) => k}>
        {(k) => <Badge label={kindLabel(k())} tone={props.values.some((v) => v.k === k() && v.a === "block") ? "danger" : props.values.some((v) => v.k === k() && v.a === "warn") ? "warning" : "default"} />}
      </Each>
    </View>
  );
}

function RecordRow(props: { record: MaskRecord; first: boolean }) {
  const r = () => props.record;
  return (
    <Row first={props.first} testID={`masking-record-${r().key}`} onPress={() => maskOpen.set(r().key)} class="min-h-[40px] py-[6px] hover:bg-pill/60">
      <Text class="w-[58px] text-[11.5px] text-muted-foreground">{timeOf(r().t)}</Text>
      <Logo name={agentLogo(r().agent ?? "")} size={14} />
      <Text class="text-[12px] text-fg-2">{agentName(r().agent ?? "other")}</Text>
      <Icon name="arrow-right" color="faint" size={11} />
      <Logo name={providerLogoOf(r().provider ?? "")} size={14} />
      <Text class="text-[12px] text-fg-2">{providerName(r().provider ?? "")}</Text>
      <Mono text={r().model ?? ""} class="min-w-0 flex-1 text-[11px] text-muted-foreground line-clamp-1" />
      <If when={r().blocked}>
        <Badge label={tr("Held back")} tone="danger" />
      </If>
      <KindBadges values={r().values} />
      <Text class="w-[36px] text-right text-[12px] font-medium text-foreground">{String(r().n)}</Text>
      <Icon name="chev-right" color="faint" size={11} />
    </Row>
  );
}

/** Each request, by day. */
function ByRequest(props: { records: MaskRecord[] }) {
  const days = () => {
    const out: { day: string; records: MaskRecord[] }[] = [];
    for (const r of props.records) {
      const day = dayOf(new Date(r.t));
      const last = out[out.length - 1];
      if (last?.day === day) last.records.push(r);
      else out.push({ day, records: [r] });
    }
    return out;
  };
  return (
    <Each of={days()} key={(d) => d.day}>
      {(d) => (
        <View class="gap-[6px]">
          <RowHead label={dayLabel(d().day)} note={trn(d().records.length, "%d request", "%d requests", d().records.length)} />
          <List>
            <Each of={d().records} key={(r) => String(r.key)}>
              {(r, i) => <RecordRow record={r()} first={i() === 0} />}
            </Each>
          </List>
        </View>
      )}
    </Each>
  );
}

/** Each value, however many requests had it. */
function ByValue(props: { records: MaskRecord[] }) {
  const values = () => {
    const by = new Map<string, { v: MaskValue; n: number; requests: number; last: string }>();
    for (const r of props.records) {
      for (const v of r.values) {
        const seen = by.get(v.p);
        if (seen) {
          seen.n += v.n;
          seen.requests += 1;
        } else by.set(v.p, { v, n: v.n, requests: 1, last: r.t });
      }
    }
    return [...by.values()].sort((a, b) => b.n - a.n);
  };
  return (
    <List>
      <Each of={values()} key={(x) => x.v.p}>
        {(x, i) => (
          <Row first={i() === 0} class="min-h-[40px] py-[6px]">
            <View class="w-[150px]">
              <Text class="text-[12px] text-foreground line-clamp-1">{kindLabel(x().v.k)}</Text>
            </View>
            <Mono text={shapeOf(x().v)} class="w-[130px] text-[11.5px] text-fg-2" />
            <Icon name="arrow-right" color="faint" size={11} />
            <Mono text={x().v.p} class="min-w-0 flex-1 text-[11.5px] text-muted-foreground line-clamp-1" />
            <Text class="text-[11.5px] text-muted-foreground">{trn(x().requests, "%d request", "%d requests", x().requests)}</Text>
            <Text class="w-[36px] text-right text-[12px] font-medium text-foreground">{String(x().n)}</Text>
          </Row>
        )}
      </Each>
    </List>
  );
}

function Records() {
  const view = signal<"request" | "value">("request");
  const filter = signal<Filter>("all");
  const records = () =>
    (masking()?.records ?? [])
      .map((r) => (filter() === "all" ? r : { ...r, values: r.values.filter((v) => groupOf(v) === filter()) }))
      .filter((r) => r.values.length > 0);
  /** The requests that had a value of `f`. */
  const count = (f: Filter) => (masking()?.records ?? []).filter((r) => f === "all" || r.values.some((v) => groupOf(v) === f)).length;
  const FILTERS = (): { value: Filter; label: string }[] => [
    { value: "all", label: tr("All") },
    { value: "secrets", label: tr("Secrets") },
    { value: "personal", label: tr("Personal data") },
    { value: "words", label: tr("Your words") },
    { value: "rules", label: tr("Your rules") },
  ];
  const clear = async () => {
    const yes = await ask({
      title: tr("Clear masking's records?"),
      description: tr("Every record of what masking kept from vendors goes, on this computer. Masking itself stays as it is."),
      confirm: tr("Clear records"),
      danger: true,
    });
    if (!yes) return;
    try {
      await Masking.clear();
      maskOpen.set(null);
      await loadMasking();
    } catch (error) {
      toast(reason(error), "error");
    }
  };
  return (
    <>
      <View class="h-[30px] flex-row items-center gap-2 pt-1">
        <Segs
          testID="masking-view"
          value={view()}
          onChange={(v) => view.set(v)}
          options={[
            { value: "request", label: tr("By request") },
            { value: "value", label: tr("By value") },
          ]}
        />
        <View class="flex-row items-center gap-[5px] pl-1">
          <View class="h-[6px] w-[6px] rounded-full bg-success" />
          <Text class="text-[11.5px] text-muted-foreground">{tr("Live")}</Text>
        </View>
        <View class="flex-1" />
        <Each of={FILTERS().filter((f) => f.value === "all" || count(f.value) > 0)} key={(f) => f.value}>
          {(f) => (
            <View
              role="button"
              testID={`masking-filter-${f().value}`}
              onPress={() => filter.set(f().value)}
              class={`h-[22px] flex-row items-center justify-center gap-[5px] rounded-full px-[9px] ${filter() === f().value ? "bg-accent" : "border border-border hover:bg-pill"}`}
            >
              <Text class={`text-[11.5px] ${filter() === f().value ? "font-medium text-primary" : "text-fg-2"}`}>{f().label}</Text>
              <Text class="text-[11px] text-muted-foreground">{String(count(f().value))}</Text>
            </View>
          )}
        </Each>
      </View>
      <If
        when={records().length > 0}
        else={
          <View class="items-center rounded-[12px] border border-dashed border-border px-6 py-8">
            <Text testID="masking-none" class="text-[12.5px] text-muted-foreground">
              {maskingOn() ? tr("Nothing masked in these days yet: requests with nothing to mask go as they are.") : tr("Nothing masked in these days.")}
            </Text>
          </View>
        }
      >
        <If when={view() === "request"} else={<ByValue records={records()} />}>
          <ByRequest records={records()} />
        </If>
      </If>
      <View class="flex-row items-center gap-2 px-1 pt-1">
        <Text class="flex-1 text-[11.5px] text-faint">{tr("Kept on this computer for 30 days, without the values themselves")}</Text>
        <If when={(masking()?.records.length ?? 0) > 0}>
          <TextButton label={tr("Clear records…")} tone="danger" testID="masking-clear" onPress={() => void clear()} />
        </If>
      </View>
    </>
  );
}

/** One step of a request's way: its number on a line, what happened, and its detail. */
function Step(props: { n: number; title: string; detail?: string; tone?: "on" | "warn" | "error" | "off"; last?: boolean }) {
  const look = () =>
    ({
      on: { box: "bg-accent", text: "text-primary" },
      warn: { box: "bg-warning-soft", text: "text-warning" },
      error: { box: "bg-destructive-soft", text: "text-destructive" },
      off: { box: "bg-pill", text: "text-muted-foreground" },
    })[props.tone ?? "on"];
  return (
    <View class="flex-row gap-[10px]">
      <View class="w-[20px] items-center">
        <View class={`h-[20px] w-[20px] items-center justify-center rounded-full ${look().box}`}>
          <Text class={`text-[10.5px] font-semibold ${look().text}`}>{String(props.n)}</Text>
        </View>
        <If when={!props.last}>
          <View class="w-px flex-1 bg-border" />
        </If>
      </View>
      <View class="flex-1 gap-[1px] pb-[12px] pt-[1px]">
        <Text class="text-[12.5px] font-medium text-foreground">{props.title}</Text>
        <If when={props.detail}>
          <Text class="text-[11.5px] text-muted-foreground">{props.detail ?? ""}</Text>
        </If>
      </View>
    </View>
  );
}

/** 171 → "171 B", 2048 → "2.0 KB". */
const bytes = (n: number) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`);
/** Microseconds as masking took them: "405 µs", "2.1 ms". */
const micros = (n: number) => (n < 1000 ? `${n} µs` : `${(n / 1000).toFixed(1)} ms`);

/** A request on its own: its steps, its text before and after, and each value. */
function Detail() {
  const all = () => masking()?.records ?? [];
  const at = () => all().findIndex((r) => r.key === maskOpen());
  const r = () => all()[at()];
  const changes = signal<MaskChange[] | null | "gone">(null);
  const show = signal<"before" | "after" | "both">("both");
  let asked = -1;
  const load = (key: number) => {
    if (asked === key) return;
    asked = key;
    changes.set(null);
    Masking.changes(String(key))
      .then((json) => {
        if (asked === key) changes.set((JSON.parse(json) as MaskChange[] | null) ?? "gone");
      })
      .catch(() => changes.set("gone"));
  };
  effect(() => {
    const key = maskOpen();
    if (key !== null) load(key);
  });
  const rec = () => r()!;
  const back = (v: MaskValue) => r()?.back?.[v.p] ?? 0;
  const putBack = () => Object.values(r()?.back ?? {}).reduce((n, x) => n + x, 0);
  const warned = () => rec().values.some((v) => v.a === "warn");
  const finders = () => [...new Set(rec().values.map((v) => foundBy(v.b)))].join(", ");
  /** How many values a change had, by the placeholders in what the vendor saw. */
  const valuesIn = (c: MaskChange) => rec().values.filter((v) => c.after.includes(v.p) || (v.w ?? []).includes(c.path)).length;
  return (
    <If when={r()}>
      <>
      <List class="px-[16px] pb-[4px] pt-[14px]">
        <View class="flex-row items-center gap-2 pb-3">
          <Logo name={agentLogo(rec().agent ?? "")} size={16} />
          <Text class="text-[13px] font-semibold text-foreground">{agentName(rec().agent ?? "other")}</Text>
          <Icon name="arrow-right" color="faint" size={11} />
          <Logo name={providerLogoOf(rec().provider ?? "")} size={16} />
          <Text class="text-[13px] font-semibold text-foreground">{providerName(rec().provider ?? "")}</Text>
          <Mono text={rec().model ?? ""} class="flex-1 text-[11.5px] text-muted-foreground line-clamp-1" />
          <IconButton icon="chev-up" label={tr("Newer request")} disabled={at() <= 0} onPress={() => maskOpen.set(all()[at() - 1]!.key)} testID="masking-newer" />
          <IconButton icon="chev-down" label={tr("Older request")} disabled={at() >= all().length - 1} onPress={() => maskOpen.set(all()[at() + 1]!.key)} testID="masking-older" />
        </View>
        <Step n={1} title={tr("Received")} detail={[tr("From %s", agentName(rec().agent ?? "other")), ...(rec().size ? [bytes(rec().size!)] : [])].join(" · ")} />
        <Step n={2} title={trn(rec().values.length, "Found %d value", "Found %d values", rec().values.length)} detail={`${finders()} · ${micros(rec().took ?? 0)}`} />
        <If
          when={!rec().blocked}
          else={<Step n={3} title={tr("Held back")} detail={tr("A value set to Block was in it: the agent was told, and nothing was sent")} tone="error" last />}
        >
          <Step n={3} title={trn(maskedIn(rec()), "Replaced %d place", "Replaced %d places", maskedIn(rec()))} detail={warned() ? tr("Some went as written: set to Warn") : tr("With placeholders")} tone={warned() ? "warn" : "on"} />
          <Step n={4} title={tr("Sent")} detail={`${providerName(rec().provider ?? "")} · ${rec().model ?? ""}`} />
          <If
            when={rec().back}
            else={<Step n={5} title={tr("Put back")} detail={tr("The answer's count is kept while pansy runs")} tone="off" last />}
          >
            <If when={putBack() > 0} else={<Step n={5} title={tr("Nothing to put back")} detail={tr("The answer named no placeholder")} tone="off" last />}>
              <Step n={5} title={trn(putBack(), "Put back %d place", "Put back %d places", putBack())} detail={tr("In the answer, as they were")} last />
            </If>
          </If>
        </If>
      </List>
      <RowHead label={tr("Request content")} note={tr("Only the parts masking changed")}>
        <Segs
          small
          testID="masking-show"
          value={show()}
          onChange={(v) => show.set(v)}
          options={[
            { value: "before", label: tr("Original") },
            { value: "after", label: tr("What the vendor saw") },
            { value: "both", label: tr("Side by side") },
          ]}
        />
      </RowHead>
      <If
        when={Array.isArray(changes())}
        else={
          <View class="rounded-[12px] border border-dashed border-border px-4 py-5">
            <Text class="text-[12px] text-muted-foreground">
              {changes() === "gone" ? tr("The text is kept in memory for the latest 100 requests only: pansy restarted, or newer requests took this one's place.") : tr("Loading…")}
            </Text>
          </View>
        }
      >
        <List testID="masking-changes">
          <If when={show() === "both"}>
            <View class="flex-row gap-3 border-b border-line-2 bg-card-2 px-[14px] py-[6px]">
              <Text class="flex-1 text-[11px] font-medium text-muted-foreground">{tr("As written here")}</Text>
              <View class="w-px" />
              <Text class="flex-1 text-[11px] font-medium text-muted-foreground">{tr("What the vendor saw")}</Text>
            </View>
          </If>
          <Each of={(changes() as MaskChange[]) ?? []} key={(c) => c.path}>
            {(c, i) => (
              <View class={`gap-[6px] px-[14px] py-[10px] ${i() === 0 ? "" : "border-t border-line-2"}`}>
                <View class="flex-row items-center gap-2">
                  <Mono text={c().path} class="flex-1 text-[10.5px] text-faint" />
                  <Text class="text-[10.5px] text-muted-foreground">{trn(valuesIn(c()), "%d value", "%d values", valuesIn(c()))}</Text>
                </View>
                <View class="flex-row gap-3">
                  <If when={show() !== "after"}>
                    <Text class="flex-1 font-mono text-[11.5px] text-fg-2">{c().before}</Text>
                  </If>
                  <If when={show() === "both"}>
                    <View class="w-px bg-line-2" />
                  </If>
                  <If when={show() !== "before"}>
                    <Text class="flex-1 font-mono text-[11.5px] text-foreground">{c().after}</Text>
                  </If>
                </View>
              </View>
            )}
          </Each>
        </List>
      </If>
      <RowHead
        label={tr("How it was masked")}
        note={`${trn(rec().values.length, "%d value", "%d values", rec().values.length)} · ${trn(maskedIn(rec()), "%d place", "%d places", maskedIn(rec()))}`}
        class="pt-1"
      />
      <List testID="masking-values-list">
        <Each of={rec().values} key={(v) => v.p}>
          {(v, i) => (
            <View class={`gap-[6px] px-[14px] py-[10px] ${i() === 0 ? "" : "border-t border-line-2"}`}>
              <View class="flex-row items-center gap-[10px]">
                <View class="w-[150px] flex-row items-center gap-[6px]">
                  <Text class="text-[12.5px] font-medium text-foreground">{kindLabel(v().k)}</Text>
                  <Text class="text-[11px] text-muted-foreground">{`×${v().n}`}</Text>
                </View>
                <Mono text={shapeOf(v())} class="w-[120px] text-[11.5px] text-fg-2" />
                <Icon name="arrow-right" color="faint" size={11} />
                <Mono text={v().a === "warn" ? tr("sent as written") : v().p} class="min-w-0 flex-1 text-[11.5px] line-clamp-1" />
                <If when={v().a}>
                  <Badge label={v().a === "block" ? tr("Block") : tr("Warn")} tone={v().a === "block" ? "danger" : "warning"} />
                </If>
              </View>
              <View class="flex-row gap-5 pl-[160px]">
                <Fact label={tr("Found by")} value={foundBy(v().b)} />
                <Fact label={tr("Where")} value={(v().w ?? []).join(", ") || "—"} mono />
                <Fact label={tr("In the answer")} value={!rec().back ? "—" : back(v()) ? trn(back(v()), "Put back %d time", "Put back %d times", back(v())) : tr("Not named")} />
              </View>
            </View>
          )}
        </Each>
      </List>
      <Text class="px-1 text-[11.5px] text-faint">
        {tr("A value itself is in this computer's memory only, and gone once pansy restarts. The record on disk keeps its kind, length and first characters.")}
      </Text>
      </>
    </If>
  );
}

/** A label over a value: Found by / Rules. */
function Fact(props: { label: string; value: string; mono?: boolean }) {
  return (
    <View class="gap-[1px]">
      <Text class="text-[10.5px] text-faint">{props.label}</Text>
      <Text class={`text-[11.5px] text-fg-2 ${props.mono ? "font-mono" : ""}`}>{props.value}</Text>
    </View>
  );
}

export function MaskingPage() {
  const any = () => (masking()?.records.length ?? 0) > 0;
  return (
    <Page testID="page-masking">
      <If
        when={maskOpen() === null}
        else={<Detail />}
      >
        <If when={maskingOn() || any()} else={settings() ? <Off /> : undefined}>
          <Summary />
          <Records />
        </If>
      </If>
    </Page>
  );
}

