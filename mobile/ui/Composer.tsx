// 输入框 (phone-ui.md "3. 输入框"): a glass capsule floating over the feed:
// attachments (phase 2), the text, the model's pill, send. While a turn
// runs, send steers it, or queues after it (a long press switches), and a
// red stop sits beside it. Under it, the permission mode and plan mode.
import { signal, Text, View, type JSX } from "nsh";
import { Button as BaseButton, Input } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Mode } from "../net/types";
import { tr } from "./i18n";
import { agentLogo, BLUR, Chip, Icon, Logo, MOTION, SHADOW, toast, type IconName } from "./parts";
import { shortModel } from "./ModelSheet";

export type SendMode = "steer" | "queue";

export const modeLabel = (m: Mode) => ({ ask: tr("Ask every time"), edits: tr("Auto-accept edits"), auto: tr("Auto"), full: tr("Full access") })[m];

export function Composer(props: {
  text: string;
  onText: (t: string) => void;
  placeholder: string;
  agent: string;
  model: string;
  running?: boolean;
  onSend: (mode: SendMode | "start") => void;
  /** Stop the turn; pressed again within a few seconds, `force`: end the agent. */
  onStop?: (force: boolean) => void;
  onModel: () => void;
  accessory?: JSX.Element;
  disabled?: boolean;
}) {
  const sendMode = signal<SendMode>("steer");
  const stopping = signal(false);
  const empty = () => !props.text.trim();
  const icon = (): IconName => (!props.running ? "arrow-up" : sendMode() === "steer" ? "corner-left-up" : "list-ordered");
  const label = () => (!props.running ? tr("Send") : sendMode() === "steer" ? tr("Steer") : tr("Queue"));
  const send = () => {
    if (empty() || props.disabled) return;
    props.onSend(props.running ? sendMode() : "start");
  };
  return (
    <View class="gap-[10px]">
      <View
        class="min-h-14 flex-row items-end gap-[2px] rounded-[27px] border border-border bg-composer p-[6px]"
        style={{ ...BLUR, boxShadow: SHADOW.composer() }}
      >
        <View role="button" accessibilityLabel={tr("Attach")} onPress={() => toast(tr("Attachments come in phase 2"))} class="h-11 w-11 items-center justify-center active:opacity-50">
          <Icon name="plus" color="fg-2" size={22} />
        </View>
        <Input
          value={props.text}
          onChange={({ value }) => props.onText(value)}
          placeholder={props.placeholder}
          accessibilityLabel={tr("Message")}
          testID="composer-input"
          multiline
          submitBehavior="newline"
          ios={{ inputAccessoryViewID: "composer-accessory" }}
          class="max-h-[160px] min-h-11 flex-1 px-1 py-[10px] text-[16px] leading-[22px] text-fg placeholder:text-fg-3"
        />
        <View class="h-11 flex-row items-center gap-[2px]">
          <View
            role="button"
            testID="model-pill"
            accessibilityLabel={tr("Model: %s", props.model)}
            onPress={props.onModel}
            class="h-8 flex-row items-center gap-[5px] rounded-full border border-border bg-card px-[10px] active:opacity-70"
          >
            <Logo name={agentLogo(props.agent)} size={14} />
            <Text class="text-[13px] font-medium text-fg line-clamp-1" style={{ maxWidth: 90 }}>
              {shortModel(props.model)}
            </Text>
            <Icon name="chevron-down" color="fg-2" size={12} />
          </View>
          <View
            role="button"
            testID="send"
            accessibilityLabel={label()}
            accessibilityHint={props.running ? tr("Hold to switch between steer and queue") : undefined}
            accessibilityState={{ disabled: (empty() && !props.running) || props.disabled }}
            onPress={send}
            onLongPress={() => {
              if (!props.running) return;
              sendMode.set(sendMode() === "steer" ? "queue" : "steer");
              Haptics.selection();
              toast(sendMode() === "steer" ? tr("Send steers the running turn") : tr("Send queues after the running turn"));
            }}
            class={`mx-[6px] h-8 w-8 items-center justify-center rounded-full bg-primary active:scale-90 ${MOTION}`}
            style={{ opacity: empty() || props.disabled ? 0.4 : 1 }}
          >
            <Icon name={icon()} color="primary-foreground" size={16} />
          </View>
          <If when={props.running && props.onStop}>
            <BaseButton
              testID="stop"
              accessibilityLabel={stopping() ? tr("Force stop") : tr("Stop")}
              onPress={() => {
                props.onStop?.(stopping());
                Haptics.impact("medium");
                stopping.set(true);
                setTimeout(() => stopping.set(false), 4000);
              }}
              class={`mr-[2px] h-8 w-8 items-center justify-center rounded-full ${stopping() ? "bg-danger" : "bg-danger-soft"} active:scale-90 ${MOTION}`}
            >
              <View class={`h-[13px] w-[13px] rounded-[3px] ${stopping() ? "bg-danger-soft" : "bg-danger"}`} />
            </BaseButton>
          </If>
        </View>
      </View>
      {props.accessory}
    </View>
  );
}

/** The bar under the composer: the permission mode, plan mode (phase 2), `@` files (phase 2). */
export function Accessory(props: { mode: Mode; onMode: () => void }) {
  return (
    <View class="flex-row gap-2 px-1">
      <Chip small icon="shield" label={modeLabel(props.mode)} chevron onPress={props.onMode} testID="mode-chip" />
      <Chip small icon="list-checks" label={tr("Plan")} onPress={() => toast(tr("Plan mode comes in phase 2"))} testID="plan-chip" />
      <Chip small icon="at-sign" label="" onPress={() => toast(tr("@ files come in phase 2"))} />
    </View>
  );
}
