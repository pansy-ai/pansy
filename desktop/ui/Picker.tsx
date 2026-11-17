// The model picker, as magpie's: a pop-up at the field that opens it, a
// search line ("Filter, or type any model id…"), a rail of the providers'
// logos down its left, and the models under their provider's name, the
// routing groups first. Typing a model id it does not list picks it as
// written.
import { Scroll, signal, Text, View, type JSX } from "nsh";
import { Input, Popover } from "nsh/base";
import { tr } from "./i18n";
import { choices, contextOf, groupProviders, groups, providerOf, providers, type Choice } from "./model";
import { Badge, Icon, Logo, LogoStack, MOTION, providerLogo, SHADOW } from "./parts";

export interface PickerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** What the pop-up is placed against: the field or button that opened it. */
  trigger: JSX.Element;
  current: string | null;
  /** Leave groups out (a group's own members). */
  noGroups?: boolean;
  /** Ids not offered (a group's members already in it). */
  exclude?: string[];
  /** A first row that is not a model: the agent's own default (`onPick("")`). */
  reset?: { label: string; note: string };
  onPick: (id: string) => void;
  align?: "start" | "center" | "end";
  /** Its size where the window is small (the quick panel): 490 by 380 points of models otherwise. */
  width?: number;
  listHeight?: number;
}

function matches(choice: Choice, query: string): boolean {
  if (!query) return true;
  const hay = `${choice.id} ${choice.name} ${choice.provider}`.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((word) => hay.includes(word));
}

function Heading(props: { text: string }) {
  return (
    <View class="h-6 justify-end px-2">
      <Text class="text-[10.5px] font-semibold uppercase tracking-[0.06em] text-faint">{props.text}</Text>
    </View>
  );
}

/** A model's provider logo; a routing group's members' logos, stacked. */
export function ChoiceLogo(props: { id: string; size: number }) {
  const group = () => groups().find((g) => `group/${g.id}` === props.id);
  return (
    <If when={group()} else={<Logo name={providerLogo(providerOf(props.id)?.preset)} size={props.size} />}>
      <LogoStack names={groupProviders(group()!).map((p) => providerLogo(p.preset))} size={props.size} />
    </If>
  );
}

function Option(props: { choice: Choice; current: boolean; onPick: () => void }) {
  const c = () => props.choice;
  return (
    <View
      role="button"
      testID={`pick-${c().id}`}
      onPress={props.onPick}
      class={`h-[34px] flex-row items-center gap-[9px] rounded-[7px] px-2 ${props.current ? "bg-sel" : "hover:bg-pill"}`}
    >
      <ChoiceLogo id={c().id} size={16} />
      <Text class="text-[12.5px] text-foreground line-clamp-1">{c().name}</Text>
      <If when={contextOf(c().id)}>
        <Badge label={contextOf(c().id) ?? ""} />
      </If>
      <Text class="flex-1 text-[10.5px] text-muted-foreground line-clamp-1">{c().group ? tr("routing group") : c().provider}</Text>
      <If when={props.current}>
        <Icon name="tick" color="primary" size={12} />
      </If>
    </View>
  );
}

/** magpie's rail button: its icon in ink on a faint pill under the pointer, a little smaller while pressed. */
function RailItem(props: { on: boolean; label: string; onPress: () => void; children: JSX.Element }) {
  return (
    <View
      role="button"
      accessibilityLabel={props.label}
      onPress={props.onPress}
      class={`group h-[31px] w-[31px] items-center justify-center rounded-[8px] active:scale-94 ${MOTION} ${props.on ? "bg-sel" : "hover:bg-pill/68"}`}
    >
      {props.children}
    </View>
  );
}

function Content(outer: { of: PickerProps }) {
  const props = outer.of;
  const query = signal("");
  /** The rail's choice: every model, the groups, or one provider's. */
  const rail = signal<string>("all");
  const offered = () => choices().filter((c) => !(props.noGroups && c.group) && !(props.exclude ?? []).includes(c.id));
  const listed = () => {
    const q = query().trim();
    return offered().filter((c) => {
      if (!matches(c, q)) return false;
      if (rail() === "all") return true;
      if (rail() === "groups") return c.group;
      return !c.group && providerOf(c.id)?.id === rail();
    });
  };
  const sections = () => {
    const list = listed();
    const out: { title: string; items: Choice[] }[] = [];
    const inGroups = list.filter((c) => c.group);
    if (inGroups.length) out.push({ title: tr("Routing groups"), items: inGroups });
    for (const p of providers()) {
      const items = list.filter((c) => !c.group && providerOf(c.id)?.id === p.id);
      if (items.length) out.push({ title: p.name, items });
    }
    return out;
  };
  const railProviders = () => providers().filter((p) => offered().some((c) => !c.group && providerOf(c.id)?.id === p.id));
  // A model id typed in full that is not listed: picked as written.
  const typed = () => {
    const q = query().trim();
    return q.includes("/") && !/\s/.test(q) && !choices().some((c) => c.id === q) ? q : null;
  };
  const pick = (id: string) => {
    props.onOpenChange(false);
    props.onPick(id);
  };
  return (
    <View testID="picker" style={{ width: props.width ?? 490 }}>
      <View class="h-[39px] flex-row items-center gap-2 border-b border-line-2 px-3">
        <Icon name="find" color="faint" size={13} />
        <Input
          value={query()}
          onChange={({ value }) => query.set(value)}
          onSubmit={() => {
            const first = typed() ?? listed()[0]?.id;
            if (first) pick(first);
          }}
          autoFocus
          testID="picker-search"
          placeholder={tr("Filter, or type any model id…")}
          accessibilityLabel={tr("Search models")}
          class="h-[38px] flex-1 bg-transparent text-[13px] text-foreground placeholder:text-faint"
        />
      </View>
      <View class="flex-row" style={{ maxHeight: props.listHeight ?? 380 }}>
        <View class="w-[43px] items-center gap-[3px] border-r border-line-2 px-[5px] py-[6px]">
          <RailItem on={rail() === "all"} label={tr("Every model")} onPress={() => rail.set("all")}>
            <Icon name="nav-groups" color={rail() === "all" ? "foreground" : "muted-foreground"} hover="foreground" size={15} />
          </RailItem>
          <If when={!props.noGroups && offered().some((c) => c.group)}>
            <RailItem on={rail() === "groups"} label={tr("Routing groups")} onPress={() => rail.set("groups")}>
              <Icon name="nav-gateway" color={rail() === "groups" ? "foreground" : "muted-foreground"} hover="foreground" size={15} />
            </RailItem>
          </If>
          <View class="my-[2px] h-px w-6 bg-line-2" />
          <Scroll class="min-h-0 shrink">
            <View class="items-center gap-[3px]">
              <Each of={railProviders()} key={(p) => p.id}>
                {(p) => (
                  <RailItem on={rail() === p().id} label={p().name} onPress={() => rail.set(rail() === p().id ? "all" : p().id)}>
                    <Logo name={providerLogo(p().preset)} size={16} />
                  </RailItem>
                )}
              </Each>
            </View>
          </Scroll>
        </View>
        <Scroll class="min-h-0 flex-1">
          <View class="p-[5px]">
            <If when={props.reset}>
              <View role="button" testID="pick-reset" onPress={() => pick("")} class={`h-[34px] flex-row items-center gap-[9px] rounded-[7px] px-2 ${props.current ? "hover:bg-pill" : "bg-sel"}`}>
                <Icon name="close" color="muted-foreground" size={14} />
                <Text class="text-[12.5px] text-foreground">{props.reset?.label ?? ""}</Text>
                <Text class="flex-1 text-[10.5px] text-muted-foreground line-clamp-1">{props.reset?.note ?? ""}</Text>
              </View>
            </If>
            <If when={typed()}>
              <View role="button" testID="pick-typed" onPress={() => pick(typed()!)} class="h-[34px] flex-row items-center gap-[9px] rounded-[7px] px-2 hover:bg-pill">
                <Icon name="add" color="primary" size={14} />
                <Text class="flex-1 text-[12.5px] text-foreground">{tr("Use %s", typed() ?? "")}</Text>
              </View>
            </If>
            <Each of={sections()} key={(s) => s.title}>
              {(s) => (
                <View>
                  <Heading text={s().title} />
                  <Each of={s().items} key={(c) => c.id}>
                    {(c) => <Option choice={c()} current={props.current === c().id} onPick={() => pick(c().id)} />}
                  </Each>
                </View>
              )}
            </Each>
            <If when={listed().length === 0 && !typed()}>
              <View class="items-center gap-1 px-4 py-8">
                <Text class="text-[12.5px] text-foreground">{tr("No models match")}</Text>
                <Text class="text-center text-[11.5px] text-muted-foreground">{tr("Add a provider, or refresh a provider's models.")}</Text>
              </View>
            </If>
          </View>
        </Scroll>
      </View>
    </View>
  );
}

/** The picker's pop-up against `trigger`; pressing the trigger opens it. */
export function ModelPicker(props: PickerProps) {
  return (
    <Popover
      open={props.open}
      onOpenChange={({ open }) => props.onOpenChange(open)}
      placement="bottom"
      align={props.align ?? "end"}
      offset={4}
      margin={8}
      trigger={props.trigger}
      popupClass="overflow-hidden rounded-[11px] border border-border bg-popover"
      popupStyle={{ boxShadow: SHADOW.pop() }}
    >
      <Content of={props} />
    </Popover>
  );
}
