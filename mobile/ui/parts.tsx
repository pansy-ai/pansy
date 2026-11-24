// The phone's building blocks, in T3 Code's mobile look (docs/plans/phone-ui.md
// "设计令牌"): zinc neutrals, pansy's indigo, flat lists with inset hairlines,
// glass for what floats, 24pt grouped cards, round buttons. Colors are the
// tokens of nsh.json's theme, light and dark.
import { colorScheme, Image, Scroll, signal, Text, token, View, type JSX, type Tokens } from "nsh";
import { Button as BaseButton, Input as BaseInput, Sheet } from "nsh/base";
import { ICONS } from "./icons";
import { LOGOS } from "./logos";
import { tr } from "./i18n";

export type IconName = keyof typeof ICONS;
export type Color = Exclude<keyof Tokens, "radius" | "font-sans" | "font-mono">;

const dark = () => colorScheme() === "dark";

/** The shadows of what floats: the composer, glass. */
export const SHADOW = {
  composer: () => `0 6px 28px ${dark() ? "rgba(0,0,0,0.35)" : "rgba(0,0,0,0.15)"}`,
  glass: () => `0 14px 28px ${dark() ? "rgba(0,0,0,0.22)" : "rgba(0,0,0,0.08)"}`,
  small: () => `0 4px 14px ${dark() ? "rgba(0,0,0,0.22)" : "rgba(0,0,0,0.08)"}`,
};

/** T3's MOBILE_MOTION: an adjustment, 120ms. */
export const MOTION = "transition duration-120 ease-out";

/** Glass: translucent ground, a blur behind and a hairline edge. */
export const GLASS = "bg-glass border border-border";
export const BLUR = { backdropFilter: "blur(20px)" } as const;
/** What floats over the list as T3's does (the tab bar, compose, search, the filters): Liquid Glass on iOS 26, the blur elsewhere. */
export const LIQUID = { ...BLUR, glassEffect: "regular" } as const;

const svgSrc = (svg: string, color: string) => `data:image/svg+xml,${encodeURIComponent(svg.replaceAll("currentColor", color))}`;

/** A Lucide icon in one of the theme's colors, written into the SVG. */
export function Icon(props: { name: IconName; color?: Color; size?: number }) {
  const size = () => props.size ?? 18;
  return <Image src={svgSrc(ICONS[props.name], token(props.color ?? "fg"))} style={{ width: size(), height: size() }} />;
}

export type LogoName = keyof typeof LOGOS;

/** Each agent's logo: Claude keeps its orange, the others are drawn as the files have them. */
const AGENT_LOGOS: Record<string, LogoName> = {
  "claude-code": "claudecode-color",
  claude: "claudecode-color",
  codex: "codex-color",
  "gemini-cli": "geminicli-color",
  opencode: "opencode",
};

/** Each provider's logo, by the provider's id or the start of a model's name. */
const PROVIDER_LOGOS: Record<string, LogoName> = {
  deepseek: "deepseek-color",
  codex: "openai",
  openai: "openai",
  anthropic: "claude-color",
  claude: "claude-color",
  kimi: "kimi",
  moonshot: "kimi",
  qwen: "qwen-color",
};

export const agentLogo = (agent: string): LogoName | null => AGENT_LOGOS[agent] ?? null;
export const providerLogo = (id: string): LogoName | null => PROVIDER_LOGOS[id.split("/")[0]] ?? null;

/** The agents' names as people say them. */
export function agentName(agent: string): string {
  return { codex: "Codex", "claude-code": "Claude Code", claude: "Claude Code", "gemini-cli": "Gemini CLI", opencode: "OpenCode" }[agent] ?? agent;
}

/** A logo; one in `currentColor` takes the ink. */
export function Logo(props: { name: LogoName | null; size?: number }) {
  const size = () => props.size ?? 14;
  return (
    <If when={props.name} else={<Icon name="circle" color="fg-3" size={size()} />}>
      <Image src={svgSrc(LOGOS[props.name!], token("fg"))} style={{ width: size(), height: size() }} />
    </If>
  );
}

/** A project's mark, T3's monogram: two letters in mono, in a color of its name on a 15% wash of it. */
export function ProjectMark(props: { name: string; dim?: boolean }) {
  const hue = () => monogramHue(props.name);
  const text = () => monogram(props.name);
  return (
    <View class="h-5 w-5 items-center justify-center rounded-[5px]" style={{ backgroundColor: `${hue()}26`, opacity: props.dim ? 0.4 : 1 }}>
      <Text class="font-mono" style={{ color: hue(), fontSize: [...text()].length === 1 ? 12 : 10.3, lineHeight: 20 }}>
        {text()}
      </Text>
    </View>
  );
}

/** T3's monogram (`projectIcon.ts`): the first word's first letter, then its first digit, the last word's first letter or its own last letter; `pansy` is "PY". */
export function monogram(name: string): string {
  const words = name.normalize("NFKC").trim().match(/[\p{L}\p{N}]+/gu) ?? [];
  const first = words[0];
  if (!first) return "PR";
  const glyphs = [...first];
  const second = glyphs.slice(1).find((g) => /\p{N}/u.test(g)) ?? (words.length > 1 ? [...words[words.length - 1]][0] : glyphs[glyphs.length - 1]) ?? glyphs[0];
  return [...`${glyphs[0]}${second}`.toUpperCase()].slice(0, 2).join("");
}

/** T3's monogram colors, Tailwind's 500s, picked by a hash of the name. */
const MONOGRAM_HUES = ["#6b7280", "#ef4444", "#f97316", "#f59e0b", "#eab308", "#84cc16", "#22c55e", "#10b981", "#14b8a6", "#06b6d4", "#0ea5e9", "#3b82f6", "#6366f1", "#8b5cf6", "#a855f7", "#d946ef", "#ec4899", "#f43f5e"];

export function monogramHue(name: string): string {
  const seed = name.normalize("NFKC").trim().toLocaleLowerCase("en-US") || "project";
  let i = 0;
  for (const g of seed) i = (i * 31 + (g.codePointAt(0) ?? 0)) % MONOGRAM_HUES.length;
  return MONOGRAM_HUES[i];
}

/** A round glass button of the header: filter, new task, back. */
export function GlassButton(props: { icon: IconName; label: string; onPress: () => void; testID?: string; size?: number }) {
  const size = () => props.size ?? 38;
  return (
    <BaseButton
      onPress={props.onPress}
      accessibilityLabel={props.label}
      testID={props.testID}
      class={`items-center justify-center rounded-full ${GLASS} active:scale-90 ${MOTION}`}
      style={{ width: size(), height: size(), boxShadow: SHADOW.small(), ...LIQUID }}
    >
      <Icon name={props.icon} size={18} />
    </BaseButton>
  );
}

/** A filter or option capsule (T3's `Chip`): glass with the muted ink, the chosen one filled with the accent. */
export function Chip(props: { label: string; on?: boolean; icon?: IconName; logo?: LogoName | null; chevron?: boolean; onPress?: () => void; testID?: string; small?: boolean }) {
  const ink = (): Color => (props.on ? "primary-foreground" : "fg-2");
  return (
    <View
      role="button"
      testID={props.testID}
      onPress={props.onPress}
      class={`flex-row items-center gap-[6px] rounded-2xl px-3 active:opacity-70 ${props.small ? "h-[30px]" : "h-8"} ${props.on ? "bg-primary" : GLASS}`}
      style={props.on ? undefined : LIQUID}
    >
      <If when={props.icon}>
        <Icon name={props.icon!} color={ink()} size={props.small ? 14 : 15} />
      </If>
      <If when={props.logo}>
        <Logo name={props.logo!} />
      </If>
      <Text class={`max-w-[180px] font-medium text-[13px] text-${ink()} line-clamp-1`}>{props.label}</Text>
      <If when={props.chevron}>
        <Icon name="chevron-down" color={ink()} size={11} />
      </If>
    </View>
  );
}

/** A full-width round 48pt button: the accent, a quiet one, a red one, the inverted one, an outlined one. */
export function Btn(props: { label: string; onPress: () => void; tone?: "primary" | "quiet" | "danger" | "prominent" | "outline"; icon?: IconName; disabled?: boolean; testID?: string; class?: string }) {
  const look = () =>
    ({
      primary: { box: "bg-primary", ink: "primary-foreground" as Color },
      quiet: { box: "bg-grouped", ink: "fg" as Color },
      danger: { box: "bg-danger-soft", ink: "danger" as Color },
      prominent: { box: "bg-prominent", ink: "prominent-fg" as Color },
      outline: { box: "bg-card border border-border", ink: "fg" as Color },
    })[props.tone ?? "primary"];
  return (
    <BaseButton
      onPress={props.onPress}
      disabled={props.disabled}
      testID={props.testID}
      accessibilityLabel={props.label}
      class={`h-12 flex-row items-center justify-center gap-2 rounded-full px-5 active:scale-[0.98] disabled:opacity-50 ${look().box} ${MOTION} ${props.class ?? ""}`}
    >
      <If when={props.icon}>
        <Icon name={props.icon!} color={look().ink} size={18} />
      </If>
      <Text class={`text-[16px] font-bold text-${look().ink}`}>{props.label}</Text>
    </BaseButton>
  );
}

/** A grouped card: 24pt corners on the grouped ground. */
export function Group(props: { children?: JSX.Element; testID?: string; class?: string; danger?: boolean }) {
  return (
    <View testID={props.testID} class={`overflow-hidden rounded-3xl bg-grouped ${props.danger ? "border border-danger" : ""} ${props.class ?? ""}`}>
      {props.children}
    </View>
  );
}

/** The small heading over a group. */
export function GroupHead(props: { text: string; children?: JSX.Element }) {
  return (
    <View class="flex-row items-center px-4">
      <Text class="flex-1 text-[13px] font-medium text-fg-3">{props.text}</Text>
      {props.children}
    </View>
  );
}

/** A row of a group: an icon, a name and a line under it, what goes on the right; a hairline over it unless first. */
export function GroupRow(props: {
  title: string;
  sub?: string;
  icon?: IconName;
  lead?: JSX.Element;
  trailing?: JSX.Element;
  chevron?: boolean;
  first?: boolean;
  onPress?: () => void;
  onLongPress?: () => void;
  testID?: string;
  tone?: "danger" | "primary" | "muted";
  mono?: boolean;
}) {
  const ink = (): Color => (props.tone === "danger" ? "danger" : props.tone === "primary" ? "primary" : props.tone === "muted" ? "fg-2" : "fg");
  return (
    <View
      role={props.onPress ? "button" : undefined}
      testID={props.testID}
      onPress={props.onPress}
      onLongPress={props.onLongPress}
      class={`min-h-[56px] flex-row items-center gap-3 px-4 py-3 ${props.onPress ? "active:bg-line" : ""}`}
    >
      <If when={!props.first}>
        <View class="absolute right-0 top-0 h-px bg-line" style={{ left: props.icon || props.lead ? 50 : 16 }} />
      </If>
      <If when={props.icon}>
        <Icon name={props.icon!} color={props.tone ? ink() : "fg-2"} size={22} />
      </If>
      {props.lead}
      <View class="min-w-0 flex-1 gap-[1px]">
        <Text class={`text-[16px] font-medium text-${ink()} ${props.mono ? "font-mono text-[14px]" : ""}`}>{props.title}</Text>
        <If when={props.sub}>
          <Text class="text-[14px] text-fg-2 line-clamp-2">{props.sub ?? ""}</Text>
        </If>
      </View>
      {props.trailing}
      <If when={props.chevron}>
        <Icon name="chevron-right" color="fg-3" size={14} />
      </If>
    </View>
  );
}

/** A switch as iOS draws one: 51×31, green when on. */
export function Toggle(props: { on: boolean; onChange: (on: boolean) => void; label: string; testID?: string; disabled?: boolean }) {
  return (
    <View
      role="switch"
      accessibilityLabel={props.label}
      accessibilityState={{ checked: props.on, disabled: props.disabled }}
      testID={props.testID}
      onPress={() => !props.disabled && props.onChange(!props.on)}
      class={`h-[31px] w-[51px] rounded-full p-[2px] ${props.on ? "bg-toggle-on" : "bg-border"} ${props.disabled ? "opacity-50" : ""} ${MOTION}`}
    >
      <View
        class={`h-[27px] w-[27px] rounded-full bg-thumb ${MOTION}`}
        style={{ transform: [{ translateX: props.on ? 20 : 0 }], boxShadow: "0 2px 4px rgba(0,0,0,0.2)" }}
      />
    </View>
  );
}

/** A segmented control: a grouped track, the choice on a card thumb. */
export function Seg<T extends string>(props: { options: { value: T; label: string }[]; value: T; onChange: (v: T) => void; testID?: string }) {
  return (
    <View testID={props.testID} class="flex-row rounded-[10px] bg-grouped p-[3px]">
      <Each of={props.options} key={(o) => o.value}>
        {(o) => {
          const on = () => props.value === o().value;
          return (
            <View
              role="tab"
              accessibilityState={{ selected: on() }}
              testID={`${props.testID ?? "seg"}-${o().value}`}
              onPress={() => props.onChange(o().value)}
              class={`flex-1 items-center rounded-lg py-[6px] ${on() ? "bg-card" : ""}`}
              style={on() ? { boxShadow: SHADOW.small() } : undefined}
            >
              <Text class={`text-[13px] font-medium ${on() ? "text-fg" : "text-fg-2"}`}>{o().label}</Text>
            </View>
          );
        }}
      </Each>
    </View>
  );
}

/** A text field on the grouped ground, 12pt corners. */
export function Field(props: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  testID?: string;
  label?: string;
  mono?: boolean;
  multiline?: boolean;
  masked?: boolean;
  class?: string;
  onSubmit?: () => void;
  autoFocus?: boolean;
  keyboardType?: "default" | "url" | "numeric";
}) {
  return (
    <BaseInput
      value={props.value}
      onChange={({ value }) => props.onChange(value)}
      onSubmit={() => props.onSubmit?.()}
      placeholder={props.placeholder}
      accessibilityLabel={props.label ?? props.placeholder}
      testID={props.testID}
      multiline={props.multiline}
      masked={props.masked}
      autoFocus={props.autoFocus}
      autoCapitalize="none"
      autoCorrect={false}
      keyboardType={props.keyboardType}
      class={`rounded-xl bg-grouped px-[14px] py-3 text-[16px] text-fg placeholder:text-fg-3 ${props.mono ? "font-mono" : ""} ${props.class ?? ""}`}
    />
  );
}

/** A bare input inside a group's row. */
export function RowInput(props: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; testID?: string; first?: boolean; mono?: boolean; masked?: boolean; keyboardType?: "default" | "url" | "number-pad"; maxLength?: number }) {
  return (
    <View class="min-h-[52px] flex-row items-center px-4">
      <If when={!props.first}>
        <View class="absolute left-4 right-0 top-0 h-px bg-line" />
      </If>
      <Text class="w-20 text-[16px] font-medium text-fg">{props.label}</Text>
      <BaseInput
        value={props.value}
        onChange={({ value }) => props.onChange(value)}
        placeholder={props.placeholder}
        accessibilityLabel={props.label}
        testID={props.testID}
        masked={props.masked}
        keyboardType={props.keyboardType}
        maxLength={props.maxLength}
        autoCapitalize="none"
        autoCorrect={false}
        class={`flex-1 py-3 text-[15px] text-fg placeholder:text-fg-3 ${props.mono ? "font-mono" : ""}`}
      />
    </View>
  );
}

/** A note on a soft ground: amber for a warning, red for an error with what to do. */
export function Note(props: { text: string; tone: "warning" | "danger"; icon?: IconName; action?: { label: string; onPress: () => void }; testID?: string }) {
  return (
    <View testID={props.testID} class={`flex-row items-start gap-2 rounded-2xl px-[14px] py-[10px] ${props.tone === "warning" ? "bg-warning-soft" : "border border-danger bg-danger-soft"}`}>
      <Icon name={props.icon ?? (props.tone === "warning" ? "shield-alert" : "circle-alert")} color={props.tone} size={16} />
      <Text class={`flex-1 text-[14px] text-${props.tone}`}>{props.text}</Text>
      <If when={props.action}>
        <Text role="button" onPress={() => props.action!.onPress()} class={`text-[14px] font-bold text-${props.tone}`}>
          {props.action?.label ?? ""}
        </Text>
      </If>
    </View>
  );
}

/** A turning circle, the "working" mark. */
export function Spinner(props: { size?: number; color?: Color }) {
  return (
    <View style={{ animationName: "spin", animationDuration: 900, animationTimingFunction: "linear", animationIterationCount: "infinite" }}>
      <Icon name="loader-circle" color={props.color ?? "sky"} size={props.size ?? 15} />
    </View>
  );
}

/** Loading, as T3 shows it: a spinner and a line in 18 bold, in the middle. */
export function Loading(props: { text: string }) {
  return (
    <View class="flex-1 items-center justify-center gap-3 py-16">
      <Spinner size={22} />
      <Text class="text-[18px] font-bold text-fg">{props.text}</Text>
    </View>
  );
}

/**
 * A bottom sheet with a grabber and two resting heights (its start, then
 * the whole screen), as phone-ui.md "导航" has them; a title in the middle of
 * its head and the buttons on either side.
 */
export function BottomSheet(props: {
  open: boolean;
  onClose: () => void;
  title?: string;
  titleMono?: boolean;
  subtitle?: string;
  start?: number;
  left?: JSX.Element;
  right?: JSX.Element;
  children?: JSX.Element;
  footer?: JSX.Element;
  testID?: string;
}) {
  return (
    <Sheet
      open={props.open}
      onOpenChange={(e) => !e.open && props.onClose()}
      detents={[{ fraction: props.start ?? 0.55 }, "large"]}
      grabber
      closeOnSwipe
      grabberClass="mx-auto mt-[6px] h-[5px] w-9 rounded-full bg-fg-3 opacity-35"
      popupClass="rounded-t-[28px] bg-sheet"
      popupStyle={{ ...BLUR, boxShadow: SHADOW.glass() }}
      backdropClass="bg-backdrop"
      accessibilityLabel={props.title}
      footer={props.footer}
      footerClass="bg-sheet px-5 pb-[calc(env(safe-area-inset-bottom)+12px)] pt-2"
    >
      <View testID={props.testID} class="flex-1">
        <View class="min-h-[52px] flex-row items-center gap-2 px-4 pb-[6px] pt-2">
          <View class="w-[38px]">{props.left}</View>
          <View class="min-w-0 flex-1 items-center">
            <If when={props.title}>
              <Text class={`font-bold text-fg line-clamp-1 ${props.titleMono ? "font-mono text-[16px]" : "text-[18px]"}`}>{props.title ?? ""}</Text>
            </If>
            <If when={props.subtitle}>
              <Text class="text-[13px] text-fg-2 line-clamp-1">{props.subtitle ?? ""}</Text>
            </If>
          </View>
          <View class="w-[38px] items-end">{props.right ?? <GlassButton icon="x" label={tr("Close")} onPress={props.onClose} testID="sheet-close" />}</View>
        </View>
        <Scroll class="flex-1" keyboardShouldPersistTaps="handled">
          {props.children}
          {/* the footer (a 48pt button, 8 over, 12 and the safe area under) is drawn over the content's end: room to scroll it clear */}
          <If when={props.footer}>
            <View class="pb-[calc(env(safe-area-inset-bottom)+68px)]" />
          </If>
        </Scroll>
      </View>
    </Sheet>
  );
}

/** A line of short messages at the bottom: glass, 28pt corners, a tick and what happened, maybe "Undo". */
export type ToastItem = { id: number; text: string; tone?: "ok" | "error"; action?: { label: string; onPress: () => void } };
export const toasts = signal<ToastItem[]>([]);
let toastId = 0;

export function toast(text: string, options: { tone?: "ok" | "error"; action?: { label: string; onPress: () => void } } = {}) {
  const item = { id: ++toastId, text, ...options };
  toasts.set([...toasts(), item].slice(-3));
  setTimeout(() => toasts.set(toasts().filter((t) => t.id !== item.id)), options.action ? 5000 : 3000);
}

export function Toasts() {
  return (
    <View class="absolute bottom-0 left-3 right-3 gap-2 pb-[calc(env(safe-area-inset-bottom)+96px)]" style={{ pointerEvents: "box-none" }}>
      <Each of={toasts()} key={(t) => t.id}>
        {(t) => (
          <View testID="toast" class={`min-h-14 flex-row items-center gap-[10px] rounded-[28px] py-2 pl-4 pr-2 ${GLASS}`} style={{ ...BLUR, boxShadow: SHADOW.glass() }}>
            <Icon name={t().tone === "error" ? "circle-alert" : "circle-check"} color={t().tone === "error" ? "danger" : "emerald"} size={20} />
            <Text class="flex-1 text-[14px] font-semibold text-fg">{t().text}</Text>
            <If when={t().action}>
              <Chip
                label={t().action?.label ?? ""}
                onPress={() => {
                  t().action?.onPress();
                  toasts.set(toasts().filter((x) => x.id !== t().id));
                }}
              />
            </If>
          </View>
        )}
      </Each>
    </View>
  );
}

/** `+N −N` in emerald and rose. */
export function AddDel(props: { add: number; del: number; size?: number; mono?: boolean }) {
  const cls = () => `${props.mono ? "font-mono" : ""} text-[${props.size ?? 13}px]`;
  return (
    <View class="flex-row items-center gap-1">
      <If when={props.add > 0 || props.del === 0}>
        <Text class={`${cls()} text-emerald`}>+{props.add}</Text>
      </If>
      <If when={props.del > 0}>
        <Text class={`${cls()} text-rose`}>−{props.del}</Text>
      </If>
    </View>
  );
}

/** A choice among a few, in a sheet: each a row, the chosen one ticked. */
export function PickSheet<T extends string>(props: {
  open: boolean;
  onClose: () => void;
  title: string;
  options: { value: T; label: string; sub?: string; icon?: IconName; logo?: LogoName | null }[];
  value: T | null;
  onPick: (v: T) => void;
  testID?: string;
  children?: JSX.Element;
}) {
  return (
    <BottomSheet open={props.open} onClose={props.onClose} title={props.title} start={0.5} testID={props.testID}>
      <View class="gap-[14px] px-5 pb-8 pt-[6px]">
        <Group>
          <Each of={props.options} key={(o) => o.value}>
            {(o, i) => (
              <GroupRow
                first={i() === 0}
                title={o().label}
                sub={o().sub}
                icon={o().icon}
                lead={o().logo ? <Logo name={o().logo!} size={22} /> : undefined}
                testID={`${props.testID ?? "pick"}-${o().value}`}
                onPress={() => {
                  props.onPick(o().value);
                  props.onClose();
                }}
                trailing={props.value === o().value ? <Icon name="check" color="primary" size={18} /> : undefined}
              />
            )}
          </Each>
        </Group>
        {props.children}
      </View>
    </BottomSheet>
  );
}
