// One agent's row, as magpie draws it: its logo, which is its handle (a
// press, or a right click on the row, opens Move up, Move down, Disconnect
// and Hide; dragged, it moves the row among the others), its name, its
// version and the update that is out, whether it is connected, Details for a
// connected one, the model it starts on and its switch. Opened, it says
// where it picks its models, Claude Code's tiers (each its model and its
// effort, the subagents' and auto mode's), the models it lists, a new
// session's effort, and what pansy changed in its files, with Disconnect and
// restore.
import { PanResponder, signal, Text, token, View, type JSX } from "nsh";
import { ContextMenu, Menu, MenuItem, MenuSeparator, Popover, Slider } from "nsh/base";
import { Agents } from "pansy/native";
import { ChangesList } from "./AgentModals";
import {
  arrange,
  askDisconnect,
  changes,
  cliOf,
  connect,
  dragEnd,
  dragging,
  dragMove,
  dragStart,
  effortName,
  efforting,
  expanded,
  fieldName,
  fieldOf,
  hasTiers,
  isHidden,
  kindOf,
  loadChanges,
  moveAgent,
  picking,
  plainFields,
  rowHeight,
  setField,
  setHidden,
  shiftOf,
  status,
  switchOn,
  TIERS,
  updateCli,
  updating,
  viewing,
} from "./agentState";
import { tr } from "./i18n";
import { act, choiceName, choices, go, isBusy, loadAgents, providerOf, providers, type Agent } from "./model";
import { ChoiceLogo, ModelPicker } from "./Picker";
import { openPreview } from "./Preview";
import { adding } from "./Providers";
import { agentLogo, Dot, EffortBars, effortLit, FieldButton, Hint, Icon, Logo, LogoStack, MOTION, providerLogo, SHADOW, Switch, type IconName } from "./parts";

const MENU = "min-w-[200px] rounded-[10px] border border-border bg-popover p-1";

function Item(props: { icon: IconName; label: string; onSelect: () => void; disabled?: boolean; testID?: string }) {
  return (
    <MenuItem
      disabled={props.disabled}
      testID={props.testID}
      onSelect={props.onSelect}
      class="h-[28px] flex-row items-center gap-[9px] rounded-[7px] px-2 hover:bg-pill focus:bg-pill disabled:opacity-40"
    >
      <Icon name={props.icon} color="fg-2" size={13} />
      <Text class="text-[12.5px] text-foreground">{props.label}</Text>
    </MenuItem>
  );
}

/** The row's menu: a hidden one's Show, a folded one's Hide, else the rest. */
function AgentMenu(props: { agent: Agent; inFold: boolean }) {
  const a = () => props.agent;
  const i = () => arrange().shown.findIndex((x) => x.id === a().id);
  return (
    <If
      when={!props.inFold}
      else={
        <If when={isHidden(a())} else={<Item icon="eye-off-s" label={tr("Hide")} testID={`menu-hide-${a().id}`} onSelect={() => setHidden(a(), true)} />}>
          <Item icon="eye-s" label={tr("Show")} testID={`menu-show-${a().id}`} onSelect={() => setHidden(a(), false)} />
        </If>
      }
    >
      <Item icon="up" label={tr("Move up")} disabled={i() <= 0} testID={`menu-up-${a().id}`} onSelect={() => moveAgent(a().id, i() - 1)} />
      <Item icon="down" label={tr("Move down")} disabled={i() < 0 || i() >= arrange().shown.length - 1} testID={`menu-down-${a().id}`} onSelect={() => moveAgent(a().id, i() + 1)} />
      <If when={a().connected}>
        <MenuSeparator class="mx-1 my-1 h-px bg-line-2" />
        <Item icon="unplug" label={tr("Disconnect from pansy")} testID={`menu-disconnect-${a().id}`} onSelect={() => askDisconnect(a())} />
      </If>
      <MenuSeparator class="mx-1 my-1 h-px bg-line-2" />
      <Item icon="eye-off-s" label={tr("Hide")} testID={`menu-hide-${a().id}`} onSelect={() => setHidden(a(), true)} />
    </If>
  );
}

/** Which of an agent's subagents go on the model set for them. */
const subagentsSaid = (a: Agent) => (a.subagentEvery ? tr("every subagent, whatever model its lead names") : tr("a subagent that names no model"));

/**
 * One of an agent's models, picked in place: the one it starts on, the one
 * for its quick tasks (`small`), its subagents' (`subagent`), or a field's
 * (`field`: a tier's, auto mode's). `label` names it inside the button.
 */
function ModelField(props: { agent: Agent; small?: boolean; subagent?: boolean; field?: string; width?: number; label?: string }) {
  const a = () => props.agent;
  const field = () => (props.field ? fieldOf(a(), props.field) : undefined);
  const model = () => (props.field ? (field()?.value ?? null) : props.subagent ? a().subagentModel : props.small ? a().smallModel : a().model);
  const open = () => picking()?.id === a().id && picking()?.small === !!props.small && !!picking()?.subagent === !!props.subagent && picking()?.field === props.field;
  const follows = () => (props.field ? !!field()?.follows : !!props.small);
  const reset = () =>
    props.subagent
      ? { label: tr("Left to %s", a().name), note: subagentsSaid(a()) }
      : props.field === "auto_mode"
        ? { label: tr("Left to %s", a().name), note: tr("the model that checks each action in auto mode") }
        : follows()
          ? { label: tr("The same as the main model"), note: props.small && !hasTiers(a()) ? tr("titles, summaries and other small requests") : "" }
          : props.field
            ? { label: tr("Left to %s", a().name), note: "" }
            : undefined;
  const unset = () => (props.field === "auto_mode" ? tr("default") : follows() ? (hasTiers(a()) ? tr("same as model") : tr("same as the main model")) : props.field || props.subagent ? tr("left to %s", a().name) : tr("default"));
  return (
    <ModelPicker
      open={open()}
      onOpenChange={(o) => picking.set(o ? { id: a().id, small: !!props.small, subagent: !!props.subagent, field: props.field } : null)}
      current={a().connected ? model() : null}
      reset={reset()}
      align={props.width ? "start" : "end"}
      onPick={(id) => {
        // A key more or less in its files: what pansy changed is read again.
        if (props.field) void setField(a(), props.field, id);
        else if (props.subagent) void act(`agent:${a().id}`, () => Agents.setSubagentModel(a().id, id)).then(() => (loadAgents(), loadChanges(a().id)));
        else if (props.small) void act(`agent:${a().id}`, () => Agents.setSmallModel(a().id, id)).then(() => (loadAgents(), loadChanges(a().id)));
        else connect(a(), id);
      }}
      trigger={
        <FieldButton
          testID={props.field ? `field-${a().id}-${props.field}` : props.subagent ? `subagent-${a().id}` : props.small ? `small-${a().id}` : `model-${a().id}`}
          open={open()}
          width={props.width}
          label={props.label}
          empty={!a().connected || !model()}
          value={a().connected && model() ? choiceName(model()) : unset()}
          icon={a().connected && model() ? <ChoiceLogo id={model()!} size={14} /> : <Logo name={agentLogo(a().id)} size={14} />}
        />
      }
    />
  );
}

/**
 * An effort, picked as magpie's is: its bars and its level on a button, and
 * under it a slider whose first stop is the agent's own and whose others are
 * the levels, lowest first. `label` names it inside the button; `needs`
 * says what it waits for, and nothing is picked till then.
 */
function EffortField(props: { agent: Agent; name: string; label?: boolean; needs?: string }) {
  const a = () => props.agent;
  const levels = () => fieldOf(a(), props.name)?.levels ?? [];
  const value = () => (a().connected && !props.needs ? (fieldOf(a(), props.name)?.value ?? null) : null);
  const open = () => efforting()?.id === a().id && efforting()?.key === props.name;
  const stops = (): (string | null)[] => [null, ...levels()];
  /** The stop the slider is at while it is dragged; null: the field's own. */
  const dragged = signal<number | null>(null);
  const at = () => dragged() ?? Math.max(0, stops().indexOf(value()));
  // Let go at a stop, the effort is that one, and the slider is put away.
  const commit = (i: number) => {
    const level = stops()[i] ?? null;
    dragged.set(null);
    efforting.set(null);
    if (level !== value()) void setField(a(), props.name, level ?? "");
  };
  const button = (
    <View
      testID={`effort-${a().id}-${props.name}`}
      accessibilityLabel={tr("%s: %s", fieldName(props.name), effortName(value()))}
      class={`h-[25px] flex-row items-center gap-[5px] rounded-[6px] pl-[7px] pr-[5px] ${props.needs ? "bg-pill opacity-50" : open() ? "bg-pill-hover" : "bg-pill hover:bg-pill-hover"}`}
      style={{ boxShadow: open() ? `inset 0 0 0 1px ${token("primary")}` : undefined }}
    >
      <If when={props.label}>
        <Text class="mr-[1px] text-[11px] text-faint">{fieldName(props.name)}</Text>
      </If>
      <EffortBars lit={effortLit(levels(), value(), 4)} color={value() ? "foreground" : "muted-foreground"} />
      <Text class={`text-[12.5px] font-medium ${value() ? "text-foreground" : "text-muted-foreground"}`}>{effortName(value())}</Text>
      <Icon name="chev-down" color="faint" size={11} />
    </View>
  );
  return (
    <If when={!props.needs} else={<Hint text={props.needs ?? ""}>{button}</Hint>}>
      <Popover
        open={open()}
        onOpenChange={({ open: o }) => {
          dragged.set(null);
          efforting.set(o ? { id: a().id, key: props.name } : null);
        }}
        placement="bottom"
        align="start"
        offset={4}
        margin={8}
        trigger={button}
        popupClass="rounded-[11px] border border-border bg-popover"
        popupStyle={{ boxShadow: SHADOW.pop() }}
      >
        <View testID="effort-picker" class="w-[232px] px-4 pb-[11px] pt-[13px]">
          <View class="flex-row items-center">
            <Text class="flex-1 text-[12px] text-muted-foreground">{fieldName(props.name)}</Text>
            <Text testID="effort-value" class="text-[12.5px] font-semibold text-foreground">
              {effortName(stops()[at()] ?? null)}
            </Text>
          </View>
          <Slider
            value={at()}
            min={0}
            max={stops().length - 1}
            step={1}
            testID="effort-slider"
            accessibilityLabel={fieldName(props.name)}
            onChange={({ value: v }) => dragged.set(Array.isArray(v) ? v[0] : v)}
            onChangeEnd={({ value: v }) => commit(Array.isArray(v) ? v[0] : v)}
            class="mt-[6px]"
            trackClass="h-6 justify-center"
            indicatorClass="h-[3px] rounded-full bg-pill-hover"
            rangeClass="rounded-full bg-foreground"
            thumbClass="size-[18px] rounded-full border border-border bg-card"
            thumbStyle={{ boxShadow: SHADOW.pop() }}
          />
          <View class="mt-[2px] flex-row items-center">
            <Text role="button" testID="effort-default" onPress={() => commit(0)} class="text-[11px] text-muted-foreground hover:text-foreground">
              {tr("default")}
            </Text>
            <View class="flex-1" />
            <Text role="button" testID="effort-top" onPress={() => commit(stops().length - 1)} class="text-[11px] text-muted-foreground hover:text-foreground">
              {effortName(levels()[levels().length - 1] ?? null)}
            </Text>
          </View>
        </View>
      </Popover>
    </If>
  );
}

/** magpie's quiet button of an opened row: Details, View, Disconnect and restore; in ink on a pill under the pointer, a red one on a red wash. */
function Quiet(props: { label: string; onPress: () => void; chevron?: "right" | "down"; open?: boolean; danger?: boolean; testID?: string }) {
  return (
    <View
      role="button"
      testID={props.testID}
      onPress={props.onPress}
      class={`group h-6 flex-row items-center gap-1 rounded-[6px] px-2 ${props.open ? "bg-pill" : props.danger ? "hover:bg-destructive/10" : "hover:bg-pill"} ${MOTION}`}
    >
      <Text class={`text-[12px] ${props.danger ? "text-destructive" : props.open ? "text-foreground" : "text-fg-2 group-hover:text-foreground"} ${MOTION}`}>{props.label}</Text>
      <If when={props.chevron}>
        <Icon name={props.chevron === "down" ? "chev-down" : "chev-right"} color="faint" size={10} />
      </If>
    </View>
  );
}

/** Text with `code` set as code where `%s` stands in `said`. */
function WithCode(props: { said: string; code: string }) {
  const parts = () => props.said.split("\u0001");
  return (
    <Text class="text-[12.5px] leading-[24px] text-fg-2">
      {parts()[0]}
      <Text class="font-mono text-[11.5px] text-foreground">{props.code}</Text>
      {parts()[1] ?? ""}
    </Text>
  );
}

function Kv(props: { label: string; first?: boolean; children?: JSX.Element; testID?: string }) {
  return (
    <View testID={props.testID} class={`flex-row ${props.first ? "" : "border-t border-line-2"}`}>
      <Text class="w-[128px] shrink-0 pb-[11px] pr-4 pt-[14px] text-[12px] text-muted-foreground line-clamp-1">{props.label}</Text>
      <View class="min-w-0 flex-1 justify-center gap-1 py-[11px]">{props.children}</View>
    </View>
  );
}

/** One line of the tiers: what it is for, its model, its effort, and how it is picked in the agent. */
function TierLine(props: { label: string; hint?: string; children?: JSX.Element; testID?: string }) {
  return (
    <View testID={props.testID} class="min-h-[25px] flex-row items-center gap-2">
      <Text class="w-[70px] shrink-0 text-[12px] text-fg-2 line-clamp-1">{props.label}</Text>
      {props.children}
      <Text class="min-w-0 flex-1 text-[12px] text-muted-foreground line-clamp-1">{props.hint ?? ""}</Text>
    </View>
  );
}

const PICK = 220;

/** Claude Code's tiers, each with the `/model` that picks it; then its subagents and its auto mode. */
function Tiers(props: { agent: Agent }) {
  const a = () => props.agent;
  const also = (tier: string) => (tier === "sonnet" ? ` · ${tr("the default tier")}` : tier === "haiku" ? ` · ${tr("also for small background tasks")}` : "");
  const tiers = () => TIERS.filter((t) => (t === "haiku" ? a().hasSmallModel : !!fieldOf(a(), t)));
  return (
    <Kv label={tr("Tiers")} testID={`tiers-${a().id}`}>
      <Each of={tiers()} key={(t) => t}>
        {(t) => (
          <TierLine label={t()[0].toUpperCase() + t().slice(1)} hint={`/model ${t()}${also(t())}`}>
            <If when={t() === "haiku"} else={<ModelField agent={a()} field={t()} width={PICK} />}>
              <ModelField agent={a()} small width={PICK} />
            </If>
            <If when={fieldOf(a(), `${t()}_effort`)}>
              <EffortField agent={a()} name={`${t()}_effort`} />
            </If>
          </TierLine>
        )}
      </Each>
      <If when={a().hasSubagentModel}>
        <TierLine label={tr("Subagents")} hint={subagentsSaid(a())} testID={`subagents-${a().id}`}>
          <ModelField agent={a()} subagent width={PICK} />
          <If when={fieldOf(a(), "subagent_effort")}>
            <EffortField agent={a()} name="subagent_effort" needs={a().subagentModel ? undefined : tr("Set a subagent model first")} />
          </If>
        </TierLine>
      </If>
      <If when={fieldOf(a(), "auto_mode")}>
        <TierLine label={tr("Auto mode")} hint={tr("the model that checks each action in auto mode")}>
          <ModelField agent={a()} field="auto_mode" width={PICK} />
        </TierLine>
      </If>
    </Kv>
  );
}

/** Whose pansy's models are, a chip each: the routing groups, then each provider with how many it has. */
function ModelChips() {
  const chips = () => {
    const out: { key: string; name: string; n: number; logos: ReturnType<typeof providerLogo>[] }[] = [];
    const inGroups = choices().filter((c) => c.group);
    if (inGroups.length > 0) out.push({ key: "groups", name: tr("Routing groups"), n: inGroups.length, logos: [] });
    for (const p of providers()) {
      const n = choices().filter((c) => !c.group && providerOf(c.id)?.id === p.id).length;
      if (n > 0) out.push({ key: p.id, name: p.name, n, logos: [providerLogo(p.preset)] });
    }
    return out;
  };
  return (
    <View class="min-h-[24px] flex-row flex-wrap items-center gap-x-2 gap-y-[6px]">
      <Each of={chips()} key={(c) => c.key}>
        {(c) => (
          <View class="h-[22px] flex-row items-center gap-[5px] rounded-[6px] border border-border bg-card px-2">
            <If when={c().logos.length > 0} else={<Icon name="nav-gateway" color="muted-foreground" size={13} />}>
              <LogoStack names={c().logos} size={13} />
            </If>
            <Text class="text-[12px] text-fg-2 line-clamp-1">{c().name}</Text>
            <Text class="text-[12px] text-muted-foreground">{String(c().n)}</Text>
          </View>
        )}
      </Each>
    </View>
  );
}

/** A connected agent's row, opened. */
function Details(props: { agent: Agent }) {
  const a = () => props.agent;
  const ch = () => changes()[a().id] ?? [];
  const shown = () => viewing()[a().id] === true;
  const files = () => ch().map((f, i) => (i === 0 ? f.path : (f.path.split(/[\\/]/).pop() ?? f.path)));
  return (
    <View testID={`agent-details-${a().id}`} class="border-t border-line-2 bg-card-2 pb-[14px] pl-[50px] pr-4 pt-1">
      <Kv first label={tr("In %s", a().name)}>
        <If
          when={a().picksIn}
          else={
            <If when={hasTiers(a())} else={<Text class="text-[12.5px] leading-[24px] text-fg-2">{tr("Reopen %s: it starts on the model picked here.", a().name)}</Text>}>
              <WithCode said={tr("New sessions take it; switch the tiers in %s.", "\u0001")} code="/model" />
            </If>
          }
        >
          <WithCode said={tr("Pick any model in %s's %s.", a().name, "\u0001")} code={a().picksIn ?? ""} />
        </If>
      </Kv>
      <If when={hasTiers(a())}>
        <Tiers agent={a()} />
      </If>
      <If when={!hasTiers(a()) && a().hasSmallModel}>
        <Kv label={tr("Quick tasks")}>
          <View class="flex-row items-center gap-2">
            <ModelField agent={a()} small width={PICK} />
            <Text class="flex-1 text-[12px] text-muted-foreground line-clamp-1">{tr("titles, summaries and other small requests")}</Text>
          </View>
        </Kv>
      </If>
      <If when={!hasTiers(a()) && a().hasSubagentModel}>
        <Kv label={tr("Subagents")} testID={`subagents-${a().id}`}>
          <View class="flex-row items-center gap-2">
            <ModelField agent={a()} subagent width={PICK} />
            <Text class="flex-1 text-[12px] text-muted-foreground line-clamp-1">{subagentsSaid(a())}</Text>
          </View>
        </Kv>
      </If>
      <If when={a().picksIn}>
        <Kv label={tr("Model list")} testID={`model-list-${a().id}`}>
          <ModelChips />
          <Text class="text-[12px] leading-[18px] text-muted-foreground">{tr("All %d models of pansy, under the Pansy provider", choices().length)}</Text>
        </Kv>
      </If>
      <If when={plainFields(a()).length > 0}>
        <Kv label={tr("New sessions")} testID={`new-sessions-${a().id}`}>
          <View class="flex-row flex-wrap items-center gap-2">
            <Each of={plainFields(a())} key={(f) => f.key}>
              {(f) => (
                <If when={f().kind === "effort"} else={<ModelField agent={a()} field={f().key} width={PICK} label={fieldName(f().key)} />}>
                  <EffortField agent={a()} name={f().key} label />
                </If>
              )}
            </Each>
          </View>
        </Kv>
      </If>
      <View class="mt-[2px] flex-row items-center gap-2 border-t border-line-2 pt-3">
        <If when={ch().length > 0}>
          <Text class="min-w-0 shrink text-[12px] text-muted-foreground line-clamp-1">
            {tr("Changed ")}
            <Each of={files()} key={(f, i) => `${i}${f}`}>
              {(f, i) => (
                <>
                  {i() === 0 ? "" : " · "}
                  <Text class="font-mono text-[11.5px] text-fg-2">{f()}</Text>
                </>
              )}
            </Each>
          </Text>
          <Quiet
            label={shown() ? tr("Hide changes") : tr("View")}
            chevron={shown() ? "down" : "right"}
            open={shown()}
            testID={`changes-${a().id}`}
            onPress={() => viewing.update((v) => ({ ...v, [a().id]: !shown() }))}
          />
        </If>
        <View class="flex-1" />
        <Quiet label={tr("Disconnect and restore")} danger testID={`disconnect-${a().id}`} onPress={() => askDisconnect(a())} />
      </View>
      <If when={shown() && ch().length > 0}>
        <View class="mt-[10px]">
          <ChangesList changes={ch()} />
        </View>
      </If>
    </View>
  );
}

/** The agent's program beside its name: that it is gone, or its version and the update that is out. */
function CliTag(props: { agent: Agent }) {
  const a = () => props.agent;
  const cli = () => cliOf(a());
  const busy = () => updating().includes(a().id);
  const said = () =>
    !cli()?.via
      ? tr("%s %s · pansy can't tell how it was installed — update it the way you installed it", a().name, cli()?.version ?? "")
      : cli()?.update
        ? tr("%s %s is installed · %s is out", a().name, cli()?.version ?? "", cli()?.latest ?? "")
        : tr("%s %s · up to date", a().name, cli()?.version ?? "");
  return (
    <>
      <If when={a().cliMissing}>
        <Hint text={tr("%s's settings are still here (%s), but its command-line program isn't found: it may have been uninstalled. Install another agent, below the list, has its install command.", a().name, a().configFiles[0] ?? "")}>
          <Text testID={`cli-missing-${a().id}`} class="text-[11.5px] text-muted-foreground">
            {tr("CLI not found")}
          </Text>
        </Hint>
      </If>
      <If when={!a().cliMissing && cli()?.version}>
        <Hint text={said()}>
          <Text testID={`version-${a().id}`} class="text-[11.5px] text-muted-foreground">
            {cli()?.version ?? ""}
          </Text>
        </Hint>
      </If>
      <If when={!a().cliMissing && (cli()?.update || busy())}>
        <Hint text={tr("Updates with %s", cli()?.command ?? "")}>
          <View
            role="button"
            testID={`update-${a().id}`}
            accessibilityLabel={busy() ? tr("Updating…") : tr("Update to %s", cli()?.latest ?? "")}
            disabled={busy()}
            onPress={() => void updateCli(a())}
            class={`group h-5 flex-row items-center gap-1 rounded-full border border-border bg-card pl-[6px] pr-2 hover:bg-pill active:scale-96 ${MOTION}`}
          >
            <Icon name={busy() ? "sync" : "bottom"} color="muted-foreground" size={11} />
            <Text class={`text-[11.5px] font-medium ${busy() ? "text-muted-foreground" : "text-fg-2 group-hover:text-foreground"} ${MOTION}`}>
              {busy() ? tr("Updating…") : tr("Update to %s", cli()?.latest ?? "")}
            </Text>
          </View>
        </Hint>
      </If>
    </>
  );
}

/** One agent's row; `inFold`, under the list's fold; `first`, no line above it. */
export function AgentRow(props: { agent: Agent; first: boolean; inFold?: boolean }) {
  const a = () => props.agent;
  const kind = () => kindOf(a());
  const st = () => status(a());
  const open = () => expanded() === a().id && a().connected;
  const putAway = () => !!props.inFold && isHidden(a());
  const toggle = () => {
    const next = open() ? null : a().id;
    expanded.set(next);
    if (next) loadChanges(next);
  };
  // The logo is the row's grip: dragged up or down, the row goes with it
  // and the others make room; a press alone still opens its menu.
  const grip = PanResponder.create({
    onMoveShouldSetPanResponder: (_e, g) => !props.inFold && Math.abs(g.dy) > 4 && Math.abs(g.dy) > Math.abs(g.dx),
    onPanResponderGrant: () => dragStart(a().id),
    onPanResponderMove: (_e, g) => dragMove(g.dy),
    onPanResponderRelease: () => dragEnd(true),
    onPanResponderTerminate: () => dragEnd(false),
  });
  const held = () => dragging()?.id === a().id;
  const line = (
    <View class="min-h-[56px] flex-row items-center gap-3 py-[7px] pl-[14px] pr-[12px]">
      <Menu
        placement="bottom"
        align="start"
        offset={4}
        trigger={
          <View
            {...grip.panHandlers}
            testID={`handle-${a().id}`}
            accessibilityLabel={tr("%s's menu", a().name)}
            class={`-m-[3px] h-7 w-7 items-center justify-center rounded-[6px] hover:bg-pill aria-expanded:bg-pill ${MOTION}`}
            style={{ opacity: putAway() ? 0.6 : 1 }}
          >
            <Logo name={agentLogo(a().id)} size={22} />
          </View>
        }
        popupClass={MENU}
        popupStyle={{ boxShadow: SHADOW.pop() }}
      >
        <AgentMenu agent={a()} inFold={!!props.inFold} />
      </Menu>
      <View class="min-w-0 flex-1">
        <View class="h-[18px] flex-row items-center gap-[7px]">
          <Text class="text-[13px] font-medium leading-[18px] text-foreground line-clamp-1" style={{ opacity: putAway() ? 0.6 : 1 }}>
            {a().name}
          </Text>
          <CliTag agent={a()} />
          <If when={putAway()}>
            <Hint text={tr("Hidden by you · show it in the list again")}>
              <View
                role="button"
                testID={`show-${a().id}`}
                onPress={() => setHidden(a(), false)}
                class={`group h-[22px] flex-row items-center gap-1 rounded-full border border-border bg-card px-2 hover:bg-pill ${MOTION}`}
              >
                <Icon name="eye-s" color="muted-foreground" hover="foreground" size={12} />
                <Text class={`text-[11.5px] text-fg-2 group-hover:text-foreground ${MOTION}`}>{tr("Show")}</Text>
              </View>
            </Hint>
          </If>
        </View>
        <If when={kind() !== "gone"}>
          <View class="h-[18px] flex-row items-center gap-[6px]">
            <Dot tone={st().tone} />
            <Text testID={`status-${a().id}`} class="flex-1 text-[12px] leading-[18px] text-muted-foreground line-clamp-1">
              {st().text}
            </Text>
          </View>
        </If>
      </View>
      <If when={kind() === "ok"}>
        <If when={!a().connected}>
          <Quiet label={tr("What it changes")} testID={`preview-${a().id}`} onPress={() => openPreview(a())} />
        </If>
        <If when={a().connected}>
          <Quiet label={open() ? tr("Collapse") : tr("Details")} chevron={open() ? undefined : "right"} open={open()} testID={`details-${a().id}`} onPress={toggle} />
        </If>
        <View class="ml-2">
          <ModelField agent={a()} />
        </View>
      </If>
      <If when={kind() === "empty"}>
        <View
          role="button"
          testID={`add-provider-${a().id}`}
          onPress={() => {
            adding.set(true);
            go("providers");
          }}
          class="h-6 items-center justify-center rounded-[6px] bg-pill px-[10px] hover:bg-pill-hover"
        >
          <Text class="text-[12px] font-medium text-foreground">{tr("Add a provider")}</Text>
        </View>
      </If>
      <If when={kind() !== "gone" || a().connected}>
        <View class="ml-1" style={{ opacity: kind() === "empty" ? 0.4 : 1 }}>
          <Switch
            checked={a().connected}
            label={tr("Connect %s to pansy", a().name)}
            testID={`connect-${a().id}`}
            disabled={isBusy(`agent:${a().id}`) || kind() === "empty"}
            onChange={(on) => (on ? switchOn(a()) : askDisconnect(a()))}
          />
        </View>
      </If>
    </View>
  );
  return (
    <View
      testID={`agent-${a().id}`}
      onLayout={({ bounds }) => rowHeight(a().id, bounds.size.height)}
      class={`${props.first ? "" : "border-t border-line-2"} ${held() ? "bg-card" : dragging() ? MOTION : ""}`}
      style={{
        transform: [{ translateY: props.inFold ? 0 : shiftOf(a().id) }],
        zIndex: held() ? 2 : 0,
        boxShadow: held() ? SHADOW.pop() : undefined,
      }}
    >
      <ContextMenu trigger={line} popupClass={MENU} popupStyle={{ boxShadow: SHADOW.pop() }}>
        <AgentMenu agent={a()} inFold={!!props.inFold} />
      </ContextMenu>
      <If when={open()}>
        <Details agent={a()} />
      </If>
    </View>
  );
}

/** Where rows will be, while the agents are looked for. */
export function SkeletonRow(props: { first: boolean; name: number }) {
  return (
    <View class={`h-[56px] flex-row items-center gap-3 pl-[14px] pr-[12px] ${props.first ? "" : "border-t border-line-2"}`}>
      <View class="h-[22px] w-[22px] rounded-[6px] bg-pill" />
      <View class="flex-1">
        <View class="h-[11px] rounded-[4px] bg-pill" style={{ width: props.name }} />
      </View>
      <View class="h-[25px] w-[196px] rounded-[6px] bg-pill" />
      <View class="h-[19px] w-8 rounded-full bg-pill" />
    </View>
  );
}
