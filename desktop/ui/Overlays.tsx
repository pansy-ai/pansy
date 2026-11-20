// What sits above the pages: a question to answer before something is
// done, and short toasts.
import { keymap, Text, View } from "nsh";
import { tr } from "./i18n";
import { question, toasts } from "./model";
import { Modal, PrimaryButton, SHADOW, TextButton } from "./parts";

/** A question in magpie's modal: what happens, Cancel and the action. */
export function QuestionDialog() {
  keymap.bind("escape", () => question()?.answer(false));
  return (
    <If when={question()}>
      <Modal
        testID="question"
        width={420}
        title={question()?.title ?? ""}
        onClose={() => question()?.answer(false)}
        foot={
          <>
            <View class="flex-1" />
            <TextButton label={tr("Cancel")} testID="question-cancel" onPress={() => question()?.answer(false)} />
            <PrimaryButton danger={question()?.danger} label={question()?.confirm ?? ""} testID="question-confirm" onPress={() => question()?.answer(true)} />
          </>
        }
      >
        <Text class="text-[13px] text-fg-2">{question()?.description ?? ""}</Text>
      </Modal>
    </If>
  );
}

/** Toasts at the foot of the window, as magpie's. */
export function Toasts() {
  return (
    <View style={{ pointerEvents: "box-none" }} class="absolute bottom-4 left-[212px] right-0 items-center gap-2">
      <Each of={toasts()} key={(t) => t.id}>
        {(t) => (
          <View
            style={{ boxShadow: SHADOW.pop() }}
            class={`max-w-[520px] rounded-[10px] border px-[14px] py-[8px] ${t().tone === "error" ? "border-destructive/40 bg-destructive-soft" : "border-border bg-popover"}`}
          >
            <Text testID="toast" class={`text-[13px] ${t().tone === "error" ? "text-destructive" : "text-foreground"}`}>
              {t().message}
            </Text>
          </View>
        )}
      </Each>
    </View>
  );
}
