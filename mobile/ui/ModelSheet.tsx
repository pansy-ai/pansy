// 模型 (phone-ui.md "4. 模型"): on top, the thread's permission mode and the
// effort where the agent takes one, in view as the sheet opens; under them
// pansy's routing groups and each provider's models (models.list),
// searchable, the chosen one ticked. A change counts from the next turn.
import { effect, resource, signal, Text, View } from "nsh";
import type { Mode, Models } from "../net/types";
import { conn } from "../state/store";
import { tr } from "./i18n";
import { modeNote, modesOf } from "./ModeSheet";
import { agentLogo, agentName, BottomSheet, Field, Group, GroupHead, GroupRow, Icon, Loading, Logo, providerLogo, Seg } from "./parts";

export const EFFORTS = ["low", "medium", "high", "xhigh"] as const;
export type Effort = (typeof EFFORTS)[number];

/** Whether `agent` takes an effort: Codex's and Claude Code's turns do. */
export const takesEffort = (agent: string) => agent === "codex" || agent === "claude-code";

const effortLabel = (e: Effort) => ({ low: tr("Low"), medium: tr("Medium"), high: tr("High"), xhigh: tr("Extra high") })[e];
const modeShort = (m: Mode) => ({ ask: tr("Ask"), edits: tr("Accept edits"), auto: tr("Auto"), full: tr("Full access") })[m];
const routingLabel = (r: string) => ({ order: tr("in order"), ordered: tr("in order"), rotate: tr("rotating"), roundRobin: tr("rotating"), least: tr("least used"), leastUsed: tr("least used") })[r] ?? r;

/** A context window as `128K`, `1M`. */
export const contextSize = (n?: number) => (!n ? "" : n >= 1_000_000 ? `${Math.round(n / 100_000) / 10}M` : `${Math.round(n / 1000)}K`);

/** The model's short name, for the composer's pill: `v4` of `deepseek/v4`, `daily` of `group/daily`. */
export const shortModel = (id: string) => id.split("/").pop() || id;

export function ModelSheet(props: {
  open: boolean;
  onClose: () => void;
  machine: string;
  agent: string;
  model: string;
  effort?: string;
  onPick: (model: string, effort?: string) => void;
  /** The thread's: "only this thread"; a new task's has none. */
  thread?: boolean;
  /** The thread's permission mode, chosen here too; without `onMode`, none. */
  mode?: Mode;
  onMode?: (m: Mode) => void;
}) {
  const q = signal("");
  const effort = signal<Effort>("medium");
  effect(() => {
    if (props.open) effort.set((EFFORTS as readonly string[]).includes(props.effort ?? "") ? (props.effort as Effort) : "medium");
  });
  const models = resource(
    () => (props.open ? `${props.machine}:${props.agent}` : null),
    async () => (await conn(props.machine)?.call<Models>("models.list", { agent: props.agent })) ?? { groups: [], providers: [] },
  );
  const match = (s: string) => !q().trim() || s.toLowerCase().includes(q().trim().toLowerCase());
  const pick = (id: string) => {
    props.onPick(id, takesEffort(props.agent) ? effort() : undefined);
    props.onClose();
  };
  const tick = (id: string) => (props.model === id ? <Icon name="check" color="primary" size={18} /> : undefined);
  return (
    <BottomSheet
      open={props.open}
      onClose={props.onClose}
      title={tr("Model")}
      subtitle={props.thread ? tr("%s · only this thread", agentName(props.agent)) : agentName(props.agent)}
      start={0.75}
      testID="model-sheet"
      left={<Logo name={agentLogo(props.agent)} size={20} />}
    >
      <View class="gap-[14px] px-5 pb-10 pt-[6px]">
        <If when={props.onMode}>
          <GroupHead text={tr("Permissions")} />
          <Seg testID="permission" value={props.mode ?? "full"} onChange={(m) => props.onMode?.(m)} options={modesOf(props.agent).map((m) => ({ value: m, label: modeShort(m) }))} />
          <Text class="-mt-[6px] px-4 text-[13px] leading-[18px] text-fg-2">{modeNote(props.mode ?? "full")}</Text>
        </If>
        <If when={takesEffort(props.agent)}>
          <GroupHead text={tr("Effort")} />
          <Seg
            testID="effort"
            value={effort()}
            onChange={(e) => {
              effort.set(e);
              props.onPick(props.model, e);
            }}
            options={EFFORTS.map((e) => ({ value: e, label: effortLabel(e) }))}
          />
        </If>
        <Field value={q()} onChange={q.set} placeholder={tr("Search models")} testID="model-search" />
        <If when={!models.loading() || models()} else={<Loading text={tr("Loading models…")} />}>
          <If when={(models()?.groups ?? []).some((g) => match(g.id))}>
            <GroupHead text={tr("Routing groups")} />
            <Group>
              <Each of={(models()?.groups ?? []).filter((g) => match(g.id))} key={(g) => g.id}>
                {(g, i) => (
                  <GroupRow
                    first={i() === 0}
                    icon="route"
                    title={g().id}
                    sub={tr("%d models · %s", g().members, routingLabel(g().routing))}
                    testID={`model-${g().id}`}
                    onPress={() => pick(g().id)}
                    trailing={tick(g().id)}
                  />
                )}
              </Each>
            </Group>
          </If>
          <Each of={(models()?.providers ?? []).filter((p) => p.models.some((m) => match(m.id)))} key={(p) => p.id}>
            {(p) => (
              <>
                <GroupHead text={p().name} />
                <Group>
                  <Each of={p().models.filter((m) => match(m.id))} key={(m) => m.id}>
                    {(m, i) => (
                      <GroupRow
                        first={i() === 0}
                        lead={<Logo name={providerLogo(p().id) ?? providerLogo(m().id)} size={22} />}
                        title={m().id}
                        sub={[contextSize(m().context), m().reasoning ? tr("reasoning") : "", m().vision ? tr("images") : ""].filter(Boolean).join(" · ") || undefined}
                        testID={`model-${m().id}`}
                        onPress={() => pick(m().id)}
                        trailing={tick(m().id)}
                      />
                    )}
                  </Each>
                </Group>
              </>
            )}
          </Each>
          <Text class="text-center text-[13px] text-fg-2">{tr("Takes effect from the next turn")}</Text>
        </If>
      </View>
    </BottomSheet>
  );
}
