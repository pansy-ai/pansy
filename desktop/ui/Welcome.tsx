// The welcome, as T3 Code's: the window's first look on a computer pansy
// is new to, before the sidebar and the pages. Which computer it sets up,
// then a line each: the agents found here (and how to install more), the
// providers other apps have set up, ticked to bring over, and the folders
// the agents worked in, which Sessions lists. Its button imports what is
// ticked and opens the window; it is shown once to everyone, and not again
// once gone through (NSH's storage).
import { Clipboard, Scroll, signal, Storage, Text, View, type JSX } from "nsh";
import { Button as BaseButton } from "nsh/base";
import { Prefs, Providers, Sessions } from "pansy/native";
import { here } from "./agentState";
import { tr, trn } from "./i18n";
import { act, agents, agentsRead, isBusy, loadAgents, loadImportFound, loadPage, loadProviders, page, toast, type ImportItem, type ImportSource, type SessionList } from "./model";
import { agentLogo, Check, CopyButton, Icon, List, Logo, logoNamed, MOTION, Row, TextButton } from "./parts";
import { agentName } from "./Sessions";

/** Kept across runs: the welcome was gone through. */
const DONE_KEY = "welcome.done";

/** The welcome shows; null while that is being found out. */
export const welcoming = signal<boolean | null>(null);

/** Shows the welcome unless it was gone through. */
export async function startWelcome() {
  welcoming.set(!(await Storage.getItem(DONE_KEY).catch(() => null)));
}

/** A provider another app has, to bring over. */
type Offer = { source: ImportSource; item: ImportItem; key: string };
/** A folder the agents worked in. */
type Project = { path: string; sessions: number; last: string };

const offers = signal<Offer[] | null>(null);
/** Ticked to bring over, by offer. */
const picked = signal<Record<string, boolean>>({});
const projects = signal<Project[] | null>(null);
/** The agents whose sessions they are. */
const projectAgents = signal<string[]>([]);
const home = signal("");
/** What is unrolled: the agents to install, the providers, the projects. */
const open = signal<Record<string, boolean>>({});
const toggle = (k: string) => open.update((o) => ({ ...o, [k]: !o[k] }));

async function look() {
  try {
    const sources = JSON.parse(await Providers.importSources()) as ImportSource[];
    const list = sources.flatMap((source) =>
      source.items.filter((item) => !item.skip && item.status !== "same").map((item) => ({ source, item, key: `${source.id}\n${item.ref}` })),
    );
    picked.set(Object.fromEntries(list.map((o) => [o.key, o.item.picked])));
    offers.set(list);
  } catch {
    offers.set([]); // a look of its own: adding by hand is still there
  }
  try {
    const said = JSON.parse(await Sessions.list()) as SessionList;
    const by = new Map<string, Project>();
    for (const s of said.sessions) {
      if (!s.cwd) continue;
      const p = by.get(s.cwd) ?? { path: s.cwd, sessions: 0, last: "" };
      p.sessions += 1;
      if ((s.last ?? "") > p.last) p.last = s.last ?? "";
      by.set(s.cwd, p);
    }
    home.set(said.home);
    projectAgents.set(once(said.sessions.filter((s) => s.cwd).map((s) => s.agent)));
    projects.set([...by.values()].sort((a, b) => b.last.localeCompare(a.last)));
  } catch {
    projects.set([]);
  }
}

/** A path as the welcome says it: the home folder as ~. */
function tilde(path: string): string {
  const h = home();
  return h && (path.startsWith(`${h}/`) || path.startsWith(`${h}\\`)) ? `~${path.slice(h.length)}` : path;
}

const baseName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;

/** The names in `names` once each, in order. */
const once = (names: string[]) => [...new Set(names)];

const chosen = () => (offers() ?? []).filter((o) => picked()[o.key]);

/** Imports what is ticked, then opens the window, the welcome gone through. */
function begin() {
  const picks = chosen().map((o) => ({ source: o.source.id, ref: o.item.ref, mode: o.item.keyOf ? "key" : "add", name: "" }));
  void act("welcome", async () => {
    if (picks.length) {
      const out = JSON.parse(await Providers.import(JSON.stringify(picks))) as { names: string[]; ids: string[] };
      if (out.names.length) toast(tr("Imported %d: %s", out.names.length, out.names.join(", ")));
      await Promise.all([loadProviders(), loadImportFound()]);
      // the vendors' own model lists, as for a provider added by hand
      for (const id of out.ids) void Providers.refreshModels(id).then(() => loadProviders()).catch(() => {});
    }
    await Storage.setItem(DONE_KEY, "1").catch(() => {});
    welcoming.set(false);
    void loadPage(page());
  });
}

/** One line of the welcome: its name on the left, under a hairline. */
function Section(props: { label: string; testID: string; children?: JSX.Element }) {
  return (
    <View testID={props.testID} class="flex-row gap-4 border-t border-line-2 py-[18px]">
      <Text class="w-[104px] pt-[2px] text-[13px] text-muted-foreground">{props.label}</Text>
      <View class="min-w-0 flex-1 gap-[10px]">{props.children}</View>
    </View>
  );
}

/** Review all N, or Hide, at a line's end. */
function Unroll(props: { label: string; open: boolean; onPress: () => void; testID: string }) {
  return (
    <View role="button" testID={props.testID} onPress={props.onPress} class={`group h-[26px] flex-row items-center gap-[5px] rounded-[6px] px-[7px] hover:bg-pill ${MOTION}`}>
      <Text class={`text-[13px] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{props.open ? tr("Hide") : props.label}</Text>
      <Icon name={props.open ? "chev-up" : "chev-down"} color="muted-foreground" hover="foreground" size={10} />
    </View>
  );
}

/** A line's head: what it is, a line under it, and what is at its end. */
function Head(props: { lead: JSX.Element; title: string; sub?: string; end?: JSX.Element; testID?: string }) {
  return (
    <View class="min-h-[30px] flex-row items-center gap-3">
      <View class="w-[20px] items-center">{props.lead}</View>
      <View class="min-w-0 flex-1">
        <Text testID={props.testID} class="text-[14px] font-medium text-foreground line-clamp-1">
          {props.title}
        </Text>
        <If when={props.sub}>
          <Text class="text-[13px] text-muted-foreground line-clamp-1">{props.sub}</Text>
        </If>
      </View>
      {props.end}
    </View>
  );
}

/** The agents on this computer, ready to connect, and how to install more. */
function AgentsLine() {
  const missing = () => agents().filter((a) => !(a.installed || a.connected) && a.install.length > 0);
  const unrolled = () => open().agents ?? (agentsRead() && here().length === 0);
  return (
    <Section label={tr("Agents")} testID="welcome-agents">
      <If when={agentsRead()} else={<Text class="text-[13px] text-muted-foreground">{tr("Looking for agents…")}</Text>}>
        <Each of={here()} key={(a) => a.id}>
          {(a) => (
            <Head
              testID={`welcome-agent-${a().id}`}
              lead={<Logo name={agentLogo(a().id)} size={18} />}
              title={a().name}
              end={
                <View class="flex-row items-center gap-[5px]">
                  <Icon name="tick" color="success" size={12} />
                  <Text class="text-[13px] font-medium text-success">{tr("Ready")}</Text>
                </View>
              }
            />
          )}
        </Each>
        <If when={here().length === 0}>
          <Head lead={<Icon name="nav-agents" color="muted-foreground" size={16} />} title={tr("No agents found")} sub={tr("Install one, then look again: pansy finds it here.")} />
        </If>
        <If when={missing().length > 0}>
          <View role="button" testID="welcome-more-agents" onPress={() => toggle("agents")} class="group min-h-[30px] flex-row items-center gap-3">
            <View class="w-[20px] items-center">
              <Icon name="add" color="muted-foreground" hover="foreground" size={14} />
            </View>
            <Text class={`text-[14px] text-muted-foreground group-hover:text-foreground ${MOTION}`}>{tr("Add more agents")}</Text>
            <View class="min-w-0 flex-1 flex-row items-center gap-[9px] overflow-hidden">
              <Each of={missing().slice(0, 8)} key={(a) => a.id}>
                {(a) => <Logo name={agentLogo(a().id)} size={15} />}
              </Each>
            </View>
            <Icon name={unrolled() ? "chev-up" : "chev-down"} color="muted-foreground" hover="foreground" size={10} />
          </View>
          <If when={unrolled()}>
            <View class="gap-2 pl-8">
              <View class="flex-row items-center gap-2">
                <Text class="flex-1 text-[12px] leading-[18px] text-muted-foreground">{tr("Run a command in a terminal, then look again: pansy lists the agent once it is installed.")}</Text>
                <TextButton label={tr("Look again")} icon="sync" testID="welcome-agents-again" onPress={() => void loadAgents()} />
              </View>
              <List>
                <Each of={missing()} key={(a) => a.id}>
                  {(a, i) => (
                    <Row first={i() === 0} testID={`welcome-install-${a().id}`}>
                      <Logo name={agentLogo(a().id)} size={16} />
                      <Text class="w-[120px] text-[13px] text-foreground line-clamp-1">{a().name}</Text>
                      <View class="min-w-0 flex-1 rounded-[5px] bg-pill px-[6px] py-[2px]">
                        <Text class="font-mono text-[12px] text-fg-2 line-clamp-1">{a().install[0].command}</Text>
                      </View>
                      <CopyButton label={tr("%s's install command", a().name)} onCopy={() => Clipboard.writeText(a().install[0].command).then(() => toast(tr("Copied")))} />
                    </Row>
                  )}
                </Each>
              </List>
            </View>
          </If>
        </If>
      </If>
    </Section>
  );
}

/** The providers other apps have set up, ticked to bring over, and each of them unrolled. */
function ProvidersLine() {
  const list = () => offers() ?? [];
  const n = () => chosen().length;
  const from = () => once(list().map((o) => o.source.name)).join(", ");
  const tickAll = (on: boolean) => picked.set(Object.fromEntries(list().map((o) => [o.key, on])));
  return (
    <Section label={tr("Providers")} testID="welcome-providers">
      <If when={offers()} else={<Text class="text-[13px] text-muted-foreground">{tr("Looking for providers in other apps…")}</Text>}>
        <If
          when={list().length > 0}
          else={<Head lead={<Icon name="nav-providers" color="muted-foreground" size={16} />} title={tr("No providers in other apps")} sub={tr("Add one on the Providers page once you start.")} />}
        >
          <Head
            testID="welcome-providers-title"
            lead={<Check checked={n() === list().length} indeterminate={n() > 0 && n() < list().length} onChange={tickAll} label={tr("Import them all")} testID="welcome-providers-all" />}
            title={trn(list().length, "Import %d provider and its keys", "Import %d providers and their keys", list().length)}
            sub={tr("From %s", from())}
            end={<Unroll label={tr("Review all %d", list().length)} open={!!open().providers} onPress={() => toggle("providers")} testID="welcome-providers-review" />}
          />
          <If when={open().providers}>
            <View class="pl-8">
              <List>
                <Each of={list()} key={(o) => o.key}>
                  {(o, i) => (
                    <Row first={i() === 0} testID={`welcome-provider-${o().source.id}-${o().item.ref}`}>
                      <Check checked={!!picked()[o().key]} onChange={(on) => picked.update((p) => ({ ...p, [o().key]: on }))} label={o().item.name} />
                      <Logo name={logoNamed(o().item.icon)} size={16} />
                      <View class="min-w-0 flex-1">
                        <Text class="text-[13px] text-foreground line-clamp-1">{o().item.name}</Text>
                        <Text class="text-[12px] text-muted-foreground line-clamp-1">{[o().item.host, `${o().source.name} · ${o().item.from}`].filter(Boolean).join(" · ")}</Text>
                      </View>
                      <Text class="font-mono text-[12px] text-muted-foreground">{o().item.key}</Text>
                    </Row>
                  )}
                </Each>
              </List>
            </View>
          </If>
        </If>
      </If>
    </Section>
  );
}

/** The folders the agents worked in, each with its sessions. */
function ProjectsLine() {
  const list = () => projects() ?? [];
  return (
    <Section label={tr("Projects")} testID="welcome-projects">
      <If when={projects()} else={<Text class="text-[13px] text-muted-foreground">{tr("Looking for projects…")}</Text>}>
        <If
          when={list().length > 0}
          else={<Head lead={<Icon name="folder-open" color="muted-foreground" size={16} />} title={tr("No projects yet")} sub={tr("The folders your agents work in show up on the Sessions page.")} />}
        >
          <Head
            testID="welcome-projects-title"
            lead={<Icon name="folder-open" color="fg-2" size={16} />}
            title={trn(list().length, "%d project on this computer", "%d projects on this computer", list().length)}
            sub={tr("From your %s sessions", once(projectAgents().map(agentName)).join(", "))}
            end={<Unroll label={tr("Review all %d", list().length)} open={!!open().projects} onPress={() => toggle("projects")} testID="welcome-projects-review" />}
          />
          <If when={open().projects}>
            <View class="pl-8">
              <List>
                <Each of={list()} key={(p) => p.path}>
                  {(p, i) => (
                    <Row first={i() === 0} testID={`welcome-project-${baseName(p().path)}`}>
                      <View class="min-w-0 flex-1">
                        <Text class="text-[13px] text-foreground line-clamp-1">{baseName(p().path)}</Text>
                        <Text class="text-[12px] text-muted-foreground line-clamp-1">{tilde(p().path)}</Text>
                      </View>
                      <Text class="text-[12px] text-muted-foreground">{trn(p().sessions, "%d session", "%d sessions", p().sessions)}</Text>
                    </Row>
                  )}
                </Each>
              </List>
            </View>
          </If>
        </If>
      </If>
    </Section>
  );
}

/** The window's first look on this computer, in its place. */
export function WelcomePage() {
  void look();
  const computer = Prefs.computerName();
  const said = () => (computer ? tr("Setting up %s.", "\u0001").split("\u0001") : [tr("Setting up this computer.")]);
  const n = () => chosen().length;
  return (
    <Scroll class="flex-1">
      <View testID="welcome" class="w-full max-w-[720px] self-center px-10 pb-8 pt-[72px]">
        <Text class="text-[26px] font-semibold text-foreground">{tr("Welcome to pansy")}</Text>
        <Text testID="welcome-computer" class="mb-6 mt-1 text-[14px] text-muted-foreground">
          {said()[0]}
          <If when={computer}>
            <Text class="text-foreground">{computer}</Text>
            {said()[1] ?? ""}
          </If>
        </Text>
        <AgentsLine />
        <ProvidersLine />
        <ProjectsLine />
        <View class="flex-row justify-end border-t border-line-2 pt-6">
          <BaseButton
            onPress={begin}
            disabled={isBusy("welcome")}
            testID="welcome-start"
            class={`h-[34px] flex-row items-center rounded-[8px] bg-primary px-4 hover:brightness-108 active:scale-96 disabled:opacity-40 ${MOTION}`}
          >
            <Text class="text-[14px] font-medium text-primary-foreground">{n() ? trn(n(), "Import %d provider and start", "Import %d providers and start", n()) : tr("Start")}</Text>
          </BaseButton>
        </View>
        <Text class="mt-10 text-[12px] text-muted-foreground">{tr("Everything runs on this computer: your keys, prompts and replies go only to the providers you pick.")}</Text>
      </View>
    </Scroll>
  );
}
