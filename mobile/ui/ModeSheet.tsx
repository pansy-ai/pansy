// 线程设置 (phone-ui.md "5. 线程设置"): the permission mode, four options as
// T3 names them (only those the agent can keep), plan mode (phase 2), and
// where the thread works.
import { Text, View } from "nsh";
import type { Mode } from "../net/types";
import { tr } from "./i18n";
import { BottomSheet, Group, GroupHead, GroupRow, Toggle } from "./parts";
import { modeLabel } from "./Composer";

/** The modes `agent` can keep: Codex and Claude Code all four, the others asking or not. */
export const modesOf = (agent: string): Mode[] => (agent === "codex" || agent === "claude-code" ? ["ask", "edits", "auto", "full"] : ["ask", "full"]);

const modeNote = (m: Mode) =>
  ({
    ask: tr("Commands and file changes wait for your approval."),
    edits: tr("File changes go through; anything else waits for you."),
    auto: tr("The agent's own review decides; routine work goes through."),
    full: tr("Nothing asks. The agent runs commands and changes files."),
  })[m];

export function ModeSheet(props: { open: boolean; onClose: () => void; agent: string; mode: Mode; onMode: (m: Mode) => void; workspace?: string }) {
  return (
    <BottomSheet open={props.open} onClose={props.onClose} title={tr("Thread settings")} start={0.6} testID="mode-sheet">
      <View class="gap-[14px] px-5 pb-10 pt-[6px]">
        <GroupHead text={tr("Permissions")} />
        <View class="gap-2">
          <Each of={modesOf(props.agent)} key={(m) => m}>
            {(m) => {
              const on = () => props.mode === m();
              return (
                <View
                  role="radio"
                  accessibilityState={{ checked: on() }}
                  testID={`mode-${m()}`}
                  onPress={() => {
                    props.onMode(m());
                    props.onClose();
                  }}
                  class={`min-h-12 gap-[2px] rounded-2xl border px-[14px] py-3 ${on() ? "border-primary bg-accent" : "border-border bg-grouped"}`}
                >
                  <Text class="text-[14px] font-bold text-fg">{modeLabel(m())}</Text>
                  <Text class="text-[14px] text-fg-2">{modeNote(m())}</Text>
                </View>
              );
            }}
          </Each>
        </View>
        <Group>
          <GroupRow first title={tr("Plan mode")} sub={tr("Phase 2")} trailing={<Toggle on={false} disabled onChange={() => {}} label={tr("Plan mode")} />} />
        </Group>
        <If when={props.workspace}>
          <GroupHead text={tr("Workspace")} />
          <Group>
            <GroupRow first title={props.workspace ?? ""} mono />
          </Group>
        </If>
      </View>
    </BottomSheet>
  );
}
