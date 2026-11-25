// 新任务 (phone-ui.md "9. 新任务"): a full-screen sheet asking what to do in
// a project (projects.list), on a machine, with an agent, a model and a
// workspace (this branch, a new branch, a new worktree from a base); sent,
// it makes the thread (thread.create) and opens it.
import { effect, resource, signal, Text, View } from "nsh";
import { Dialog } from "nsh/base";
import type { Project, Workspace } from "../net/types";
import { conn, machines, online } from "../state/store";
import { Composer } from "./Composer";
import { tr } from "./i18n";
import { ModelSheet } from "./ModelSheet";
import { newTask, openThread } from "./nav";
import { agentLogo, agentName, BottomSheet, Chip, Field, GlassButton, Group, GroupHead, GroupRow, Icon, Loading, Note, PickSheet, toast } from "./parts";

const AGENTS = ["codex", "claude-code"];

export function NewTask() {
  const machineId = signal("");
  const folder = signal("");
  const agent = signal("codex");
  const model = signal("");
  const effort = signal<string | undefined>(undefined);
  const ws = signal<Workspace["kind"]>("current");
  const base = signal("");
  const text = signal("");
  const sending = signal(false);
  const picking = signal<"machine" | "project" | "agent" | "model" | "workspace" | null>(null);

  effect(() => {
    const t = newTask();
    if (!t) return;
    const ms = machines();
    machineId.set(t.machine ?? ms.find((m) => online(m.id))?.id ?? ms[0]?.id ?? "");
    text.set("");
    picking.set(null);
  });

  const projects = resource(
    () => (newTask() && machineId() && online(machineId()) ? machineId() : null),
    async (id) => (await conn(id)!.call<{ projects: Project[] }>("projects.list")).projects,
  );
  effect(() => {
    const ps = projects();
    if (ps?.length && !ps.some((p) => p.folder === folder())) folder.set(ps[0].folder);
  });
  const project = () => projects()?.find((p) => p.folder === folder());
  effect(() => {
    const p = project();
    if (p) base.set(p.branch ?? "main");
  });

  const wsLabel = () => ({ current: tr("Current branch"), branch: tr("New branch"), worktree: tr("New worktree · from %s", base() || "main") })[ws()];
  const machineName = () => machines().find((m) => m.id === machineId())?.name ?? tr("Machine");

  const send = async () => {
    const c = conn(machineId());
    if (!c || !folder() || !text().trim() || sending()) return;
    sending.set(true);
    try {
      const r = await c.call<{ threadId: string }>("thread.create", {
        folder: folder(),
        agent: agent(),
        ...(model() ? { model: model() } : {}),
        ...(effort() ? { effort: effort() } : {}),
        mode: "full",
        workspace: ws() === "worktree" ? { kind: "worktree", base: base() || undefined } : { kind: ws() },
        text: text().trim(),
      });
      const id = machineId();
      newTask.set(null);
      openThread(id, r.threadId);
    } catch (e) {
      toast(tr("Couldn't start it: %s", String((e as Error).message ?? e)), { tone: "error" });
    } finally {
      sending.set(false);
    }
  };

  const heading = () => tr("Do something in %s?").split("%s");

  return (
    <Dialog open={newTask() !== null} onOpenChange={(e) => !e.open && newTask.set(null)} animation="slide" class="flex-1 bg-screen" testID="new-task-screen">
      <View class="flex-1 pb-[env(safe-area-inset-bottom)] pt-[env(safe-area-inset-top)]">
        <View class="h-[52px] flex-row items-center gap-2 px-4">
          <GlassButton icon="x" label={tr("Close")} onPress={() => newTask.set(null)} testID="new-task-close" />
          <Text class="flex-1 text-center text-[18px] font-bold text-fg">{tr("New task")}</Text>
          <View class="w-[38px]" />
        </View>
        <View class="flex-1 justify-center px-7">
          <If when={online(machineId())} else={<Note tone="warning" icon="wifi-off" text={tr("%s is offline", machineName())} />}>
            <If when={projects()} else={<Loading text={tr("Loading projects…")} />}>
              <Text class="text-center text-[26px] font-medium leading-[32px] tracking-[-0.6px] text-fg">
                {heading()[0]}
                <Text role="button" testID="new-task-project" class="underline" onPress={() => picking.set("project")}>
                  {project()?.name ?? tr("a project")}
                </Text>
                {heading()[1] ?? ""}
              </Text>
            </If>
          </If>
          <View class="mt-[22px] flex-row flex-wrap justify-center gap-2">
            <Chip icon="laptop" label={machineName()} chevron onPress={() => picking.set("machine")} testID="new-task-machine" />
            <Chip logo={agentLogo(agent())} label={agentName(agent())} chevron onPress={() => picking.set("agent")} testID="new-task-agent" />
            <Chip label={model() || tr("Agent's default")} chevron onPress={() => picking.set("model")} testID="new-task-model" />
            <If when={project()?.git !== false}>
              <Chip icon="git-branch" label={wsLabel()} chevron onPress={() => picking.set("workspace")} testID="new-task-workspace" />
            </If>
          </View>
        </View>
        <View class="px-3 pb-3">
          <Composer
            text={text()}
            onText={text.set}
            placeholder={tr("Ask anything")}
            agent={agent()}
            model={model() || tr("default")}
            onSend={() => void send()}
            onModel={() => picking.set("model")}
            disabled={!folder() || sending()}
          />
        </View>
      </View>
      <PickSheet
        open={picking() === "machine"}
        onClose={() => picking.set(null)}
        title={tr("Machine")}
        testID="new-task-pick-machine"
        value={machineId()}
        onPick={machineId.set}
        options={machines().map((m) => ({ value: m.id, label: m.name, sub: online(m.id) ? tr("Online") : tr("Offline"), icon: "laptop" as const }))}
      />
      <PickSheet
        open={picking() === "project"}
        onClose={() => picking.set(null)}
        title={tr("Project")}
        testID="new-task-pick-project"
        value={folder()}
        onPick={folder.set}
        options={(projects() ?? []).map((p) => ({ value: p.folder, label: p.name, sub: p.branch ? `${p.folder} · ${p.branch}` : p.folder, icon: "folder" as const }))}
      />
      <PickSheet
        open={picking() === "agent"}
        onClose={() => picking.set(null)}
        title={tr("Agent")}
        testID="new-task-pick-agent"
        value={agent()}
        onPick={(a) => {
          agent.set(a);
          model.set("");
        }}
        options={AGENTS.map((a) => ({ value: a, label: agentName(a), logo: agentLogo(a) }))}
      />
      <ModelSheet
        open={picking() === "model"}
        onClose={() => picking.set(null)}
        machine={machineId()}
        agent={agent()}
        model={model()}
        effort={effort()}
        onPick={(m, e) => {
          model.set(m);
          effort.set(e);
        }}
      />
      <BottomSheet open={picking() === "workspace"} onClose={() => picking.set(null)} title={tr("Workspace")} start={0.45} testID="new-task-pick-workspace">
        <View class="gap-[14px] px-5 pb-8 pt-[6px]">
          <Group>
            <GroupRow first icon="git-branch" title={tr("Current branch")} sub={project()?.branch} onPress={() => ws.set("current")} trailing={ws() === "current" ? <Icon name="check" color="primary" /> : undefined} testID="ws-current" />
            <GroupRow icon="plus" title={tr("New branch")} sub={tr("Switches over in the same folder")} onPress={() => ws.set("branch")} trailing={ws() === "branch" ? <Icon name="check" color="primary" /> : undefined} testID="ws-branch" />
            <GroupRow icon="layers" title={tr("New worktree")} sub={tr("A folder of its own; your changes at hand stay as they are")} onPress={() => ws.set("worktree")} trailing={ws() === "worktree" ? <Icon name="check" color="primary" /> : undefined} testID="ws-worktree" />
          </Group>
          <If when={ws() === "worktree"}>
            <GroupHead text={tr("Base")} />
            <Field value={base()} onChange={base.set} placeholder="origin/main" mono testID="ws-base" />
          </If>
        </View>
      </BottomSheet>
    </Dialog>
  );
}
