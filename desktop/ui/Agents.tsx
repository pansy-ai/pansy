// Agents, as magpie lists them: what connecting does, said once above one
// card of rows; over the card, the agents whose programs are behind, with
// Update all. The agents in use stay in view; the rest fold under the
// card's foot (Show N more), the ones hidden by hand last, and Manage
// hides or shows many at once. Under the card, how to install the agents
// not on this computer, and the profiles.
import { Clipboard, Text, View } from "nsh";
import { arrange, cliOf, here, installsOpen, isHidden, managing, showAll, showHidden, updatable, updateAll, updating, updatingAll } from "./agentState";
import { AgentRow, SkeletonRow } from "./AgentRow";
import { tr } from "./i18n";
import { agents, agentsRead, go, providers, toast, type Agent } from "./model";
import { ProfilesBar } from "./ProfilesBar";
import { adding } from "./Providers";
import { agentLogo, CopyButton, Hint, Icon, List, Logo, MOTION, Page, PrimaryButton } from "./parts";

/** Above the list: what connecting does, or, before any agent is connected, the welcome. */
function Lead() {
  const first = () => agentsRead() && !agents().some((a) => a.connected);
  const said = () => tr("Switch an agent on and the models you set up in pansy show up as the %s provider in its own model list; or pick its model right here.", "\u0001").split("\u0001");
  return (
    <If
      when={!first()}
      else={
        <View testID="agents-welcome" class="mb-[2px] rounded-[12px] border border-border bg-card px-[18px] py-4">
          <Text class="mb-1 text-[14px] font-semibold text-foreground">{tr("Let your agents use the models you set up in pansy")}</Text>
          <Text class="text-[12.5px] leading-[19px] text-muted-foreground">
            {tr("Switch an agent on and your models show up as the pansy provider in its own model list, to change in the agent. Or pick a model right here: the agent is connected and starts on it. Switch it off and it goes back to how it was.")}
          </Text>
          <If when={providers().length === 0}>
            <View class="mt-3 flex-row">
              <PrimaryButton
                label={tr("Add a provider")}
                testID="agents-add-provider"
                onPress={() => {
                  adding.set(true);
                  go("providers");
                }}
              />
            </View>
          </If>
        </View>
      }
    >
      <Text testID="agents-lead" class="mx-[2px] mb-[2px] text-[12.5px] text-fg-2">
        {said()[0]}
        <Text class="font-semibold text-foreground">pansy</Text>
        {said()[1] ?? ""}
      </Text>
    </If>
  );
}

/**
 * Above the list once two agents' programs are behind: which, and Update
 * all, which updates them one at a time; then how far it is, and how it
 * went, with each agent's versions from and to.
 */
function Updates() {
  const all = () => updatingAll();
  const running = () => {
    const s = all();
    return !!s && s.done < s.n;
  };
  const behind = () => here().filter((a) => cliOf(a)?.update || updating().includes(a.id));
  const failed = () => !running() && (all()?.failed.length ?? 0) > 0;
  const said = () => {
    const s = all();
    if (s && s.done < s.n) return tr("Updating %s… (%d of %d)", s.now, s.done + 1, s.n);
    if (s) {
      const ok = s.n - s.failed.length;
      const list = s.changed.join(", ");
      const names = s.failed.map((f) => f.name).join(", ");
      if (s.failed.length === 0) return tr("%d agents updated: %s", ok, list);
      return ok > 0 ? tr("%d of %d agents updated: %s · %d didn't: %s", ok, s.n, list, s.failed.length, names) : tr("%d of %d agents updated · %d didn't: %s", ok, s.n, s.failed.length, names);
    }
    return tr("%d agents have updates: %s", behind().length, behind().map((a) => a.name).join(", "));
  };
  /** Every change and every error whole, where the line is cut short. */
  const whole = () => [...(all()?.changed ?? []), ...(all()?.failed ?? []).map((f) => f.why)].join("\n") || said();
  return (
    <If when={running() || !!all() || behind().length >= 2}>
      <View testID="agents-updates" class="mx-[2px] min-h-[22px] flex-row items-center gap-[10px]">
        <View class="min-w-0 flex-1">
          <Hint text={whole()}>
            <Text testID="agents-updates-said" class={`text-[12.5px] line-clamp-1 ${failed() ? "text-destructive" : "text-fg-2"}`}>
              {said()}
            </Text>
          </Hint>
        </View>
        <If when={running() || updatable().length >= 2}>
          <Hint
            text={
              running()
                ? tr("Updating…")
                : updatable()
                    .map((a) => `${a.name}: ${cliOf(a)?.version ?? ""} → ${cliOf(a)?.latest ?? ""}`)
                    .join("\n")
            }
          >
            <View
              role="button"
              testID="agents-update-all"
              disabled={running()}
              onPress={() => void updateAll()}
              class={`group h-5 flex-row items-center gap-1 rounded-full border border-border bg-card pl-[6px] pr-2 hover:bg-pill active:scale-96 ${MOTION}`}
            >
              <Icon name={running() ? "sync" : "bottom"} color="muted-foreground" size={11} />
              <Text class={`text-[11.5px] font-medium ${running() ? "text-muted-foreground" : "text-fg-2 group-hover:text-foreground"} ${MOTION}`}>{running() ? tr("Updating…") : tr("Update all")}</Text>
            </View>
          </Hint>
        </If>
      </View>
    </If>
  );
}

/** A caption in the fold: Not set up, Hidden. */
function Cap(props: { text: string }) {
  return <Text class="border-t border-line-2 px-[14px] pb-[3px] pt-[10px] text-[11px] font-medium tracking-[0.02em] text-muted-foreground">{props.text}</Text>;
}

/** The card's foot: Show N more in its middle, Manage at its end. */
function Foot(props: { shown: Agent[]; folded: Agent[] }) {
  const unset = () => props.folded.filter((a) => !isHidden(a));
  const byHand = () => props.folded.filter(isHidden);
  const label = () =>
    showAll()
      ? tr("Show less")
      : unset().length === 0
        ? tr("%d hidden agents", byHand().length)
        : props.shown.length === 0
          ? tr("Show all %d agents", unset().length)
          : tr("Show %d more", unset().length);
  return (
    <View class="h-[35px] flex-row items-center border-t border-line-2 px-2">
      <View class="flex-1" />
      <If when={props.folded.length > 0}>
        <View
          role="button"
          testID="agents-more"
          onPress={() => {
            showAll.set(!showAll());
            if (unset().length === 0) showHidden.set(showAll());
          }}
          class={`group h-[29px] flex-row items-center gap-[5px] rounded-[6px] px-[6px] hover:bg-card-2 ${MOTION}`}
        >
          <Text class={`text-[12px] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{label()}</Text>
          <Icon name={showAll() ? "chev-up" : "chev-down"} color="muted-foreground" hover="foreground" size={10} />
        </View>
      </If>
      <View class="flex-1 flex-row justify-end">
        <View
          role="button"
          testID="agents-manage-open"
          accessibilityLabel={tr("Manage agents")}
          onPress={() => managing.set(true)}
          class={`group h-[29px] flex-row items-center gap-[5px] rounded-[6px] px-2 hover:bg-card-2 ${MOTION}`}
        >
          <Icon name="list" color="muted-foreground" hover="foreground" size={14} />
          <Text class={`text-[12px] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{tr("Manage")}</Text>
        </View>
      </View>
    </View>
  );
}

/** What unrolls under the foot: the agents not set up, then the hidden ones. */
function Fold(props: { folded: Agent[] }) {
  const unset = () => props.folded.filter((a) => !isHidden(a));
  const byHand = () => props.folded.filter(isHidden);
  return (
    <>
      <If when={unset().length > 0}>
        <Cap text={tr("Not set up")} />
        <Each of={unset()} key={(a) => a.id}>
          {(a, i) => <AgentRow agent={a()} first={i() === 0} inFold />}
        </Each>
      </If>
      <If when={byHand().length > 0 && unset().length === 0}>
        <Cap text={tr("Hidden")} />
        <Each of={byHand()} key={(a) => a.id}>
          {(a, i) => <AgentRow agent={a()} first={i() === 0} inFold />}
        </Each>
      </If>
      <If when={byHand().length > 0 && unset().length > 0}>
        <View role="button" testID="agents-hidden" onPress={() => showHidden.set(!showHidden())} class="flex-row items-center gap-1 border-t border-line-2 px-[14px] pb-2 pt-[10px]">
          <Text class="text-[11px] font-medium tracking-[0.02em] text-muted-foreground">{tr("Hidden (%d)", byHand().length)}</Text>
          <Icon name={showHidden() ? "chev-down" : "chev-right"} color="muted-foreground" size={9} />
        </View>
        <If when={showHidden()}>
          <Each of={byHand()} key={(a) => a.id}>
            {(a, i) => <AgentRow agent={a()} first={i() === 0} inFold />}
          </Each>
        </If>
      </If>
    </>
  );
}

/** What runs an install command; the tools by their own names. */
const via = (v: string) => (v === "script" ? tr("Installer") : ({ powershell: "PowerShell", brew: "Homebrew", npm: "npm" } as Record<string, string>)[v] ?? v);

/** The agents not on this computer (or whose program is gone), with the commands their vendors give. */
function Installs() {
  const items = () => agents().filter((a) => (!(a.installed || a.connected) || a.cliMissing) && a.install.length > 0);
  const open = () => installsOpen() ?? here().length === 0;
  return (
    <If when={items().length > 0}>
      <View testID="agents-install" class="mt-[2px]">
        <View role="button" testID="agents-install-head" onPress={() => installsOpen.set(!open())} class="flex-row items-center gap-[5px] self-start px-[2px] py-1">
          <Text class="text-[12.5px] text-muted-foreground hover:text-foreground">{tr("Install another agent (%d)", items().length)}</Text>
          <Icon name={open() ? "chev-up" : "chev-down"} color="muted-foreground" size={10} />
        </View>
        <If when={open()}>
          <Text class="mx-[2px] mb-2 mt-1 text-[12px] leading-[18px] text-muted-foreground">
            {tr("Run a command in a terminal, then come back: pansy lists the agent here once it is installed. The npm commands need Node.js (nodejs.org).")}
          </Text>
          <Each of={items()} key={(a) => a.id}>
            {(a) => (
              <View testID={`install-${a().id}`} class="mb-[6px] flex-row items-start gap-3 rounded-[10px] border border-border bg-card px-3 py-[9px]">
                <View class="w-[150px] flex-row items-center gap-2 pt-[2px]">
                  <Logo name={agentLogo(a().id)} size={18} />
                  <View class="min-w-0 flex-1">
                    <Text class="text-[13px] font-medium text-foreground line-clamp-1">{a().name}</Text>
                    <If when={a().cliMissing}>
                      <Text class="text-[11px] text-muted-foreground">{tr("CLI not found")}</Text>
                    </If>
                  </View>
                </View>
                <View class="min-w-0 flex-1 gap-1">
                  <Each of={a().install} key={(c) => c.command}>
                    {(c) => (
                      <View class="h-[22px] flex-row items-center gap-2">
                        <Text class="w-[84px] text-[11.5px] text-muted-foreground">{via(c().via)}</Text>
                        <View class="min-w-0 flex-1 rounded-[5px] bg-pill px-[6px] py-[2px]">
                          <Text class="font-mono text-[11.5px] text-fg-2 line-clamp-1">{c().command}</Text>
                        </View>
                        <CopyButton label={tr("%s's install command", a().name)} onCopy={() => Clipboard.writeText(c().command).then(() => toast(tr("Copied")))} />
                      </View>
                    )}
                  </Each>
                </View>
              </View>
            )}
          </Each>
        </If>
      </View>
    </If>
  );
}

export function AgentsPage() {
  const layout = () => arrange();
  return (
    <Page testID="page-agents">
      <Lead />
      <Updates />
      <List testID="agents">
        <If
          when={agentsRead()}
          else={
            <Each of={[92, 64, 78, 64, 92]} key={(w, i) => `${i}${w}`}>
              {(w, i) => <SkeletonRow first={i() === 0} name={w()} />}
            </Each>
          }
        >
          <Each of={layout().shown} key={(a) => a.id}>
            {(a, i) => <AgentRow agent={a()} first={i() === 0} />}
          </Each>
          <If when={here().length === 0}>
            <View class="items-center gap-1 px-5 py-9">
              <Text class="text-[13px] font-medium text-foreground">{tr("No agents found")}</Text>
              <Text class="text-center text-[12.5px] text-muted-foreground">{tr("Install Claude Code, Codex, Gemini CLI, OpenCode… and pansy will list them here.")}</Text>
            </View>
          </If>
          <If when={here().length > 0}>
            <Foot shown={layout().shown} folded={layout().folded} />
          </If>
          <If when={showAll() && layout().folded.length > 0}>
            <Fold folded={layout().folded} />
          </If>
        </If>
      </List>
      <Installs />
      <ProfilesBar />
    </Page>
  );
}
