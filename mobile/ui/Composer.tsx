// 输入框 (phone-ui.md "3. 输入框"): a glass capsule floating over the feed,
// one line: the text, the model, send; a long text wraps and grows upward,
// the model and send kept at the bottom. While a turn runs, send steers it, or
// queues after it (a long press switches), and a red stop sits beside it.
// The permission mode is chosen in the model's sheet.
import { signal, Text, View, type InputRef } from "nsh";
import { Button as BaseButton, Input } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Mode } from "../net/types";
import { tr } from "./i18n";
import { agentLogo, BLUR, Icon, Logo, MOTION, SHADOW, toast, type IconName } from "./parts";
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
  disabled?: boolean;
  /** The input took the keyboard (true) or let it go. */
  onFocusChange?: (focused: boolean) => void;
}) {
  const sendMode = signal<SendMode>("steer");
  const stopping = signal(false);
  const empty = () => !props.text.trim();
  const icon = (): IconName => (!props.running ? "arrow-up" : sendMode() === "steer" ? "corner-left-up" : "list-ordered");
  const label = () => (!props.running ? tr("Send") : sendMode() === "steer" ? tr("Steer") : tr("Queue"));
  let input: InputRef | undefined;
  const send = () => {
    if (empty() || props.disabled) return;
    props.onSend(props.running ? sendMode() : "start");
    // sent: the keyboard goes, the feed shows what follows
    input?.blur();
  };
  return (
    <View class="gap-[10px]">
      <View
        class="flex-row items-end gap-[2px] rounded-[26px] border border-border bg-composer p-[6px]"
        style={{ ...BLUR, boxShadow: SHADOW.composer() }}
      >
        <Input
          ref={(r) => (input = r)}
          onFocus={() => props.onFocusChange?.(true)}
          onBlur={() => props.onFocusChange?.(false)}
          value={props.text}
          onChange={({ value }) => props.onText(value)}
          placeholder={props.placeholder}
          accessibilityLabel={tr("Message")}
          testID="composer-input"
          multiline
          submitBehavior="newline"
          ios={{ inputAccessoryViewID: "composer-accessory" }}
          class="max-h-[160px] min-h-10 flex-1 pl-3 pr-1 py-[9px] text-[16px] leading-[22px] text-fg placeholder:text-fg-3"
        />
        <View class="h-10 flex-row items-center gap-[2px]">
          <View
            role="button"
            testID="model-pill"
            accessibilityLabel={tr("Model: %s", props.model)}
            onPress={props.onModel}
            class="h-9 flex-row items-center gap-1 rounded-full px-2 active:opacity-60"
          >
            <Logo name={agentLogo(props.agent)} size={16} />
            <Text class="text-[13px] font-medium text-fg-2 line-clamp-1" style={{ maxWidth: 80 }}>
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
            class={`mx-[2px] h-9 w-9 items-center justify-center rounded-full bg-primary active:scale-90 ${MOTION}`}
            style={{ opacity: empty() || props.disabled ? 0.4 : 1 }}
          >
            <Icon name={icon()} color="primary-foreground" size={18} />
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
              class={`mr-[2px] h-9 w-9 items-center justify-center rounded-full ${stopping() ? "bg-danger" : "bg-danger-soft"} active:scale-90 ${MOTION}`}
            >
              <View class={`h-[13px] w-[13px] rounded-[3px] ${stopping() ? "bg-danger-soft" : "bg-danger"}`} />
            </BaseButton>
          </If>
        </View>
      </View>
    </View>
  );
}
