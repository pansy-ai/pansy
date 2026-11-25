// 输入框 (phone-ui.md "3. 输入框"): a glass capsule floating over the feed,
// one line: the text, the model, send; a long text wraps and grows upward,
// the model and send kept at the bottom. While a turn runs, send steers it, or
// queues after it (a long press switches), and a red stop sits beside it.
// The permission mode is chosen in the model's sheet. The microphone dictates
// into the text where the caret was, live as it is heard; nothing is sent.
import { cleanup, host, signal, Speech, Text, untracked, View, type InputRef, type SpeechErrorCode, type SpeechSession } from "nsh";
import { Button as BaseButton, Input } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Mode } from "../net/types";
import { locale, tr } from "./i18n";
import { agentLogo, BLUR, Icon, Logo, MOTION, SHADOW, toast, type IconName } from "./parts";
import { shortModel } from "./ModelSheet";

export type SendMode = "steer" | "queue";

/** What a failed dictation says; `native`: the system's recognizer, else the browser's. */
function speechError(error: SpeechErrorCode | string, native: boolean): string {
  switch (error) {
    case "not-allowed":
    case "service-not-allowed":
      return native
        ? tr("No microphone or speech recognition permission. Allow them in Settings.")
        : tr("No microphone permission. Allow the microphone in the browser's settings.");
    case "no-speech":
      return tr("Didn't hear anything");
    case "audio-capture":
      return tr("No microphone found");
    case "network":
      return tr("Voice input needs a network connection");
    case "language-not-supported":
      return tr("This device can't recognize this language");
    case "unavailable":
      return tr("Voice input isn't available here");
    default:
      return tr("Voice input failed");
  }
}

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

  // Dictation (after noop's): NSH's Speech where the system recognizes speech
  // (iOS, macOS), the browser's through host.js on the web; where neither
  // does, no microphone.
  const canDictate = signal(false);
  const listening = signal<"off" | "on" | "stopping">("off");
  let native = false;
  let session: SpeechSession | null = null;
  void Promise.resolve()
    .then(() => Speech.available())
    .catch(() => false)
    .then(async (ok) => {
      native = ok;
      if (ok || !host.available()) return canDictate.set(ok);
      const state = (await host.call("speechState").catch(() => null)) as { supported?: boolean } | null;
      canDictate.set(!!state?.supported);
    });
  let caret: { start: number; end: number } | null = null;
  // the text before and after where the words go
  let before = "";
  let after = "";
  // the text as dictation last left it: any other is the user's typing
  let dictated = "";
  /** The whole transcript so far, final and still-being-heard words. */
  const heard = (text: string) => {
    if (untracked(listening) === "off") return;
    const words = text.trim();
    // a space between Latin words, none in Chinese
    const gap = /[A-Za-z0-9.,!?;:]$/.test(before) && /^[A-Za-z0-9]/.test(words) ? " " : "";
    const head = before + gap + words;
    dictated = head + after;
    props.onText(dictated);
    input?.setSelection(head.length);
  };
  const ended = (error?: string | null) => {
    if (untracked(listening) === "off") return;
    listening.set("off");
    if (error) toast(speechError(error, native), { tone: "error" });
  };
  const couldNotStart = () => {
    session = null;
    listening.set("off");
    toast(tr("Couldn't start voice input"), { tone: "error" });
  };
  const startDictation = () => {
    const v = props.text;
    const start = Math.min(caret?.start ?? v.length, v.length);
    const end = Math.min(Math.max(caret?.end ?? v.length, start), v.length);
    before = v.slice(0, start);
    after = v.slice(end);
    dictated = v;
    listening.set("on");
    Haptics.selection();
    // the app's Chinese, else the system's language
    const lang = locale() === "zh-Hans" ? "zh-CN" : null;
    if (!native) {
      host.call("speechStart", { lang }).catch(() => untracked(listening) === "on" && couldNotStart());
      return;
    }
    try {
      const s = Speech.start({ lang });
      session = s;
      s.on("result", (r) => session === s && heard(r.text));
      s.on("end", (e) => {
        if (session !== s) return;
        session = null;
        ended(e.error);
      });
    } catch {
      couldNotStart();
    }
  };
  /** Stops listening: the words still being heard come first, unless `drop`
   *  (the text was sent or typed into). */
  const stopDictation = (drop: boolean) => {
    if (untracked(listening) === "off") return;
    if (drop) {
      listening.set("off");
      const s = session;
      session = null;
      if (s) s.abort();
      else void host.call("speechAbort").catch(() => {});
    } else {
      listening.set("stopping");
      if (session) session.stop();
      else void host.call("speechStop").catch(() => {});
    }
  };
  if (host.available()) {
    host.on("speech", (p) => heard(String((p as { text?: string } | null)?.text ?? "")));
    host.on("speechEnd", (p) => ended((p as { error?: string } | null)?.error));
  }
  cleanup(() => stopDictation(true));

  const send = () => {
    if (empty() || props.disabled) return;
    stopDictation(true);
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
          onChange={({ value }) => {
            // typing takes the text back from dictation
            if (value !== dictated) stopDictation(true);
            props.onText(value);
          }}
          onSelectionChange={(e) => (caret = e)}
          placeholder={props.placeholder}
          accessibilityLabel={tr("Message")}
          testID="composer-input"
          multiline
          submitBehavior="newline"
          ios={{ inputAccessoryViewID: "composer-accessory" }}
          class="max-h-[160px] min-h-10 flex-1 pl-3 pr-1 py-[9px] text-[16px] leading-[22px] text-fg placeholder:text-fg-3"
        />
        <View class="h-10 flex-row items-center gap-[2px]">
          <If when={canDictate()}>
            <BaseButton
              testID="dictate"
              accessibilityLabel={listening() === "off" ? tr("Voice input") : tr("Stop voice input")}
              onPress={() => (listening() === "off" ? startDictation() : stopDictation(listening() === "stopping"))}
              class={`h-9 w-9 items-center justify-center rounded-full ${listening() === "off" ? "" : "bg-danger-soft"} active:scale-90 ${MOTION}`}
            >
              <Icon name="mic" color={listening() === "off" ? "fg-2" : "danger"} size={18} />
            </BaseButton>
          </If>
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
