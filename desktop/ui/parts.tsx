// The building blocks of every screen: shadcn/ui's headless parts from
// nsh/base (Switch, ToggleGroup, Popover, Tooltip, Input) dressed as magpie
// draws its window, with the colors of nsh.json's theme (magpie's palette,
// light and dark) and its sizes: 13px text, 12px hairline cards, 26px
// controls.
import { colorScheme, Image, Scroll, signal, Text, token, View, type JSX, type Tokens } from "nsh";
import { Button as BaseButton, Checkbox, Input as BaseInput, Menu, MenuItem, Switch as BaseSwitch, Toggle, ToggleGroup, Tooltip } from "nsh/base";
import { tr } from "./i18n";
import { ICONS } from "./icons";
import atomcode from "./logos/atomcode.png";
import ccSwitch from "./logos/cc-switch.png";
import crush from "./logos/crush.png";
import gpustack from "./logos/gpustack.png";
import mlxServe from "./logos/mlx-serve.png";
import mothx from "./logos/mothx.png";
import snow from "./logos/snow.png";
import zcode from "./logos/zcode.png";
import { LOGOS } from "./logos";
import { presets } from "./model";

export type IconName = keyof typeof ICONS;
export type Color = Exclude<keyof Tokens, "radius">;

const dark = () => colorScheme() === "dark";

/** magpie's shadows, which differ by scheme. */
export const SHADOW = {
  /** The thumb of a segmented control, the open page in the sidebar. */
  thumb: () => (dark() ? "inset 0 0.5px 0 rgba(255,255,255,0.08), 0 1px 2px rgba(0,0,0,0.3)" : "0 1px 2px rgba(0,0,0,0.14), 0 0 0 0.5px rgba(0,0,0,0.08)"),
  /** A popover. */
  pop: () => (dark() ? "0 16px 44px rgba(0,0,0,0.55), 0 2px 8px rgba(0,0,0,0.3), 0 0 0 0.5px rgba(255,255,255,0.1)" : "0 16px 44px rgba(20,22,30,0.16), 0 2px 8px rgba(20,22,30,0.06), 0 0 0 0.5px rgba(20,22,30,0.1)"),
  /** A modal. */
  modal: () => (dark() ? "0 24px 60px rgba(0,0,0,0.6), 0 2px 8px rgba(0,0,0,0.3)" : "0 24px 60px rgba(0,0,0,0.32), 0 2px 8px rgba(0,0,0,0.12)"),
};

/**
 * magpie's motion (app.css): one easing, quick to start and soft to land,
 * over a sixth of a second, for a control's ground, ink and press.
 */
export const MOTION = "transition duration-160 ease-[cubic-bezier(0.2,0.8,0.2,1)]";

const svgSrc = (svg: string, color: string) => `data:image/svg+xml,${encodeURIComponent(svg.replaceAll("currentColor", color))}`;

/**
 * One of the window's icons (ui/icons) in one of the theme's colors. The
 * color is written into the SVG rather than tinted, so every host draws it
 * at its size and in place. `hover` is the color it turns while the pointer
 * is over the `group` it is in, as magpie's icons go from muted to ink: the
 * icon again in that color, faded in over it.
 */
export function Icon(props: { name: IconName; color?: Color; hover?: Color; size?: number }) {
  const size = () => props.size ?? 16;
  const plain = () => <Image src={svgSrc(ICONS[props.name], token(props.color ?? "foreground"))} style={{ width: size(), height: size() }} />;
  return (
    <If when={props.hover} else={plain()}>
      <View style={{ width: size(), height: size() }}>
        {plain()}
        <Image
          src={svgSrc(ICONS[props.name], token(props.hover ?? "foreground"))}
          class={`absolute left-0 top-0 opacity-0 group-hover:opacity-100 ${MOTION}`}
          style={{ width: size(), height: size() }}
        />
      </View>
    </If>
  );
}

/** How many of `n` bars an effort lights among its `levels`: none for the agent's own, all for the highest. */
export function effortLit(levels: string[], level: string | null, n: number): number {
  const at = level ? levels.indexOf(level) : -1;
  return at < 0 ? 0 : Math.max(1, Math.round(((at + 1) / levels.length) * n));
}

/** An effort as magpie draws one: four bars, filled up to it. */
export function EffortBars(props: { lit: number; color?: Color; size?: number }) {
  const size = () => props.size ?? 14;
  const svg = () =>
    `<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">${[4, 7, 10, 13]
      .map((h, i) => `<rect x="${1.25 + i * 3.6}" y="${14.5 - h}" width="2.6" height="${h}" rx="1" fill="currentColor" opacity="${i < props.lit ? 1 : 0.28}"/>`)
      .join("")}</svg>`;
  return <Image src={svgSrc(svg(), token(props.color ?? "foreground"))} style={{ width: size(), height: size() }} />;
}

/** The logos that are pictures rather than SVG. */
const PICTURES = { atomcode, "cc-switch": ccSwitch, crush, gpustack, "mlx-serve": mlxServe, mothx, snow, zcode };

export type LogoName = keyof typeof LOGOS | keyof typeof PICTURES | "generic";

/** The logo of each agent (`Agent.id`). */
const AGENT_LOGOS: Record<string, LogoName> = {
  "claude-code": "claudecode-color",
  codex: "codex-color",
  "gemini-cli": "geminicli-color",
  opencode: "opencode",
  "qwen-code": "qwen-color",
  crush: "crush",
  "claude-desktop": "claude-color",
  antigravity: "antigravity-color",
  mimocode: "mimocode",
  deveco: "deveco-color",
  pi: "pi",
  stepcode: "stepfun-color",
  mothx: "mothx",
  penguin: "penguin-color",
  omo: "omo",
  "prime-agent": "prime-agent",
  mu: "mu",
  goose: "goose",
  vscode: "vscode",
  "vscode-insiders": "vscode",
  "copilot-jetbrains": "githubcopilot",
  "copilot-cli": "githubcopilot",
  reasonix: "reasonix-color",
  commandcode: "commandcode",
  fx: "fx",
  hermes: "hermes",
  morph: "mistermorph-color",
  "kimi-cli": "kimi",
  muse: "meta-color",
  empryo: "empryo-color",
  nanobot: "nanobot-color",
  miniq: "miniq-color",
  paimon: "paimon",
  ante: "generic",
  qwenpaw: "qwenpaw",
  openclaw: "openclaw",
  jcode: "jcode",
  vibe: "mistral-color",
  "minimax-code": "minimax-color",
  droid: "factory",
  qoder: "qoder",
  "qoder-cn": "qoder",
  grok: "xai",
  "grok-app": "grok",
  zcode: "zcode",
  workbuddy: "workbuddy-color",
  "workbuddy-ai": "workbuddy-color",
  codebuddy: "codebuddy-color",
  pencil: "generic",
  openpencil: "generic",
  "tuanjie-cowork": "tuanjie-cowork-color",
  liveagent: "liveagent-color",
  t3code: "t3code",
  atomcode: "atomcode",
  snow: "snow",
};

/** Whether a logo of this name is bundled. */
export const hasLogo = (icon: string | null | undefined): icon is LogoName => !!icon && (icon in LOGOS || icon in PICTURES);
/** A logo's name as the core gives it (`Preset.icon`), or magpie's cube for one there is none of. */
export const logoNamed = (icon: string | null | undefined): LogoName => (hasLogo(icon) ? icon : "generic");
/** The logo of a preset (`Provider.preset`), as magpie shows it. */
export const providerLogo = (preset: string | null | undefined): LogoName => logoNamed(presets().find((p) => p.id === preset)?.icon);
export const agentLogo = (id: string): LogoName => AGENT_LOGOS[id] ?? "generic";

/** A provider's or an agent's logo; one drawn in one color takes the theme's ink, an unknown one magpie's cube. */
export function Logo(props: { name: LogoName; size?: number }) {
  const size = () => props.size ?? 16;
  const src = () => {
    if (props.name === "generic") return svgSrc(ICONS.cube, token("muted-foreground"));
    if (props.name in PICTURES) return PICTURES[props.name as keyof typeof PICTURES];
    return svgSrc(LOGOS[props.name as keyof typeof LOGOS], token("foreground"));
  };
  return <Image src={src()} style={{ width: size(), height: size() }} />;
}

/** Logos overlapping in small discs: a group's members. */
export function LogoStack(props: { names: LogoName[]; size?: number }) {
  const size = () => props.size ?? 18;
  return (
    <View class="flex-row items-center">
      <Each of={props.names.slice(0, 3)} key={(n, i) => `${n}${i}`}>
        {(n, i) => (
          <View
            class="items-center justify-center rounded-full bg-card"
            style={{ width: size(), height: size(), marginLeft: i() === 0 ? 0 : -4, zIndex: 3 - i(), boxShadow: `0 0 0 1px ${token("border")}` }}
          >
            <Logo name={n()} size={Math.round(size() * 0.66)} />
          </View>
        )}
      </Each>
    </View>
  );
}

/** A hint shown while the pointer rests on `children`. */
export function Hint(props: { text: string; children: JSX.Element }) {
  return (
    <View
      tooltip={
        <Tooltip class="max-w-[320px] rounded-[7px] bg-popover px-[9px] py-[6px]" style={{ boxShadow: SHADOW.pop() }}>
          <Text class="text-[12px] text-foreground">{props.text}</Text>
        </Tooltip>
      }
    >
      {props.children}
    </View>
  );
}

/**
 * magpie's text button (`.text`): a word in the control ink, in ink on a
 * pill under the pointer, a little smaller while pressed. A `danger` one
 * keeps its red on a pale red pill, an `accent` one (Custom provider) its
 * accent on a pale accent one; a `link` is magpie's link (Get a key ↗):
 * accent words, underlined under the pointer, with no pill and no press.
 */
export function TextButton(props: {
  label: string;
  onPress: () => void;
  icon?: IconName;
  tone?: "default" | "accent" | "danger" | "muted" | "link";
  disabled?: boolean;
  testID?: string;
}) {
  const tone = () => props.tone ?? "default";
  const color = (): Color => ({ default: "ctl", accent: "primary", danger: "destructive", muted: "muted-foreground", link: "primary" } as const)[tone()];
  const ground = () => ({ default: "hover:bg-pill", accent: "hover:bg-accent", danger: "hover:bg-destructive-soft", muted: "hover:bg-pill", link: "" })[tone()];
  /** The ink the words and the icon take under the pointer: the plain tones' only. */
  const ink = (): Color | undefined => (props.disabled || (tone() !== "default" && tone() !== "muted") ? undefined : "foreground");
  const underline = () => !props.disabled && tone() === "link";
  return (
    <BaseButton
      onPress={props.onPress}
      disabled={props.disabled}
      testID={props.testID}
      class={`group h-[23px] flex-row items-center gap-1 rounded-[6px] px-[7px] ${ground()} ${tone() === "link" ? "" : "active:scale-96"} disabled:opacity-50 ${MOTION}`}
    >
      <If when={props.icon}>
        <Icon name={props.icon!} color={color()} hover={ink()} size={11} />
      </If>
      <Text class={`text-[12px] text-${color()} ${ink() ? `group-hover:text-${ink()}` : ""} ${underline() ? "underline-offset-2 group-hover:underline" : ""} ${MOTION}`}>{props.label}</Text>
    </BaseButton>
  );
}

/**
 * The one filled button of a place (magpie's `.text.primary`): Add, Save,
 * New group; `danger` fills it red, for a confirmation that removes or
 * undoes. A touch brighter under the pointer, a little smaller while pressed.
 */
export function PrimaryButton(props: { label: string; onPress: () => void; icon?: IconName; danger?: boolean; disabled?: boolean; testID?: string }) {
  const ink = (): Color => (props.danger ? "destructive-foreground" : "primary-foreground");
  return (
    <BaseButton
      onPress={props.onPress}
      disabled={props.disabled}
      testID={props.testID}
      class={`h-[23px] flex-row items-center gap-1 rounded-[6px] ${props.danger ? "bg-destructive" : "bg-primary"} px-[8px] hover:brightness-108 active:scale-96 disabled:opacity-40 ${MOTION}`}
    >
      <If when={props.icon}>
        <Icon name={props.icon!} color={ink()} size={11} />
      </If>
      <Text class={`text-[12px] font-medium text-${ink()}`}>{props.label}</Text>
    </BaseButton>
  );
}

/** An outlined pill button (magpie's `.ag-up`): Restart gateway; in ink on a pill under the pointer, a little smaller while pressed. */
export function PillButton(props: { label: string; onPress: () => void; icon?: IconName; disabled?: boolean; testID?: string }) {
  return (
    <BaseButton
      onPress={props.onPress}
      disabled={props.disabled}
      testID={props.testID}
      class={`group h-[26px] flex-row items-center gap-[5px] self-start rounded-full border border-border bg-card px-[10px] hover:bg-pill active:scale-96 disabled:opacity-50 ${MOTION}`}
    >
      <If when={props.icon}>
        <Icon name={props.icon!} color="muted-foreground" hover={props.disabled ? undefined : "foreground"} size={11} />
      </If>
      <Text class={`text-[12px] text-fg-2 ${props.disabled ? "" : "group-hover:text-foreground"} ${MOTION}`}>{props.label}</Text>
    </BaseButton>
  );
}

/** One turn of a refreshing icon, as magpie's (`.icon.spin`). */
const TURN = 900;

/**
 * magpie's icon button (`.icon`) of the title bar and the rows: a muted
 * icon, in ink on a pill under the pointer, a little smaller while pressed.
 * An `onPress` that returns a promise (a refresh) turns the icon until it
 * settles, to the end of the turn it is on, and takes no press meanwhile.
 */
export function IconButton(props: { icon: IconName; label: string; onPress: () => unknown; color?: Color; size?: number; disabled?: boolean; testID?: string }) {
  /** When the icon began turning; null while it is still. */
  const turning = signal<number | null>(null);
  const press = () => {
    if (turning() !== null) return;
    const done = props.onPress();
    if (!(done instanceof Promise)) return;
    const from = Date.now();
    turning.set(from);
    const stop = () => setTimeout(() => turning.set(null), TURN - ((Date.now() - from) % TURN));
    void done.then(stop, stop);
  };
  return (
    <Hint text={props.label}>
      <BaseButton
        onPress={press}
        disabled={props.disabled}
        accessibilityLabel={props.label}
        testID={props.testID}
        class={`group h-[26px] w-[26px] items-center justify-center rounded-[7px] hover:bg-pill active:scale-90 disabled:opacity-40 ${MOTION}`}
      >
        <View style={turning() === null ? undefined : { animationName: "spin", animationDuration: TURN, animationTimingFunction: "linear" }}>
          <Icon name={props.icon} color={props.color ?? "muted-foreground"} hover={props.disabled ? undefined : "foreground"} size={props.size ?? 14} />
        </View>
      </BaseButton>
    </Hint>
  );
}

/**
 * magpie's copy button: its icon, in ink on a pill under the pointer; a
 * copy that lands turns it a green tick for a moment.
 */
export function CopyButton(props: { label: string; onCopy: () => Promise<unknown>; testID?: string }) {
  const copied = signal(false);
  let back: ReturnType<typeof setTimeout> | undefined;
  const press = () =>
    void props.onCopy().then(() => {
      clearTimeout(back);
      copied.set(true);
      back = setTimeout(() => copied.set(false), 1200);
    });
  return (
    <Hint text={copied() ? tr("Copied") : tr("Copy")}>
      <BaseButton
        onPress={press}
        accessibilityLabel={props.label}
        testID={props.testID}
        class={`group h-[22px] w-[22px] items-center justify-center rounded-[5px] ${copied() ? "bg-success-soft" : "hover:bg-pill-hover"} ${MOTION}`}
      >
        <If when={copied()} else={<Icon name="copy-s" color="muted-foreground" hover="foreground" size={12} />}>
          <Icon name="tick" color="success" size={12} />
        </If>
      </BaseButton>
    </Hint>
  );
}

/** magpie's switch: a 32×19 track, ink when on, and a white thumb. */
export function Switch(props: { checked: boolean; onChange: (on: boolean) => void; label: string; disabled?: boolean; testID?: string }) {
  return (
    <BaseSwitch
      checked={props.checked}
      disabled={props.disabled}
      accessibilityLabel={props.label}
      testID={props.testID}
      hitSlop={6}
      onChange={({ checked }) => props.onChange(checked)}
      class="disabled:opacity-50"
      trackClass="h-[19px] w-8 flex-row items-center rounded-full bg-border p-[2px] transition-colors duration-150 checked:bg-switch-on"
      thumbClass="h-[15px] w-[15px] rounded-full bg-switch-thumb transition-transform duration-150 checked:translate-x-[13px] checked:bg-switch-on-thumb"
      thumbStyle={{ boxShadow: "0 1px 2px rgba(0,0,0,0.25)" }}
    />
  );
}

/** A checkbox as shadcn's: a small box, filled and ticked when checked, a dash over a partial selection. */
export function Check(props: { checked: boolean; indeterminate?: boolean; onChange: (on: boolean) => void; label: string; disabled?: boolean; testID?: string }) {
  const on = () => props.checked || !!props.indeterminate;
  return (
    <Checkbox
      checked={props.checked}
      indeterminate={props.indeterminate}
      disabled={props.disabled}
      onChange={(e) => props.onChange(e.checked)}
      accessibilityLabel={props.label}
      testID={props.testID}
      hitSlop={6}
      class="disabled:opacity-40"
      indicatorClass={`h-4 w-4 items-center justify-center rounded-[4px] border ${MOTION} ${on() ? "border-primary bg-primary" : "border-input bg-card"}`}
      indicator={
        <If when={on()}>
          <If when={props.indeterminate} else={<Icon name="tick" color="primary-foreground" size={11} />}>
            <View class="h-[1.5px] w-2 rounded-full bg-primary-foreground" />
          </If>
        </If>
      }
    />
  );
}

/** A segmented control (shadcn's toggle group): a pill track, the choice on a raised thumb. */
export function Segs<T extends string>(props: { options: { value: T; label: string }[]; value: T; onChange: (value: T) => void; testID?: string; small?: boolean }) {
  return (
    <ToggleGroup testID={props.testID} class="flex-row self-start rounded-[7px] bg-pill p-[2px]">
      <Each of={props.options} key={(o) => o.value}>
        {(option) => {
          const on = () => props.value === option().value;
          return (
            <Toggle
              pressed={on()}
              testID={`${props.testID ?? "segment"}-${option().value}`}
              onChange={() => props.onChange(option().value)}
              class={`${props.small ? "h-[20px] px-[8px]" : "h-[22px] px-[10px]"} items-center justify-center rounded-[5px] ${on() ? "bg-seg-thumb" : "hover:bg-pill-hover"}`}
              style={on() ? { boxShadow: SHADOW.thumb() } : undefined}
            >
              <Text class={`text-[12px] ${on() ? "text-foreground" : "text-muted-foreground"}`}>{option().label}</Text>
            </Toggle>
          );
        }}
      </Each>
    </ToggleGroup>
  );
}

/** magpie's Off / On: a setting's two-way segmented control. */
export function OnOff(props: { on: boolean; onChange: (on: boolean) => void; testID?: string; offLabel?: string; onLabel?: string }) {
  return (
    <Segs
      testID={props.testID}
      value={props.on ? "on" : "off"}
      onChange={(v) => props.onChange(v === "on")}
      options={[
        { value: "off", label: props.offLabel ?? tr("Off") },
        { value: "on", label: props.onLabel ?? tr("On") },
      ]}
    />
  );
}

/** A pop-up of choices (shadcn's select, as a menu): the choice in a pill, the others under it. */
export function Dropdown<T extends string>(props: { options: { value: T; label: string }[]; value: T; onChange: (value: T) => void; testID?: string; width?: number }) {
  const label = () => props.options.find((o) => o.value === props.value)?.label ?? props.value;
  return (
    <Menu
      placement="bottom"
      align="end"
      offset={4}
      trigger={<FieldButton value={label()} width={props.width ?? 150} testID={props.testID} />}
      popupClass="min-w-[160px] rounded-[10px] border border-border bg-popover p-1"
      popupStyle={{ boxShadow: SHADOW.pop() }}
    >
      <Each of={props.options} key={(o) => o.value}>
        {(o) => (
          <MenuItem
            role="menuitemradio"
            checked={props.value === o().value}
            testID={`${props.testID ?? "dropdown"}-${o().value}`}
            onSelect={() => props.onChange(o().value)}
            class="h-[28px] flex-row items-center gap-2 rounded-[7px] px-2 hover:bg-pill focus:bg-pill"
          >
            <View class="w-3">
              <If when={props.value === o().value}>
                <Icon name="tick" color="primary" size={12} />
              </If>
            </View>
            <Text class="text-[13px] text-foreground">{o().label}</Text>
          </MenuItem>
        )}
      </Each>
    </Menu>
  );
}

/** A small label in a pill: Pay as you go, 1.0M, via pansy. */
export function Badge(props: { label: string; tone?: "default" | "accent" | "success" | "warning" | "danger"; testID?: string }) {
  const look = () =>
    ({
      default: { box: "bg-pill border border-border", text: "text-muted-foreground" },
      accent: { box: "bg-accent", text: "text-primary" },
      success: { box: "bg-success-soft", text: "text-success" },
      warning: { box: "bg-warning-soft", text: "text-warning" },
      danger: { box: "bg-destructive-soft", text: "text-destructive" },
    })[props.tone ?? "default"];
  return (
    <View class={`h-[14px] flex-row items-center rounded-[4px] px-[4px] ${look().box}`}>
      <Text testID={props.testID} class={`text-[9px] font-medium ${look().text}`}>
        {props.label}
      </Text>
    </View>
  );
}

/** A rounded tag with a ring: how a group routes, a count. */
export function Tag(props: { label: string; chevron?: boolean; testID?: string }) {
  return (
    <View testID={props.testID} class="h-[18px] flex-row items-center gap-[3px] rounded-full border border-border pl-[8px] pr-[6px]">
      <Text class="text-[11px] text-muted-foreground">{props.label}</Text>
      <If when={props.chevron}>
        <Icon name="chev-down" color="muted-foreground" size={10} />
      </If>
    </View>
  );
}

/** An id in a code chip: group/coding. */
export function Code(props: { text: string; testID?: string; class?: string }) {
  return (
    <View class={`self-start rounded-[5px] bg-card-2 px-[5px] ${props.class ?? ""}`} style={{ boxShadow: `inset 0 0 0 1px ${token("line-2")}` }}>
      <Text testID={props.testID} class="font-mono text-[11px] font-medium text-fg-2">
        {props.text}
      </Text>
    </View>
  );
}

/** Monospaced text with no box: a key, a URL. */
export function Mono(props: { text: string; testID?: string; class?: string }) {
  return (
    <Text testID={props.testID} class={`font-mono text-[12px] text-foreground ${props.class ?? ""}`}>
      {props.text}
    </Text>
  );
}

/** A status dot: green on, amber to see to, grey off. */
export function Dot(props: { tone: "on" | "warn" | "off" | "error"; ring?: boolean }) {
  const color = () => ({ on: "bg-success", warn: "bg-warning", off: "bg-faint", error: "bg-destructive" })[props.tone];
  const ring = () => ({ on: "success-soft", warn: "warning-soft", off: "pill", error: "destructive-soft" } as const)[props.tone];
  return (
    <View
      class={`rounded-full ${color()} ${props.ring ? "h-[7px] w-[7px]" : "h-[6px] w-[6px]"}`}
      style={props.ring ? { boxShadow: `0 0 0 3px ${token(ring())}` } : undefined}
    />
  );
}

/** magpie's card of rows: white, a hairline edge, 12px corners. */
export function List(props: { children?: JSX.Element; testID?: string; class?: string }) {
  return (
    <View testID={props.testID} class={`overflow-hidden rounded-[12px] border border-border bg-card ${props.class ?? ""}`}>
      {props.children}
    </View>
  );
}

/** A row of a list, under a hairline unless it is the first. */
export function Row(props: { children?: JSX.Element; first?: boolean; testID?: string; class?: string; onPress?: () => void; onHover?: (over: boolean) => void }) {
  return (
    <View
      testID={props.testID}
      role={props.onPress ? "button" : undefined}
      onPress={props.onPress}
      onMouseEnter={props.onHover && (() => props.onHover?.(true))}
      onMouseLeave={props.onHover && (() => props.onHover?.(false))}
      class={`min-h-[40px] flex-row items-center gap-3 py-[7px] pl-[14px] pr-[12px] ${props.first ? "" : "border-t border-line-2"} ${props.class ?? ""}`}
    >
      {props.children}
    </View>
  );
}

/** A setting's row: its name (and a line under it) on the left, the control on the right, and what takes the row's width under them. */
export function SettingRow(props: { title: string; description?: string; children?: JSX.Element; below?: JSX.Element; first?: boolean; testID?: string; badge?: JSX.Element }) {
  return (
    <View testID={props.testID} class={`gap-[9px] py-[10px] pl-[14px] pr-[12px] ${props.first ? "" : "border-t border-line-2"}`}>
      <View class="min-h-[30px] flex-row items-center gap-3">
        <View class="flex-1 gap-[2px]">
          <View class="flex-row items-center gap-[6px]">
            <Text class="text-[13px] text-foreground">{props.title}</Text>
            {props.badge}
          </View>
          <If when={props.description}>
            <Text class="text-[12px] text-muted-foreground">{props.description ?? ""}</Text>
          </If>
        </View>
        {props.children}
      </View>
      {props.below}
    </View>
  );
}

/** The small capitals above a list, and what goes on its right. */
export function RowHead(props: { label?: string; note?: string; children?: JSX.Element; class?: string }) {
  return (
    <View class={`min-h-[26px] flex-row items-center gap-2 px-[2px] ${props.class ?? ""}`}>
      <If when={props.label}>
        <Label text={props.label ?? ""} />
      </If>
      <If when={props.note}>
        <Text class="text-[11px] text-faint">{props.note ?? ""}</Text>
      </If>
      <View class="flex-1" />
      {props.children}
    </View>
  );
}

export function Label(props: { text: string }) {
  return <Text class="text-[11px] font-medium uppercase tracking-[0.06em] text-muted-foreground">{props.text}</Text>;
}

/** A row's name and the muted line under it. */
export function Who(props: { name: string; sub?: string; children?: JSX.Element; nameTestID?: string; subTestID?: string }) {
  return (
    <View class="min-w-0 flex-1 gap-[1px]">
      <View class="flex-row items-center gap-[7px]">
        <Text testID={props.nameTestID} class="text-[13px] font-medium text-foreground line-clamp-1">
          {props.name}
        </Text>
        {props.children}
      </View>
      <If when={props.sub}>
        <Text testID={props.subTestID} class="text-[12px] text-muted-foreground line-clamp-1">
          {props.sub ?? ""}
        </Text>
      </If>
    </View>
  );
}

/** magpie's text field: white, a hairline edge, an accent ring while typing in it. */
export function Input(props: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  masked?: boolean;
  mono?: boolean;
  autoFocus?: boolean;
  testID?: string;
  label?: string;
  class?: string;
  onSubmit?: () => void;
  /** The field was left: what was typed is taken. */
  onBlur?: () => void;
  /** Several lines: the field grows with its text. */
  multiline?: boolean;
}) {
  const focused = signal(false);
  return (
    <BaseInput
      multiline={props.multiline}
      value={props.value}
      onChange={({ value }) => props.onChange(value)}
      onSubmit={() => props.onSubmit?.()}
      placeholder={props.placeholder}
      masked={props.masked}
      autoFocus={props.autoFocus}
      testID={props.testID}
      accessibilityLabel={props.label ?? props.placeholder}
      onFocus={() => focused.set(true)}
      onBlur={() => {
        focused.set(false);
        props.onBlur?.();
      }}
      class={`${props.multiline ? "min-h-[84px] py-[6px]" : "h-[26px]"} rounded-[7px] border bg-card px-[9px] text-[13px] text-foreground placeholder:text-faint ${props.mono ? "font-mono" : ""} ${focused() ? "border-primary" : "border-border"} ${props.class ?? ""}`}
      style={focused() ? { boxShadow: `0 0 0 3px ${token("accent")}` } : undefined}
    />
  );
}

/** A field that opens a pop-up: the model of an agent. */
export function FieldButton(props: { icon?: JSX.Element; value: string; empty?: boolean; open?: boolean; width?: number; testID?: string; label?: string }) {
  return (
    <View
      testID={props.testID}
      class={`h-[25px] flex-row items-center gap-[5px] rounded-[6px] pl-[8px] pr-[5px] hover:bg-pill-hover ${props.open ? "bg-pill-hover" : "bg-pill"}`}
      style={{ width: props.width ?? 196, boxShadow: props.open ? `inset 0 0 0 1px ${token("primary")}` : undefined }}
    >
      <If when={props.label}>
        <Text class="mr-[1px] text-[11px] text-faint">{props.label ?? ""}</Text>
      </If>
      {props.icon}
      <Text class={`flex-1 text-[13px] font-medium line-clamp-1 ${props.empty ? "text-muted-foreground" : "text-foreground"}`}>{props.value}</Text>
      <Icon name="chev-down" color="faint" size={11} />
    </View>
  );
}

/** A scrolling page under the title bar. */
export function Page(props: { children?: JSX.Element; testID?: string }) {
  return (
    <Scroll class="flex-1">
      <View testID={props.testID} class="gap-[10px] pb-6 pl-4 pr-4 pt-[2px]">
        {props.children}
      </View>
    </Scroll>
  );
}

/** A modal over the window: magpie's editor card, its head, its body (scrolling when taller than the window) and its foot. */
export function Modal(props: {
  title: string;
  icon?: JSX.Element;
  aside?: JSX.Element;
  onClose: () => void;
  children?: JSX.Element;
  foot?: JSX.Element;
  width?: number;
  testID?: string;
}) {
  return (
    <View class="absolute bottom-0 left-0 right-0 top-0 items-center justify-center bg-black/30 px-6 pb-6 pt-[46px]">
      <View testID={props.testID} style={{ width: props.width ?? 616, boxShadow: SHADOW.modal() }} class="max-h-full overflow-hidden rounded-[14px] border border-border bg-card">
        <View class="min-h-[50px] flex-row items-center gap-[9px] border-b border-line-2 px-[18px] py-3">
          {props.icon}
          <Text class="flex-1 text-[14px] font-semibold text-foreground line-clamp-1">{props.title}</Text>
          {props.aside}
        </View>
        <Scroll class="min-h-0 shrink">
          <View class="gap-[14px] px-[18px] py-4">{props.children}</View>
        </Scroll>
        <If when={props.foot}>
          <View class="flex-row items-center gap-2 border-t border-line-2 px-[18px] py-3">{props.foot}</View>
        </If>
      </View>
    </View>
  );
}

/** A row of a modal's body: the label column on the left, the field on the right. */
export function FormRow(props: { label: string; children?: JSX.Element; hint?: string; top?: boolean }) {
  return (
    <View class={`flex-row gap-[14px] ${props.top ? "items-start" : "items-center"}`}>
      <Text class={`w-[76px] text-right text-[12px] text-fg-2 ${props.top ? "pt-[5px]" : ""}`}>{props.label}</Text>
      <View class="min-w-0 flex-1 gap-[5px]">
        {props.children}
        <If when={props.hint}>
          <Text class="text-[12px] text-muted-foreground">{props.hint ?? ""}</Text>
        </If>
      </View>
    </View>
  );
}

/** Nothing here yet: a sentence in a dashed box, and what to do about it. */
export function Empty(props: { title: string; description?: string; action?: JSX.Element; testID?: string }) {
  return (
    <View class="items-center gap-2 rounded-[12px] border border-dashed border-border px-6 py-8">
      <Text testID={props.testID} class="text-[13px] font-medium text-foreground">
        {props.title}
      </Text>
      <If when={props.description}>
        <Text class="text-center text-[12px] text-muted-foreground">{props.description ?? ""}</Text>
      </If>
      {props.action}
    </View>
  );
}
