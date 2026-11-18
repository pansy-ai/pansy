// One agent's row, as magpie draws it: its logo, which is its handle (a
// press, or a right click on the row, opens Move up, Move down, Disconnect
// and Hide), its name and version, whether it is connected, Details for a
// connected one, the model it starts on and its switch. Opened, it says
// where it picks its models, its quick-task model, and what pansy changed
// in its files, with Disconnect and restore.
import { Text, View, type JSX } from "nsh";
import { ContextMenu, Menu, MenuItem, MenuSeparator } from "nsh/base";
import { Agents } from "pansy/native";
import { ChangesList } from "./AgentModals";
import { arrange, askDisconnect, changes, connect, expanded, isHidden, kindOf, loadChanges, moveAgent, picking, setHidden, status, switchOn, viewing } from "./agentState";
import { tr } from "./i18n";
import { act, choiceName, choices, go, isBusy, loadAgents, versions, type Agent } from "./model";
import { ChoiceLogo, ModelPicker } from "./Picker";
import { openPreview } from "./Preview";
import { adding } from "./Providers";
import { agentLogo, Dot, FieldButton, Hint, Icon, Logo, MOTION, SHADOW, Switch, type IconName } from "./parts";

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

function ModelField(props: { agent: Agent; small?: boolean }) {
  const a = () => props.agent;
  const model = () => (props.small ? a().smallModel : a().model);
  const open = () => picking()?.id === a().id && picking()?.small === !!props.small;
  return (
    <ModelPicker
      open={open()}
      onOpenChange={(o) => picking.set(o ? { id: a().id, small: !!props.small } : null)}
      current={a().connected ? model() : null}
      reset={props.small ? { label: tr("The same as the main model"), note: tr("titles, summaries and other small requests") } : undefined}
      onPick={(id) => {
        if (props.small) void act(`agent:${a().id}`, () => Agents.setSmallModel(a().id, id)).then(() => loadAgents());
        else connect(a(), id);
      }}
      trigger={
        <FieldButton
          testID={props.small ? `small-${a().id}` : `model-${a().id}`}
          open={open()}
          empty={!a().connected || !model()}
          value={a().connected && model() ? choiceName(model()) : props.small ? tr("same as the main model") : tr("default")}
          icon={a().connected && model() ? <ChoiceLogo id={model()!} size={14} /> : <Logo name={agentLogo(a().id)} size={14} />}
        />
      }
    />
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
            <If when={a().id === "claude-code"} else={<Text class="text-[12.5px] leading-[24px] text-fg-2">{tr("Reopen %s: it starts on the model picked here.", a().name)}</Text>}>
              <WithCode said={tr("New sessions take it; switch the tiers in %s.", "\u0001")} code="/model" />
            </If>
          }
        >
          <WithCode said={tr("Pick any model in %s's %s.", a().name, "\u0001")} code={a().picksIn ?? ""} />
        </If>
      </Kv>
      <If when={a().hasSmallModel}>
        <Kv label={tr("Quick tasks")}>
          <View class="flex-row items-center gap-2">
            <ModelField agent={a()} small />
            <Text class="flex-1 text-[11.5px] text-muted-foreground line-clamp-1">{tr("titles, summaries and other small requests")}</Text>
          </View>
        </Kv>
      </If>
      <If when={a().picksIn}>
        <Kv label={tr("Model list")}>
          <Text class="text-[12.5px] leading-[24px] text-fg-2">{tr("All %d models of pansy, under the Pansy provider", choices().length)}</Text>
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

/** One agent's row; `inFold`, under the list's fold; `first`, no line above it. */
export function AgentRow(props: { agent: Agent; first: boolean; inFold?: boolean }) {
  const a = () => props.agent;
  const kind = () => kindOf(a());
  const st = () => status(a());
  const version = () => versions()[a().id];
  const open = () => expanded() === a().id && a().connected;
  const putAway = () => !!props.inFold && isHidden(a());
  const toggle = () => {
    const next = open() ? null : a().id;
    expanded.set(next);
    if (next) loadChanges(next);
  };
  const line = (
    <View class="min-h-[56px] flex-row items-center gap-3 py-[7px] pl-[14px] pr-[12px]">
      <Menu
        placement="bottom"
        align="start"
        offset={4}
        trigger={
          <View testID={`handle-${a().id}`} accessibilityLabel={tr("%s's menu", a().name)} class={`-m-[3px] h-7 w-7 items-center justify-center rounded-[6px] hover:bg-pill aria-expanded:bg-pill ${MOTION}`} style={{ opacity: putAway() ? 0.6 : 1 }}>
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
          <If when={a().cliMissing}>
            <Hint text={tr("%s's settings are still here (%s), but its command-line program isn't found: it may have been uninstalled. Install another agent, below the list, has its install command.", a().name, a().configFiles[0] ?? "")}>
              <Text testID={`cli-missing-${a().id}`} class="text-[11.5px] text-muted-foreground">
                {tr("CLI not found")}
              </Text>
            </Hint>
          </If>
          <If when={!a().cliMissing && version()}>
            <Hint text={tr("%s %s · pansy can't tell how it was installed — update it the way you installed it", a().name, version() ?? "")}>
              <Text testID={`version-${a().id}`} class="text-[11.5px] text-muted-foreground">
                {version() ?? ""}
              </Text>
            </Hint>
          </If>
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
    <View testID={`agent-${a().id}`} class={props.first ? "" : "border-t border-line-2"}>
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
