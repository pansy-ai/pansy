// What connecting an agent will write, before it is written: its files,
// each key, what is there now and what would be. Asked from the row of an
// agent not connected yet; nothing is changed by looking.
import { keymap, signal, Text, View } from "nsh";
import { Agents } from "pansy/native";
import { ChangesList } from "./AgentModals";
import { connect } from "./agentState";
import { tr } from "./i18n";
import { choiceName, choices, question, reason, type Agent, type FileChange } from "./model";
import { agentLogo, Logo, Modal, PrimaryButton, TextButton } from "./parts";

/** The agent looked at, the model it would be connected with, and what that writes (`null`: being read). */
export const previewing = signal<{ agent: Agent; model: string; changes: FileChange[] | null; error: string } | null>(null);

/** The model a look is taken with: the one the agent was last on, else the first there is. */
const modelFor = (a: Agent) => a.recent.find((m) => choices().some((c) => c.id === m)) ?? choices()[0]?.id ?? "";

export function openPreview(agent: Agent) {
  const model = modelFor(agent);
  if (!model) return;
  previewing.set({ agent, model, changes: null, error: "" });
  void Agents.preview(agent.id, model)
    .then((json) => previewing.update((p) => (p && p.agent.id === agent.id ? { ...p, changes: JSON.parse(json) as FileChange[] } : p)))
    .catch((error) => previewing.update((p) => (p && p.agent.id === agent.id ? { ...p, changes: [], error: reason(error) } : p)));
}

/** Above the window, beside the other modals. */
export function PreviewDialog() {
  const p = () => previewing();
  const close = () => previewing.set(null);
  keymap.bind("escape", () => {
    if (question()) return;
    if (previewing()) close();
  });
  return (
    <If when={p()}>
      <Modal
        testID="preview"
        width={560}
        icon={<Logo name={agentLogo(p()?.agent.id ?? "")} size={18} />}
        title={tr("What connecting %s changes", p()?.agent.name ?? "")}
        onClose={close}
        foot={
          <>
            <View class="flex-1" />
            <TextButton label={tr("Close")} testID="preview-close" onPress={close} />
            <PrimaryButton
              label={tr("Connect")}
              testID="preview-connect"
              onPress={() => {
                const now = p();
                close();
                if (now) connect(now.agent, now.model);
              }}
            />
          </>
        }
      >
        <Text testID="preview-said" class="text-[12.5px] leading-[19px] text-fg-2">
          {tr("Connected with %s, pansy sets these keys in %s's own files and nothing else. Disconnecting puts back exactly these. Nothing is written until you connect.", choiceName(p()?.model ?? ""), p()?.agent.name ?? "")}
        </Text>
        <If when={p()?.changes !== null} else={<Text class="py-[14px] text-[12px] text-muted-foreground">{tr("Reading what it would write…")}</Text>}>
          <If when={!p()?.error} else={<Text class="text-[12.5px] text-destructive">{p()?.error ?? ""}</Text>}>
            <ChangesList changes={p()?.changes ?? []} />
          </If>
        </If>
      </Modal>
    </If>
  );
}
