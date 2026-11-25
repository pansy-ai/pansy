// Git (phone-ui.md "6. Git", "7. 提交"): the branch and what it holds
// (git.status), commit and push as the main action with its other ways in
// a menu, then the commit sheet: a message (or the gateway's, git.generate),
// the files to take, push and PR, and the steps as they go (git.progress).
import { effect, openUrl, resource, signal, Text, View } from "nsh";
import { Menu, MenuItem } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { GitAction, GitProgress, GitStatus } from "../net/types";
import { conn } from "../state/store";
import { tr, trn } from "./i18n";
import { AddDel, BottomSheet, Btn, Field, GlassButton, Group, GroupHead, GroupRow, Icon, Loading, Note, Spinner, toast, Toggle, BLUR, type IconName } from "./parts";

type Way = "commit_push" | "commit" | "push" | "commit_push_pr";

const wayLabel = (w: Way) => ({ commit_push: tr("Commit and push"), commit: tr("Commit only"), push: tr("Push only"), commit_push_pr: tr("Commit, push and open a PR") })[w];

export function GitSheet(props: { open: boolean; onClose: () => void; machine: string; thread: string; model: string; onDiff: () => void; onCommit: (way: Way, status: GitStatus) => void }) {
  const status = resource(
    () => (props.open ? `${props.machine}/${props.thread}` : null),
    async () => conn(props.machine)!.call<GitStatus>("git.status", { threadId: props.thread }),
  );
  const confirm = signal<Way | null>(null);
  const go = (w: Way) => {
    const s = status();
    if (!s) return;
    // on the default branch: press once more to mean it
    if (s.onDefault && confirm() !== w && w !== "push") {
      confirm.set(w);
      Haptics.notification("warning");
      return;
    }
    confirm.set(null);
    props.onCommit(w, s);
  };
  const pulling = signal(false);
  const pull = async () => {
    pulling.set(true);
    try {
      const r = await conn(props.machine)!.call<{ pull?: string }>("git.run", { threadId: props.thread, action: "pull" });
      toast(r.pull || tr("Up to date"));
      Haptics.notification("success");
      status.refetch();
    } catch (e) {
      toast(tr("Couldn't pull: %s", String((e as Error).message ?? e)), { tone: "error" });
    } finally {
      pulling.set(false);
    }
  };
  const add = () => (status()?.files ?? []).reduce((n, f) => n + f.add, 0);
  const del = () => (status()?.files ?? []).reduce((n, f) => n + f.del, 0);
  const subtitle = () => {
    const s = status();
    if (!s) return "";
    const parts = [];
    if (s.ahead) parts.push(tr("%d ahead", s.ahead));
    if (s.behind) parts.push(tr("%d behind", s.behind));
    parts.push(s.files.length ? trn(s.files.length, "1 file uncommitted", "%d files uncommitted", s.files.length) : tr("Nothing uncommitted"));
    if (s.worktree) parts.push(tr("worktree"));
    return parts.join(" · ");
  };
  return (
    <BottomSheet open={props.open} onClose={props.onClose} title={status()?.branch ?? tr("Git")} titleMono subtitle={subtitle()} start={0.55} testID="git-sheet">
      <If when={status()} else={<If when={status.error()} else={<Loading text={tr("Reading the repository…")} />}>
        <View class="px-5"><Note tone="danger" text={tr("Couldn't read it: %s", String((status.error() as Error)?.message ?? ""))} action={{ label: tr("Retry"), onPress: () => status.refetch() }} /></View>
      </If>}>
        <View class="gap-[14px] px-5 pb-10 pt-[6px]">
          <If when={status()?.onDefault}>
            <Note tone="warning" testID="git-on-default" text={confirm() ? tr("You're on %s. Press again to commit there anyway.", status()?.defaultBranch ?? "main") : tr("You're on %s", status()?.defaultBranch ?? "main")} />
          </If>
          <View class="flex-row gap-2">
            <Btn tone="prominent" icon="upload" label={wayLabel("commit_push")} onPress={() => go("commit_push")} class="flex-1" testID="git-commit-push" disabled={!status()?.files.length && !status()?.ahead} />
            <Menu
              placement="bottom"
              align="end"
              trigger={
                <View role="button" accessibilityLabel={tr("More ways")} testID="git-ways" class="h-12 w-12 items-center justify-center rounded-full bg-prominent">
                  <Icon name="chevron-down" color="prominent-fg" size={16} />
                </View>
              }
              popupClass="min-w-[240px] rounded-[14px] border border-border bg-glass p-1"
              popupStyle={BLUR}
            >
              <Each of={["commit", "push", "commit_push_pr"] as Way[]}>
                {(w) => (
                  <MenuItem onSelect={() => go(w())} testID={`git-way-${w()}`} class="min-h-11 flex-row items-center rounded-[10px] px-4 focus:bg-line active:bg-line">
                    <Text class="text-[16px] text-fg">{wayLabel(w())}</Text>
                  </MenuItem>
                )}
              </Each>
            </Menu>
          </View>
          <Group>
            <If
              when={status()?.pr}
              else={<GroupRow first icon="git-pull-request" title={tr("Open a PR")} sub={tr("Into %s; %s writes the title and description", status()?.defaultBranch ?? "main", props.model)} chevron onPress={() => go("commit_push_pr")} testID="git-open-pr" />}
            >
              <GroupRow first icon="git-pull-request" title={tr("PR #%d", status()?.pr?.number ?? 0)} sub={status()?.pr?.state ?? ""} chevron onPress={() => void openUrl(status()!.pr!.url).catch(() => {})} testID="git-pr" />
            </If>
            <GroupRow icon="file-diff" title={tr("View changes")} lead={undefined} sub={trn(status()?.files.length ?? 0, "1 file", "%d files", status()?.files.length ?? 0)} trailing={<AddDel add={add()} del={del()} size={14} />} chevron onPress={props.onDiff} testID="git-diff" />
            <GroupRow
              icon="arrow-down-to-line"
              title={tr("Pull latest")}
              sub={pulling() ? tr("Pulling…") : !status()?.upstream ? tr("Not pushed yet") : status()?.behind ? tr("%s has %d new commits", status()?.upstream ?? "origin", status()?.behind ?? 0) : tr("Up to date with %s", status()?.upstream ?? "origin")}
              tone={status()?.upstream && !pulling() ? undefined : "muted"}
              onPress={status()?.upstream && !pulling() ? () => void pull() : undefined}
              testID="git-pull"
            />
          </Group>
        </View>
      </If>
    </BottomSheet>
  );
}

type Step = { step: "commit" | "push" | "pr"; status: "waiting" | "running" | "done" | "failed"; detail?: string };

const stepsOf = (action: GitAction): Step["step"][] =>
  ({ commit: ["commit"], push: ["push"], commit_push: ["commit", "push"], create_pr: ["pr"], commit_push_pr: ["commit", "push", "pr"] })[action] as Step["step"][];

const stepLabel = (s: Step["step"]) => ({ commit: tr("Commit"), push: tr("Push"), pr: tr("Open PR") })[s];

export function CommitSheet(props: { open: boolean; onClose: () => void; machine: string; thread: string; model: string; way: Way; status: GitStatus | null }) {
  const message = signal("");
  const picked = signal<Set<string>>(new Set());
  const push = signal(true);
  const pr = signal(false);
  const prTitle = signal("");
  const generating = signal(false);
  const steps = signal<Step[] | null>(null);
  const result = signal<{ commit?: string; pr?: { number: number; url: string } } | null>(null);
  const failed = signal<string | null>(null);

  effect(() => {
    if (!props.open) return;
    message.set("");
    picked.set(new Set((props.status?.files ?? []).map((f) => f.path)));
    push.set(props.way !== "commit");
    pr.set(props.way === "commit_push_pr");
    prTitle.set("");
    steps.set(null);
    result.set(null);
    failed.set(null);
  });

  effect(() => {
    if (!props.open) return;
    const c = conn(props.machine);
    if (!c) return;
    const stop = c.on("git.progress", (p: GitProgress) => {
      if (p.threadId !== props.thread || !steps()) return;
      steps.set(steps()!.map((s) => (s.step === p.step ? { ...s, status: p.status, detail: p.detail } : s)));
    });
    return stop;
  });

  const files = () => props.status?.files ?? [];
  const onlyPush = () => props.way === "push";
  const action = (): GitAction => (onlyPush() ? "push" : pr() ? "commit_push_pr" : push() ? "commit_push" : "commit");

  const generate = async () => {
    generating.set(true);
    try {
      const r = await conn(props.machine)!.call<{ title: string; body: string }>("git.generate", { threadId: props.thread, kind: "commit" });
      message.set(r.body ? `${r.title}\n\n${r.body}` : r.title);
      Haptics.selection();
    } catch (e) {
      toast(tr("Couldn't write one: %s", String((e as Error).message ?? e)), { tone: "error" });
    } finally {
      generating.set(false);
    }
  };

  // the steps done stay done: a push that failed after its commit pushes
  // again (create_pr pushes too), a failed commit goes back to the form
  const retry = () => {
    const committed = steps()?.some((s) => s.step === "commit" && s.status === "done");
    if (!committed) return steps.set(null);
    void run(pr() ? "create_pr" : "push");
  };

  const run = async (rest?: GitAction) => {
    const a = rest ?? action();
    steps.set(rest ? steps()!.map((s) => (s.status === "done" ? s : { step: s.step, status: "waiting" })) : stepsOf(a).map((step) => ({ step, status: "waiting" })));
    failed.set(null);
    try {
      const r = await conn(props.machine)!.call<{ commit?: string; pr?: { number: number; url: string } }>("git.run", {
        threadId: props.thread,
        action: a,
        files: onlyPush() ? undefined : [...picked()],
        message: message().trim() || undefined,
        pr: pr() ? { title: prTitle().trim() || undefined, base: props.status?.defaultBranch } : undefined,
      });
      result.set({ ...result(), ...r });
      steps.set(steps()!.map((s) => (s.status === "failed" ? s : { ...s, status: "done" })));
      Haptics.notification("success");
    } catch (e) {
      failed.set(String((e as Error).message ?? e));
      steps.set(steps()!.map((s) => (s.status === "running" || s.status === "waiting" ? { ...s, status: s.status === "running" ? "failed" : "waiting" } : s)));
      Haptics.notification("error");
    }
  };

  const toggle = (path: string) => {
    const next = new Set(picked());
    if (next.has(path)) next.delete(path);
    else next.add(path);
    picked.set(next);
  };

  const stepIcon = (s: Step): { icon: IconName; color: "emerald" | "danger" | "fg-3" } => (s.status === "done" ? { icon: "circle-check", color: "emerald" } : s.status === "failed" ? { icon: "circle-x", color: "danger" } : { icon: "circle-dashed", color: "fg-3" });

  return (
    <BottomSheet
      open={props.open}
      onClose={props.onClose}
      title={onlyPush() ? tr("Push") : tr("Commit")}
      subtitle={props.status?.branch}
      start={0.8}
      testID="commit-sheet"
      left={<GlassButton icon="chevron-left" label={tr("Back")} onPress={props.onClose} />}
      right={<View />}
    >
      <View class="gap-[14px] px-5 pb-10 pt-[6px]">
        <If when={!steps()}>
          <If when={!onlyPush()}>
            <View>
              <Field value={message()} onChange={message.set} multiline placeholder={tr("Leave it empty and %s writes it", props.model)} testID="commit-message" class="min-h-24 pb-10" />
              <View class="absolute bottom-2 right-2">
                <View
                  role="button"
                  testID="commit-generate"
                  onPress={() => void generate()}
                  class="h-7 flex-row items-center gap-1 rounded-2xl border border-border bg-glass px-3 active:opacity-70"
                >
                  <If when={generating()} else={<Icon name="sparkles" color="fg" size={13} />}>
                    <Spinner size={13} />
                  </If>
                  <Text class="text-[13px] font-medium text-fg">{message() ? tr("Write again") : tr("Generate")}</Text>
                </View>
              </View>
            </View>
            <View class="flex-row justify-between px-1">
              <Text class="text-[13px] text-fg-2">{trn(files().length, "1 file", "%d files", files().length)}</Text>
              <Text
                role="button"
                testID="commit-all"
                class="text-[13px] text-primary"
                onPress={() => picked.set(picked().size === files().length ? new Set() : new Set(files().map((f) => f.path)))}
              >
                {picked().size === files().length ? tr("Select none") : tr("Select all")}
              </Text>
            </View>
            <Group>
              <Each of={files()} key={(f) => f.path}>
                {(f, i) => {
                  const dir = () => f().path.slice(0, f().path.lastIndexOf("/") + 1);
                  const name = () => f().path.slice(f().path.lastIndexOf("/") + 1);
                  return (
                    <View role="checkbox" accessibilityState={{ checked: picked().has(f().path) }} testID={`commit-file-${f().path}`} onPress={() => toggle(f().path)} class="min-h-16 flex-row items-center gap-3 px-4">
                      <If when={i() > 0}>
                        <View class="absolute left-4 right-0 top-0 h-px bg-line" />
                      </If>
                      <Icon name={picked().has(f().path) ? "square-check" : "square"} color={picked().has(f().path) ? "primary" : "fg-3"} size={20} />
                      <Text class="min-w-0 flex-1 font-mono text-[13px] text-fg line-clamp-2">
                        <Text class="text-fg-3">{dir()}</Text>
                        {name()}
                      </Text>
                      <AddDel add={f().add} del={f().del} size={12} mono />
                    </View>
                  );
                }}
              </Each>
            </Group>
          </If>
          <Group>
            <If when={!onlyPush()}>
              <GroupRow first title={tr("Push")} trailing={<Toggle on={push()} onChange={(v) => { push.set(v); if (!v) pr.set(false); }} label={tr("Push")} testID="commit-push" />} />
            </If>
            <GroupRow first={onlyPush()} title={tr("Open a PR")} sub={tr("Into %s", props.status?.defaultBranch ?? "main")} trailing={<Toggle on={pr()} onChange={(v) => { pr.set(v); if (v) push.set(true); }} label={tr("Open a PR")} testID="commit-pr" />} />
            <If when={pr()}>
              <View class="px-4 pb-3">
                <Field value={prTitle()} onChange={prTitle.set} placeholder={tr("PR title (empty: %s writes it)", props.model)} testID="commit-pr-title" class="bg-card" />
              </View>
            </If>
          </Group>
          <Btn
            label={onlyPush() ? tr("Push") : trn(picked().size, "Commit 1 file", "Commit %d files", picked().size)}
            onPress={() => void run()}
            disabled={!onlyPush() && picked().size === 0}
            testID="commit-run"
          />
        </If>
        <If when={steps()}>
          <GroupHead text={result() ? tr("Done") : failed() ? tr("Failed") : tr("In progress")} />
          <Group class="py-[6px]">
            <Each of={steps() ?? []} key={(s) => s.step}>
              {(s, i) => (
                <GroupRow
                  first={i() === 0}
                  lead={s().status === "running" ? <Spinner size={18} /> : <Icon name={stepIcon(s()).icon} color={stepIcon(s()).color} size={18} />}
                  title={stepLabel(s().step)}
                  sub={s().detail}
                  testID={`step-${s().step}-${s().status}`}
                  trailing={s().step === "commit" && result()?.commit ? <Text class="font-mono text-[12px] text-fg-2">{result()!.commit!.slice(0, 7)}</Text> : undefined}
                />
              )}
            </Each>
          </Group>
          <If when={failed()}>
            <Note tone="danger" text={failed() ?? ""} action={{ label: tr("Retry"), onPress: retry }} />
          </If>
          <If when={result()?.pr}>
            <Btn tone="outline" icon="external-link" label={tr("Open PR #%d in the browser", result()?.pr?.number ?? 0)} onPress={() => void openUrl(result()!.pr!.url).catch(() => {})} testID="commit-open-pr" />
          </If>
          <If when={result()}>
            <Btn label={tr("Done")} onPress={props.onClose} testID="commit-done" />
          </If>
        </If>
      </View>
    </BottomSheet>
  );
}

export type { Way };
