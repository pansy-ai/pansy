// Profiles under the agents, as magpie draws them: a chip each, ticked
// while the agents are on it; ↻ and × on a chip ask a second click before
// they overwrite or delete it; a click on the chip shows what it holds,
// agent by agent, with its Apply. Save current names the setup as it is.
import { signal, Text, View } from "nsh";
import { Profiles } from "pansy/native";
import { tr } from "./i18n";
import { act, agents, choiceName, isBusy, loadAgents, loadProfiles, profiles, toast, type Profile } from "./model";
import { agentLogo, Input, Label, Logo, MOTION, PrimaryButton, TextButton } from "./parts";

/** The profile whose details are shown, by name. */
const openProfile = signal<string | null>(null);
/** A ↻ or × clicked once, waiting for its second click. */
const armed = signal<{ name: string; action: "save" | "delete" } | null>(null);
let disarm: ReturnType<typeof setTimeout> | undefined;

function save(name: string) {
  void act("profile", () => Profiles.save(name), tr("Saved as %s", name)).then(() => loadProfiles());
}

function remove(name: string) {
  if (openProfile() === name) openProfile.set(null);
  void act("profile", () => Profiles.remove(name), tr("%s deleted", name)).then(() => loadProfiles());
}

function apply(name: string) {
  void act("profile", async () => {
    const problems = JSON.parse(await Profiles.apply(name)) as string[];
    toast(problems.length ? tr("%s is on, except: %s", name, problems.join("; ")) : tr("%s is on", name), problems.length ? "error" : "default");
  }).then(() => {
    openProfile.set(null);
    void loadProfiles();
    void loadAgents();
  });
}

function armOrDo(name: string, action: "save" | "delete") {
  clearTimeout(disarm);
  const a = armed();
  if (a?.name === name && a.action === action) {
    armed.set(null);
    if (action === "save") save(name);
    else remove(name);
    return;
  }
  armed.set({ name, action });
  disarm = setTimeout(() => armed.set(null), 4000);
}

/** A chip's ↻ or ×: faint until the chip is under the pointer, whole under the pointer itself, then its question once clicked. */
function ChipAct(props: { profile: string; action: "save" | "delete"; shown: boolean }) {
  const on = () => armed()?.name === props.profile && armed()?.action === props.action;
  return (
    <View
      role="button"
      testID={`profile-${props.action}-${props.profile}`}
      accessibilityLabel={props.action === "save" ? tr("Update to the current setup") : tr("Delete profile")}
      onPress={() => armOrDo(props.profile, props.action)}
      class={`${on() ? "h-5 items-center justify-center rounded-[10px] bg-destructive-soft px-[7px]" : "h-5 w-5 items-center justify-center rounded-full hover:bg-[#7f7f7f]/20 hover:opacity-100"} ${MOTION}`}
      style={{ opacity: on() ? 1 : props.shown ? 0.6 : 0 }}
    >
      <Text class={on() ? "text-[11.5px] text-destructive" : "text-[13px] leading-[14px] text-primary"}>
        {on() ? (props.action === "save" ? tr("Overwrite?") : tr("Delete?")) : props.action === "save" ? "↻" : "×"}
      </Text>
    </View>
  );
}

function Chip(props: { profile: Profile }) {
  const hover = signal(false);
  const name = () => props.profile.name;
  const open = () => openProfile() === name();
  const lit = () => hover() || open();
  return (
    <View
      testID={`profile-${name()}`}
      onMouseEnter={() => hover.set(true)}
      onMouseLeave={() => hover.set(false)}
      class={`h-[30px] flex-row items-center gap-[2px] rounded-full border pr-[6px] ${lit() ? "border-primary bg-accent" : "border-border bg-card"}`}
    >
      <View role="button" testID={`profile-use-${name()}`} onPress={() => openProfile.set(open() ? null : name())} class="h-full flex-row items-center pl-[13px] pr-[2px]">
        <If when={props.profile.active}>
          <Text class="-ml-[2px] mr-1 text-[11px] font-semibold text-primary">✓</Text>
        </If>
        <Text class={`text-[13px] ${lit() ? "text-primary" : "text-foreground"}`}>{name()}</Text>
      </View>
      <ChipAct profile={name()} action="save" shown={hover()} />
      <ChipAct profile={name()} action="delete" shown={hover()} />
    </View>
  );
}

/** What a profile holds, agent by agent, with its Apply, Rename and ×. */
function Detail(props: { profile: Profile }) {
  const p = () => props.profile;
  const renaming = signal(false);
  const to = signal("");
  const rename = () => {
    const name = to().trim();
    if (!name || name === p().name) return renaming.set(false);
    void act("profile", () => Profiles.rename(p().name, name)).then((ok) => {
      if (ok) {
        openProfile.set(name);
        renaming.set(false);
      }
      void loadProfiles();
    });
  };
  const held = () => Object.entries(p().agents);
  const agentName = (id: string) => agents().find((a) => a.id === id)?.name ?? id;
  const hasSmall = (id: string) => agents().find((a) => a.id === id)?.hasSmallModel ?? false;
  return (
    <View testID="profile-detail" class="w-full rounded-[10px] border border-border bg-card px-3 pb-[10px] pt-[9px]">
      <View class="flex-row items-center gap-[10px]">
        <If
          when={renaming()}
          else={
            <>
              <Text class="min-w-0 shrink text-[12px] font-semibold text-foreground line-clamp-1">{p().name}</Text>
              <If when={p().active}>
                <Text class="text-[11.5px] text-primary">{tr("In use")}</Text>
              </If>
              <View class="flex-1" />
            </>
          }
        >
          <Input value={to()} onChange={(v) => to.set(v)} onSubmit={rename} autoFocus testID="profile-rename-name" class="flex-1" />
        </If>
        <PrimaryButton label={tr("Apply")} testID="profile-apply" disabled={isBusy("profile")} onPress={() => apply(p().name)} />
        <TextButton
          label={renaming() ? tr("Save") : tr("Rename")}
          testID="profile-rename"
          onPress={() => {
            if (renaming()) rename();
            else {
              to.set(p().name);
              renaming.set(true);
            }
          }}
        />
        <View role="button" accessibilityLabel={tr("Close")} onPress={() => openProfile.set(null)} class="group h-6 w-6 items-center justify-center rounded-[6px] hover:bg-[#7f7f7f]/15">
          <Text class="text-[16px] leading-[16px] text-muted-foreground group-hover:text-foreground">×</Text>
        </View>
      </View>
      <If when={held().length === 0}>
        <Text class="mt-2 text-[11.5px] text-muted-foreground">{tr("No agent settings saved")}</Text>
      </If>
      <Each of={held()} key={([id]) => id}>
        {(entry) => (
          <View class="mt-[9px]">
            <View class="flex-row items-center gap-[6px]">
              <Logo name={agentLogo(entry()[0])} size={14} />
              <Text class="text-[12px] font-medium text-fg-2">{agentName(entry()[0])}</Text>
            </View>
            <View class="ml-5 mt-1 gap-[2px]">
              <View class="flex-row gap-3">
                <Text class="w-[84px] text-[12px] leading-[18px] text-muted-foreground">{tr("model")}</Text>
                <Text class="flex-1 text-[12px] leading-[18px] text-foreground line-clamp-1">{choiceName(entry()[1].model)}</Text>
              </View>
              <If when={hasSmall(entry()[0])}>
                <View class="flex-row gap-3">
                  <Text class="w-[84px] text-[12px] leading-[18px] text-muted-foreground">{tr("quick tasks")}</Text>
                  <Text class={`flex-1 text-[12px] leading-[18px] line-clamp-1 ${entry()[1].smallModel ? "text-foreground" : "text-faint"}`}>
                    {entry()[1].smallModel ? choiceName(entry()[1].smallModel!) : tr("same as model")}
                  </Text>
                </View>
              </If>
            </View>
          </View>
        )}
      </Each>
    </View>
  );
}

/** The profiles in a row under the agents; `stacked`, on lines of their own (the quick panel's pop-up). */
export function ProfilesBar(props: { stacked?: boolean }) {
  const naming = signal(false);
  const name = signal("");
  const saveNew = () => {
    const n = name().trim();
    if (!n) return;
    void act("profile", () => Profiles.save(n), tr("Saved as %s", n)).then((ok) => {
      if (ok) {
        name.set("");
        naming.set(false);
      }
      void loadProfiles();
    });
  };
  const open = () => profiles().find((p) => p.name === openProfile());
  return (
    <View testID="profiles" class={props.stacked ? "gap-2 p-[10px]" : "min-h-[36px] flex-row items-start gap-[10px] px-1 pb-[2px] pt-[2px]"}>
      <View class={props.stacked ? "px-[2px]" : "h-[30px] justify-center"}>
        <Label text={tr("Profiles")} />
      </View>
      <View class={`min-w-0 flex-row flex-wrap gap-2 ${props.stacked ? "" : "flex-1"}`}>
        <Each of={profiles()} key={(p) => p.name}>
          {(p) => <Chip profile={p()} />}
        </Each>
        <If when={profiles().length === 0 && !naming()}>
          <Text class="text-[11.5px] leading-[30px] text-muted-foreground line-clamp-1">{tr("none yet · save the setup to switch back in one click")}</Text>
        </If>
        <If when={open()}>
          <Detail profile={open()!} />
        </If>
      </View>
      <View class="h-[30px] flex-row items-center gap-2">
        <If when={naming()}>
          <Input value={name()} onChange={(v) => name.set(v)} onSubmit={saveNew} autoFocus testID="profile-name" placeholder={tr("Profile name")} class={props.stacked ? "flex-1" : "w-[140px]"} />
        </If>
        <TextButton
          label={naming() ? tr("Save") : tr("Save current")}
          icon={naming() ? undefined : "add"}
          testID={naming() ? "profile-save" : "profile-new"}
          disabled={naming() && (!name().trim() || isBusy("profile"))}
          onPress={() => (naming() ? saveNew() : naming.set(true))}
        />
        <If when={naming()}>
          <TextButton label={tr("Cancel")} onPress={() => naming.set(false)} />
        </If>
      </View>
    </View>
  );
}
