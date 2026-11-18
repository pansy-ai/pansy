// Settings › Diagnostics: whether the agents can reach their models through
// the gateway, each problem with what fixes it; and the way back when pansy
// can't serve them for a while: every agent's own config put back for now,
// its model remembered to connect it again.
import { Clipboard, signal, Text, View } from "nsh";
import { Agents, Diagnostics, Gateway, Settings } from "pansy/native";
import { tr, trn } from "./i18n";
import { act, agents, ask, choiceName, choices, go, isBusy, loadAgents, loadGateway, loadProviders, reason, settingsQuery, settingsTab, toast } from "./model";
import { FieldButton, Icon, TextButton, type Color, type IconName } from "./parts";
import { ChoiceLogo, ModelPicker } from "./Picker";

/** One finding, as `pansy_agents::doctor::Finding` serializes it. */
export interface Finding {
  /** `gateway`, `providers`, `models`, `agents`, `files`, or an agent's id. */
  about: string;
  level: "ok" | "note" | "warn" | "fail";
  code: string;
  /** An agent's name first, then its model. */
  args: string[];
}

/** The latest checks; null while the first is running. */
export const findings = signal<Finding[] | null>(null);

export async function checkAgain() {
  try {
    findings.set(JSON.parse(await Diagnostics.run()) as Finding[]);
  } catch (error) {
    toast(reason(error), "error");
  }
}

/** After a change: the checks again, and what the other pages show of it. */
function refresh() {
  return Promise.all([checkAgain(), loadAgents(), loadGateway(), loadProviders()]);
}

/** A finding in the window's language. */
export function said(f: Finding): string {
  const [a = "", b = ""] = f.args;
  switch (f.code) {
    case "config-unreadable":
      return tr("pansy's files can't be read: %s", a);
    case "gateway-running":
      return tr("The gateway answers at %s", a);
    case "gateway-port-taken":
      return tr("Another program has port %s: give pansy another one", a);
    case "gateway-stopped":
      return tr("Nothing answers at %s: agents connected to pansy can't reach their models", a);
    case "gateway-failed":
      return tr("The gateway could not start: %s", a);
    case "providers-none":
      return tr("No provider yet: agents have no model to use");
    case "providers-off":
      return tr("Every provider is switched off");
    case "providers-on":
      return trn(Number(a), "%d provider on", "%d providers on", Number(a));
    case "models-none":
      return tr("The providers serve no model yet");
    case "models":
      return trn(Number(a), "%d model to pick from", "%d models to pick from", Number(a));
    case "agent-connected":
      return tr("%s is on %s", a, b);
    case "agent-model-gone":
      return tr("%s is on %s, which no provider or group serves now", a, b);
    case "agent-subagent-gone":
      return tr("%s's subagents start on %s, which no provider or group serves now", a, b);
    case "agent-cli-missing":
      return tr("%s's settings are here, its program isn't", a);
    case "agent-paused":
      return tr("%s uses its own config for now; Connect again puts it back on %s", a, b);
    case "agent-drifted":
      return tr("%s's config no longer points at this gateway", a);
    case "agents-none":
      return tr("No agent is connected to pansy: a test request goes the way an agent's would");
    case "files-open":
      return tr("Others on this computer can read %s", a);
  }
  return f.code;
}

const LOOK: Record<Finding["level"], { icon: IconName; color: Color; mark: string }> = {
  ok: { icon: "circle-check", color: "success", mark: "✓" },
  note: { icon: "circle-alert", color: "muted-foreground", mark: "·" },
  warn: { icon: "triangle-alert", color: "warning", mark: "!" },
  fail: { icon: "circle-x", color: "destructive", mark: "✗" },
};

const problems = () => (findings() ?? []).filter((f) => f.level === "warn" || f.level === "fail").length;

/** The line under Health check. */
export function healthSaid(): string {
  if (!findings()) return tr("Checking…");
  const n = problems();
  return n === 0 ? tr("Everything the agents need works") : trn(n, "%d problem found", "%d problems found", n);
}

/** What fixes a finding, where the window can. */
function fix(f: Finding): { label: string; run: () => Promise<unknown> } | null {
  switch (f.code) {
    case "gateway-stopped":
    case "gateway-failed":
      return { label: tr("Restart gateway"), run: () => Gateway.restart() };
    case "gateway-port-taken":
      return {
        label: tr("Change the port"),
        run: async () => {
          settingsQuery.set("");
          settingsTab.set("network");
        },
      };
    case "providers-none":
    case "providers-off":
    case "models-none":
      return { label: tr("Open Providers"), run: async () => go("providers") };
    case "agent-model-gone":
    case "agent-subagent-gone":
      return { label: tr("Pick a model"), run: async () => go("agents") };
    case "agent-connected":
      return { label: tr("Restore for now"), run: () => Agents.pause(f.about) };
    case "agent-paused":
      return { label: tr("Connect again"), run: () => Agents.resume(f.about) };
    case "agent-drifted":
      return { label: tr("Connect again"), run: () => Agents.connect(f.about, f.args[1] ?? "") };
    case "config-unreadable":
      return { label: tr("Show"), run: () => Settings.openHome() };
    case "agents-none":
      return choices().length ? { label: tr("Send a test request"), run: sendTest } : null;
  }
  return null;
}

// --- A test request -------------------------------------------------------------

/** A test request's outcome, as `pansy_agents::doctor::Sent` serializes it. */
interface Sent {
  model: string;
  ok: boolean;
  /** 0: nothing answered at `url` (`said` empty), or nothing came back in time. */
  status: number;
  url: string;
  ms: number;
  said: string;
}

/** The model a test request goes to; empty: the first one. */
const testModel = signal("");
const sent = signal<Sent | null>(null);
const sending = signal(false);

async function sendTest() {
  sending.set(true);
  try {
    sent.set(JSON.parse(await Diagnostics.send(testModel())) as Sent);
  } catch (error) {
    toast(reason(error), "error");
  } finally {
    sending.set(false);
  }
}

function sentSaid(s: Sent): string {
  const secs = (s.ms / 1000).toFixed(1);
  if (s.ok) return s.said ? tr("%s answered in %s s: %s", s.model, secs, s.said) : tr("%s answered in %s s, with no words", s.model, secs);
  if (s.status === 0 && !s.said) return tr("Nothing answers at %s: agents connected to pansy can't reach their models", s.url);
  if (s.status === 0) return tr("%s did not answer in time", s.model);
  return tr("%s failed (%d) in %s s: %s", s.model, s.status, secs, s.said);
}

/** The model to send to, and Send. */
export function TestControl() {
  const open = signal(false);
  const model = () => testModel() || choices()[0]?.id || "";
  return (
    <View class="flex-row items-center gap-2">
      <ModelPicker
        open={open()}
        onOpenChange={(o) => open.set(o)}
        current={model() || null}
        onPick={(id) => testModel.set(id)}
        trigger={<FieldButton testID="test-model" open={open()} empty={!model()} width={200} value={model() ? choiceName(model()) : tr("No model yet")} icon={model() ? <ChoiceLogo id={model()} size={14} /> : undefined} />}
      />
      <TextButton label={sending() ? tr("Sending…") : tr("Send")} icon="arrow-right" disabled={sending() || !model()} testID="test-send" onPress={() => void sendTest()} />
    </View>
  );
}

/** What the last test request came back with. */
export function TestResult() {
  return (
    <If when={sent()}>
      <View testID="test-result" class="flex-row items-center gap-2 rounded-[8px] border border-line-2 py-[6px] pl-[9px] pr-[6px]">
        <Icon name={sent()?.ok ? "circle-check" : "circle-x"} color={sent()?.ok ? "success" : "destructive"} size={13} />
        <Text class="flex-1 text-[12px] text-fg-2">{sent() ? sentSaid(sent()!) : ""}</Text>
      </View>
    </If>
  );
}

/** The line under Send a test request. */
export function testSaid(): string {
  return tr("One short message through the gateway, the way an agent's request goes: whether the model answers, and how fast. It uses a few tokens");
}

function FindingRow(props: { finding: Finding; first: boolean }) {
  const f = () => props.finding;
  const look = () => LOOK[f().level];
  const key = () => `diagnose:${f().about}:${f().code}`;
  return (
    <View testID={`diagnosis-${f().about}-${f().code}`} class={`min-h-[32px] flex-row items-center gap-2 py-[5px] pl-[9px] pr-[4px] ${props.first ? "" : "border-t border-line-2"}`}>
      <Icon name={look().icon} color={look().color} size={13} />
      <Text class="flex-1 text-[12px] text-fg-2">{said(f())}</Text>
      <If when={fix(f())}>
        <TextButton
          label={fix(f())?.label ?? ""}
          disabled={isBusy(key())}
          testID={`fix-${f().about}-${f().code}`}
          onPress={() => void act(key(), () => fix(f())!.run()).then(refresh)}
        />
      </If>
    </View>
  );
}

/** Every finding, checked again each time the page opens. */
export function FindingList() {
  void refresh();
  return (
    <If when={findings()}>
      <View testID="diagnosis" class="overflow-hidden rounded-[8px] border border-line-2">
        <Each of={findings() ?? []} key={(f) => `${f.about}:${f.code}`}>
          {(f, i) => <FindingRow finding={f()} first={i() === 0} />}
        </Each>
      </View>
    </If>
  );
}

/** The findings as text, to paste into a bug report: no key, no prompt. */
function report(): string {
  return [`pansy ${Settings.version()}`, ...(findings() ?? []).map((f) => `${LOOK[f.level].mark} ${said(f)}`)].join("\n");
}

export function HealthControl() {
  const running = signal(false);
  return (
    <View class="flex-row items-center gap-1">
      <TextButton label={tr("Copy report")} tone="muted" disabled={!findings()} testID="diagnosis-copy" onPress={() => void Clipboard.writeText(report()).then(() => toast(tr("Copied")))} />
      <TextButton
        label={tr("Check again")}
        icon="refresh-cw"
        disabled={running()}
        testID="diagnosis-run"
        onPress={() => {
          running.set(true);
          void refresh().finally(() => running.set(false));
        }}
      />
    </View>
  );
}

const connected = () => agents().filter((a) => a.connected).length;
const paused = () => agents().filter((a) => a.paused).length;

export function RestoreAll() {
  return (
    <TextButton
      label={tr("Restore all")}
      disabled={connected() === 0 || isBusy("diagnose:pause-all")}
      testID="diagnosis-pause-all"
      onPress={async () => {
        const yes = await ask({
          title: tr("Restore every agent's own config for now?"),
          description: tr("Each agent connected to pansy goes back to the config it had before, and works without the gateway. Connect all again puts them back on pansy with the same models."),
          confirm: tr("Restore all"),
        });
        if (!yes) return;
        let n = 0;
        await act("diagnose:pause-all", async () => {
          n = await Agents.pauseAll();
        }).then((ok) => ok && toast(trn(n, "%d agent uses its own config for now", "%d agents use their own config for now", n)));
        await refresh();
      }}
    />
  );
}

export function ResumeAll() {
  return (
    <TextButton
      label={tr("Connect all again")}
      disabled={paused() === 0 || isBusy("diagnose:resume-all")}
      testID="diagnosis-resume-all"
      onPress={async () => {
        let n = 0;
        await act("diagnose:resume-all", async () => {
          n = await Agents.resumeAll();
        }).then((ok) => ok && toast(trn(n, "%d agent is back on pansy", "%d agents are back on pansy", n)));
        await refresh();
      }}
    />
  );
}

/** The commands that do the same without the window. */
export function FromATerminal() {
  return (
    <View class="gap-[6px]">
      <Each of={["pansy doctor", "pansy test", "pansy pause all", "pansy resume all"]} key={(c) => c}>
        {(c) => (
          <View class="h-[26px] flex-row items-center gap-2 rounded-[6px] bg-card-2 pl-[9px] pr-[3px]">
            <Text class="flex-1 font-mono text-[11.5px] text-foreground">{c()}</Text>
            <Text class="text-[11px] text-muted-foreground">
              {({ "pansy doctor": tr("checks the same"), "pansy test": tr("sends a test request"), "pansy pause all": tr("restores every agent for now") } as Record<string, string>)[c()] ?? tr("connects them again")}
            </Text>
            <TextButton label={tr("Copy")} tone="muted" onPress={() => void Clipboard.writeText(c()).then(() => toast(tr("Copied")))} />
          </View>
        )}
      </Each>
    </View>
  );
}
