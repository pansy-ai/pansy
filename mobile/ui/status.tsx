// A thread's status as the list and the thread show it: a 13pt bold word in
// its color and its icon (phone-ui.md "状态色"); working is the timer.
import { Text, View } from "nsh";
import type { ThreadSummary } from "../net/types";
import { tr } from "./i18n";
import { Icon, Spinner, type Color, type IconName } from "./parts";
import { now, timer, when } from "./time";

export type Look = { word: string; icon: IconName; color: Color; busy: boolean; spin?: boolean } | null;

/** How `t`'s status looks; null when it shows its time instead. */
export function statusLook(t: ThreadSummary): Look {
  switch (t.status) {
    case "working":
      return t.startedAt
        ? { word: timer(now() - Date.parse(t.startedAt)), icon: "loader-circle", color: "sky", busy: true, spin: true }
        : { word: tr("Waiting"), icon: "circle-dashed", color: "fg-2", busy: true };
    case "approval":
      return { word: tr("Awaiting approval"), icon: "shield-alert", color: "warning", busy: false };
    case "input":
      return { word: tr("Needs answer"), icon: "message-circle-question", color: "indigo", busy: false };
    case "failed":
      return /quota|limit|额度/i.test(t.error ?? "")
        ? { word: tr("Out of quota"), icon: "circle-alert", color: "warning", busy: false }
        : { word: tr("Failed"), icon: "circle-alert", color: "danger", busy: false };
    case "done":
      return t.unread === false ? null : { word: tr("Done"), icon: "circle-check", color: "emerald", busy: false };
    case "unsent":
      return { word: tr("Unsent"), icon: "clock-arrow-up", color: "fg-2", busy: false };
    default:
      return t.unread ? { word: tr("Done"), icon: "circle-check", color: "emerald", busy: false } : null;
  }
}

/** The status word and icon, or the time. */
export function Status(props: { thread: ThreadSummary; testID?: string }) {
  const look = () => statusLook(props.thread);
  const color = (): Color => look()?.color ?? "fg-3";
  return (
    <If when={look()} else={<Text class="text-[13px] leading-[23px] text-fg-3">{when(props.thread.updatedAt)}</Text>}>
      <View testID={props.testID} class="flex-row items-center gap-1">
        <If when={look()?.spin} else={<Icon name={look()?.icon ?? "circle"} color={color()} size={13} />}>
          <Spinner size={13} />
        </If>
        <Text class={`text-[13px] font-bold leading-[23px] text-${color()}`} style={look()?.spin ? { fontFamily: "system-ui", fontVariant: ["tabular-nums"] } : undefined}>
          {look()?.word ?? ""}
        </Text>
      </View>
    </If>
  );
}
