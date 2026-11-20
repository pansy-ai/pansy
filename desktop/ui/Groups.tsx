// Routing groups, as magpie lists them: several models an agent picks as one
// (`group/<id>`), each row its members' logos, its name and id, the members
// in order and how it routes. The gateway tries them in the group's order,
// turn by turn, or least used first, and moves on when one cannot answer.
// Below the groups, the settings that decide when it moves on.
import { signal, Text, View } from "nsh";
import { Menu, MenuItem } from "nsh/base";
import { Groups, Settings } from "pansy/native";
import { tr, trn } from "./i18n";
import { act, ask, choiceName, groupProviders, groups, isBusy, loadGroups, loadSettings, pluginRoutings, providerOf, settings, type Group, type Routing, type SettingsView } from "./model";
import { ModelPicker } from "./Picker";
import {
  Code,
  FormRow,
  Icon,
  Input,
  List,
  Logo,
  LogoStack,
  MOTION,
  Page,
  PrimaryButton,
  providerLogo,
  Row,
  RowHead,
  Segs,
  SettingRow,
  SHADOW,
  Tag,
  TextButton,
} from "./parts";

/** pansy's routings, then the ones middleware give (`<plugin>/<name>`). */
const ROUTING = (): { value: string; label: string; description: string }[] => [
  { value: "order", label: tr("In order"), description: tr("The first model until it cannot answer, then the next.") },
  { value: "rotate", label: tr("Take turns"), description: tr("The next model on each request.") },
  { value: "usage", label: tr("Least used"), description: tr("The model used least since pansy started.") },
  ...pluginRoutings().map((r) => ({ value: r, label: r, description: tr("A plugin's middleware orders the models; In order when it can't.") })),
];

/** How a group routes: its middleware's routing, or its own. */
const routeOf = (g: { routing: Routing; strategy?: string | null }): string => g.strategy || g.routing;

interface Draft {
  id: string;
  name: string;
  members: string[];
  /** A routing of `ROUTING`. */
  routing: string;
}

const draft = signal<Draft | null>(null);

function edit(group: Group | null) {
  draft.set(group ? { id: group.id, name: group.name, members: [...group.members], routing: routeOf(group) } : { id: "", name: "", members: [], routing: "order" });
}

/** "Daily coding" → "daily-coding": the id a new group gets (pansy-core's `slug`). */
const slug = (name: string) =>
  name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

function save(group: Draft) {
  return act("group", () => Groups.save(group.id, group.name, JSON.stringify(group.members), group.routing)).then((ok) => {
    void loadGroups();
    return ok;
  });
}

/** A member: its provider's logo and name, its model's. */
function memberLine(id: string): string {
  const p = providerOf(id);
  return p ? `${p.name} · ${choiceName(id)}` : id;
}

function RouteMenu(props: { group: Group }) {
  const g = () => props.group;
  return (
    <Menu
      placement="bottom"
      align="end"
      offset={4}
      trigger={<Tag label={ROUTING().find((r) => r.value === routeOf(g()))?.label ?? routeOf(g())} chevron testID={`group-route-${g().id}`} />}
      popupClass="min-w-[200px] rounded-[10px] border border-border bg-popover p-1"
      popupStyle={{ boxShadow: SHADOW.pop() }}
    >
      <Each of={ROUTING()} key={(r) => r.value}>
        {(r) => (
          <MenuItem
            role="menuitemradio"
            checked={routeOf(g()) === r().value}
            onSelect={() => void save({ id: g().id, name: g().name, members: g().members, routing: r().value })}
            class="flex-row items-start gap-2 rounded-[7px] px-2 py-[6px] hover:bg-pill focus:bg-pill"
          >
            <View class="w-3 pt-[2px]">
              <If when={routeOf(g()) === r().value}>
                <Icon name="tick" color="primary" size={12} />
              </If>
            </View>
            <View class="flex-1 gap-[2px]">
              <Text class="text-[13px] text-foreground">{r().label}</Text>
              <Text class="text-[12px] text-muted-foreground">{r().description}</Text>
            </View>
          </MenuItem>
        )}
      </Each>
    </Menu>
  );
}

function GroupRow(props: { group: Group; first: boolean }) {
  const g = () => props.group;
  return (
    <Row first={props.first} testID={`group-${g().id}`} class="min-h-[56px] py-[10px]">
      <View class="w-[44px]">
        <LogoStack names={groupProviders(g()).map((p) => providerLogo(p.preset))} size={20} />
      </View>
      <View class="min-w-0 flex-1 gap-[3px]">
        <View class="flex-row items-center gap-2">
          <Text class="text-[13px] font-medium text-foreground">{g().name}</Text>
          <Code text={`group/${g().id}`} />
        </View>
        <Text class="text-[12px] text-muted-foreground line-clamp-1">{g().members.map(memberLine).join(" → ")}</Text>
      </View>
      <RouteMenu group={g()} />
      <Tag label={trn(g().members.length, "%d model", "%d models", g().members.length)} />
      <TextButton label={tr("Edit")} testID={`group-edit-${g().id}`} onPress={() => edit(g())} />
    </Row>
  );
}

function Editor() {
  const d = () => draft()!;
  const change = (f: (d: Draft) => Draft) => draft.update((x) => (x ? f(x) : x));
  const adding = signal(false);
  const move = (i: number, by: number) =>
    change((x) => {
      const members = [...x.members];
      const j = Math.max(0, Math.min(members.length - 1, i + by));
      const [m] = members.splice(i, 1);
      members.splice(j, 0, m!);
      return { ...x, members };
    });
  // A new group's id is made from its name where it is saved.
  const submit = () => void save(d()).then((ok) => ok && draft.set(null));
  const remove = async () => {
    if (await ask({ title: tr("Delete %s?", d().name), description: tr("Agents on this group need another model."), confirm: tr("Delete"), danger: true })) {
      await act("group", () => Groups.remove(d().id));
      draft.set(null);
      await loadGroups();
    }
  };
  return (
    <View testID="group-editor" class="overflow-hidden rounded-[12px] border border-primary bg-card">
      <View class="h-[44px] flex-row items-center border-b border-line-2 px-[14px]">
        <Text class="text-[14px] font-semibold text-foreground">{d().id ? d().name : tr("New routing group")}</Text>
      </View>
      <View class="gap-[14px] px-[14px] py-[14px]">
        <FormRow label={tr("Name")}>
          <Input value={d().name} onChange={(v) => change((x) => ({ ...x, name: v }))} autoFocus testID="group-name" placeholder={tr("Daily coding")} />
        </FormRow>
        <FormRow label={tr("ID")}>
          <Text class="text-[12px] text-muted-foreground">{tr("Agents pick it as %s", `group/${d().id || (d().name.trim() ? slug(d().name) || "group" : "…")}`)}</Text>
        </FormRow>
        <FormRow label={tr("How it routes")} hint={ROUTING().find((r) => r.value === d().routing)?.description}>
          <Segs testID="group-routing" value={d().routing} onChange={(v) => change((x) => ({ ...x, routing: v }))} options={ROUTING()} />
        </FormRow>
        <FormRow label={tr("Models")} top hint={tr("The first answers for what the model can do. In order, they are tried top first.")}>
          <Each of={d().members} key={(m) => m}>
            {(m, i) => (
              <View class="h-[30px] flex-row items-center gap-[9px]">
                <Text class="w-4 text-right text-[12px] text-muted-foreground">{String(i() + 1)}</Text>
                <Logo name={providerLogo(providerOf(m())?.preset)} size={16} />
                <Text class="text-[13px] font-medium text-foreground line-clamp-1">{choiceName(m())}</Text>
                <Text class="flex-1 text-[12px] text-muted-foreground line-clamp-1">{providerOf(m())?.name ?? ""}</Text>
                <TextButton label={tr("Up")} disabled={i() === 0} onPress={() => move(i(), -1)} />
                <TextButton label={tr("Down")} disabled={i() === d().members.length - 1} onPress={() => move(i(), 1)} />
                <TextButton label={tr("Remove")} onPress={() => change((x) => ({ ...x, members: x.members.filter((y) => y !== m()) }))} />
              </View>
            )}
          </Each>
          <ModelPicker
            open={adding()}
            onOpenChange={(o) => adding.set(o)}
            current={null}
            noGroups
            exclude={d().members}
            align="start"
            onPick={(id) => change((x) => ({ ...x, members: x.members.includes(id) ? x.members : [...x.members, id] }))}
            trigger={
              <View
                testID="group-add-member"
                class={`group h-[26px] flex-row items-center gap-[5px] self-start rounded-full border border-border bg-card px-[10px] hover:bg-pill active:scale-96 ${MOTION}`}
              >
                <Icon name="add" color="muted-foreground" hover="foreground" size={11} />
                <Text class={`text-[12px] text-fg-2 group-hover:text-foreground ${MOTION}`}>{tr("Add another model")}</Text>
              </View>
            }
          />
        </FormRow>
      </View>
      <View class="flex-row items-center gap-2 border-t border-line-2 px-[14px] py-[10px]">
        <If when={d().id}>
          <TextButton label={tr("Delete")} tone="danger" testID="group-delete" onPress={() => void remove()} />
        </If>
        <View class="flex-1" />
        <TextButton label={tr("Cancel")} onPress={() => draft.set(null)} />
        <PrimaryButton label={tr("Save")} testID="group-save" disabled={!d().name.trim() || d().members.length === 0 || isBusy("group")} onPress={submit} />
      </View>
    </View>
  );
}

/** A routing setting as magpie offers it: a few values side by side, and the current one if it is none of them. */
function Choices(props: { keyName: keyof SettingsView; values: { value: number; label: string }[]; testID: string }) {
  const current = () => Number(settings()?.[props.keyName] ?? 0);
  const options = () => {
    const known = props.values.some((v) => v.value === current());
    return (known ? props.values : [...props.values, { value: current(), label: String(current()) }]).map((v) => ({ value: String(v.value), label: v.label }));
  };
  return (
    <Segs
      testID={props.testID}
      small
      value={String(current())}
      options={options()}
      onChange={(v) => void act(`setting:${props.keyName}`, () => Settings.set(props.keyName, v)).then(() => loadSettings())}
    />
  );
}

export function GroupsPage() {
  const filter = signal("");
  const listed = () => {
    const q = filter().trim().toLowerCase();
    return groups().filter(
      (g) => g.id !== draft()?.id && (!q || g.name.toLowerCase().includes(q) || g.id.includes(q) || g.members.some((m) => m.toLowerCase().includes(q))),
    );
  };
  return (
    <Page testID="page-groups">
      <View class="min-h-[26px] flex-row items-center gap-2 px-[2px]">
        <Input value={filter()} onChange={(v) => filter.set(v)} testID="group-filter" placeholder={tr("Filter groups and models")} class="h-[24px] w-[300px]" />
        <Text class="text-[11px] text-faint">{trn(groups().length, "%d group", "%d groups", groups().length)}</Text>
        <View class="flex-1" />
        <PrimaryButton label={tr("New group")} icon="add" testID="group-new" onPress={() => edit(null)} />
      </View>
      <If when={draft()}>
        <Editor />
      </If>
      <If when={listed().length > 0}>
        <List testID="groups">
          <Each of={listed()} key={(g) => g.id}>
            {(g, i) => <GroupRow group={g()} first={i() === 0} />}
          </Each>
        </List>
      </If>
      <If when={groups().length === 0 && !draft()}>
        <View class="items-center gap-2 rounded-[12px] border border-dashed border-border px-6 py-8">
          <Text class="text-[13px] font-medium text-foreground">{tr("No routing groups")}</Text>
          <Text class="text-center text-[12px] text-muted-foreground">{tr("Put models from several providers together, so a quota running out never stops an agent.")}</Text>
        </View>
      </If>
      <RowHead label={tr("Settings")} class="pt-3" />
      <If when={settings()}>
        <List>
          <SettingRow first title={tr("Rate limited")} description={tr("How long a key or model rests before it is tried again.")}>
            <Choices
              keyName="cooldownSecs"
              testID="setting-cooldown"
              values={[
                { value: 30, label: tr("30 s") },
                { value: 60, label: tr("1 min") },
                { value: 300, label: tr("5 min") },
              ]}
            />
          </SettingRow>
          <SettingRow title={tr("Slow to start")} description={tr("How long to wait for the first byte before trying the next.")}>
            <Choices
              keyName="firstByteTimeoutSecs"
              testID="setting-first-byte"
              values={[
                { value: 60, label: tr("1 min") },
                { value: 120, label: tr("2 min") },
                { value: 300, label: tr("5 min") },
              ]}
            />
          </SettingRow>
          <SettingRow title={tr("Most tries")} description={tr("Accounts or members one request may try.")}>
            <Choices
              keyName="maxAttempts"
              testID="setting-attempts"
              values={[
                { value: 3, label: "3" },
                { value: 6, label: "6" },
                { value: 10, label: "10" },
              ]}
            />
          </SettingRow>
        </List>
      </If>
    </Page>
  );
}
