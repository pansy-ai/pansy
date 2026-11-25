// 改动 (phone-ui.md "8. 改动"): a full-screen diff of four scopes
// (git.diff): this turn, the last turn, uncommitted, all against the base;
// the turns' only in pansy's worktrees (the checkpoints are there alone).
// A changes card opens it on its own turn.
// Files can be folded; lines are 22pt with a 46pt gutter in T3's colors,
// scrolling sideways. Hold a line to select, tap more, and "Comment" puts
// them in the composer.
import { effect, resource, Scroll, signal, Text, token, View } from "nsh";
import { Dialog } from "nsh/base";
import { Haptics } from "nsh/mobile";
import type { Diff, DiffFile, DiffLine, DiffScope } from "../net/types";
import { conn } from "../state/store";
import { tr, trn } from "./i18n";
import { AddDel, Btn, GlassButton, Icon, Loading, Note, Seg, SHADOW } from "./parts";

const statusWord = (s: string) => ({ added: tr("added"), modified: tr("modified"), deleted: tr("deleted"), renamed: tr("renamed"), untracked: tr("added") })[s] ?? s;

/** Most lines drawn in one file; a longer one says how many more. */
const FILE_LINES = 1500;

type Sel = { file: string; line: number };
const key = (s: Sel) => `${s.file}:${s.line}`;

function Line(props: { file: string; index: number; line: DiffLine; selected: boolean; selecting: boolean; onSelect: () => void }) {
  const look = () =>
    props.line.kind === "+" ? { bg: "bg-d-add", bar: "bg-d-addbar" } : props.line.kind === "-" ? { bg: "bg-d-del", bar: "bg-d-delbar" } : { bg: "", bar: "" };
  return (
    <View
      testID={`diff-line-${props.file}-${props.index}`}
      onLongPress={props.onSelect}
      onPress={props.selecting ? props.onSelect : undefined}
      class={`h-[22px] flex-row ${look().bg}`}
      style={props.selected ? { outlineWidth: 2, outlineStyle: "solid", outlineColor: token("primary"), outlineOffset: -2 } : undefined}
    >
      <If when={look().bar}>
        <View class={`absolute bottom-0 left-0 top-0 w-1 ${look().bar}`} />
      </If>
      <Text class="w-[46px] pr-2 text-right font-mono text-[11px] leading-[22px] text-fg-3">{String((props.line.kind === "-" ? props.line.old : props.line.new) ?? "")}</Text>
      <Text class="font-mono text-[12px] leading-[22px] text-fg" style={{ numberOfLines: 1 }}>
        {`${props.line.kind === " " ? " " : props.line.kind} ${props.line.text}`}
      </Text>
    </View>
  );
}

function File(props: { file: DiffFile; open: boolean; onToggle: () => void; isSelected: (s: Sel) => boolean; selecting: boolean; onSelect: (s: Sel) => void }) {
  const lines = () => {
    const out: ({ header: string } | { line: DiffLine; i: number })[] = [];
    let n = 0;
    for (const h of props.file.hunks) {
      out.push({ header: h.header });
      for (const l of h.lines) {
        if (n >= FILE_LINES) return out;
        out.push({ line: l, i: n++ });
      }
    }
    return out;
  };
  const total = () => props.file.hunks.reduce((n, h) => n + h.lines.length, 0);
  return (
    <View>
      <View role="button" testID={`diff-file-${props.file.path}`} onPress={props.onToggle} class="mx-2 mt-2 h-11 flex-row items-center gap-2 rounded-[10px] border border-border bg-card px-3">
        <Icon name={props.open ? "chevron-down" : "chevron-right"} color="fg-3" size={14} />
        <Text class="flex-1 font-mono text-[12px] font-semibold text-fg line-clamp-1">{props.file.oldPath ? `${props.file.oldPath} → ${props.file.path}` : props.file.path}</Text>
        <AddDel add={props.file.add} del={props.file.del} size={12} mono />
        <Text class="text-[9px] font-bold text-fg-3">{statusWord(props.file.status)}</Text>
      </View>
      <If when={props.open}>
        <If when={!props.file.binary} else={<Text class="px-5 py-2 text-[13px] text-fg-2">{tr("A binary file")}</Text>}>
          <Scroll horizontal scrollbar={false} class="mt-[6px]">
            <View style={{ minWidth: "100%" }}>
              <Each of={lines()} key={(l) => ("header" in l ? `h:${l.header}` : `l:${l.i}`)}>
                {(l) => {
                  const item = l();
                  return "header" in item ? (
                    <Text class="h-[22px] bg-d-hunk pl-3 font-mono text-[12px] leading-[22px] text-d-hunkfg">{item.header}</Text>
                  ) : (
                    <Line
                      file={props.file.path}
                      index={item.i}
                      line={item.line}
                      selecting={props.selecting}
                      selected={props.isSelected({ file: props.file.path, line: item.i })}
                      onSelect={() => props.onSelect({ file: props.file.path, line: item.i })}
                    />
                  );
                }}
              </Each>
            </View>
          </Scroll>
          <If when={total() > FILE_LINES}>
            <Text class="px-5 py-2 text-[13px] text-fg-2">{tr("%d more lines not shown", total() - FILE_LINES)}</Text>
          </If>
        </If>
      </If>
    </View>
  );
}

/** The selected lines as a quote for the composer. */
function quote(diff: Diff, sel: Set<string>): string {
  const out: string[] = [];
  for (const f of diff.files) {
    let n = 0;
    const picked: string[] = [];
    let first: number | undefined;
    for (const h of f.hunks)
      for (const l of h.lines) {
        if (sel.has(`${f.path}:${n}`)) {
          picked.push(`${l.kind}${l.text}`);
          first ??= l.new ?? l.old;
        }
        n++;
      }
    if (picked.length) out.push(`${f.path}${first ? `:${first}` : ""}\n\`\`\`diff\n${picked.join("\n")}\n\`\`\``);
  }
  return out.join("\n\n") + "\n\n";
}

export function DiffView(props: { open: boolean; onClose: () => void; machine: string; thread: string; onComment: (text: string) => void; worktree: boolean; turn?: number; scope?: DiffScope }) {
  const first = (): DiffScope => (props.turn !== undefined ? "turn" : (props.scope ?? (props.worktree ? "turn" : "uncommitted")));
  const scope = signal<DiffScope>(first());
  // the turn a changes card asked for, until another scope is chosen
  const pinned = signal<number | undefined>(props.turn);
  effect(() => {
    if (!props.open) return;
    scope.set(first());
    pinned.set(props.turn);
  });
  const folded = signal<Set<string>>(new Set());
  const list = signal(true);
  const selected = signal<Set<string>>(new Set());
  const diff = resource(
    () => (props.open ? { s: scope(), n: scope() === "turn" ? pinned() : undefined, m: props.machine, t: props.thread } : null),
    async (src) => conn(src.m)!.call<Diff>("git.diff", { threadId: src.t, scope: src.s, ...(src.n !== undefined ? { turn: src.n } : {}) }),
  );
  const scopes = () => [
    ...(props.worktree
      ? [
          { value: "turn" as const, label: pinned() !== undefined ? tr("Turn %d", pinned()!) : tr("This turn") },
          { value: "lastTurn" as const, label: tr("Last turn") },
        ]
      : []),
    { value: "uncommitted" as const, label: tr("Uncommitted") },
    { value: "base" as const, label: tr("All") },
  ];
  const add = () => (diff()?.files ?? []).reduce((n, f) => n + f.add, 0);
  const del = () => (diff()?.files ?? []).reduce((n, f) => n + f.del, 0);
  const select = (s: Sel) => {
    const next = new Set(selected());
    if (next.has(key(s))) next.delete(key(s));
    else next.add(key(s));
    if (!selected().size) Haptics.impact("medium");
    selected.set(next);
  };
  return (
    <Dialog open={props.open} onOpenChange={(e) => !e.open && props.onClose()} animation="slide" class="flex-1 bg-screen" testID="diff-view">
      <View class="flex-1 pt-[env(safe-area-inset-top)]">
        <View class="flex-row items-center gap-2 px-4 pb-[6px] pt-1">
          <GlassButton icon="x" label={tr("Close")} onPress={props.onClose} testID="diff-close" />
          <View class="flex-1 items-center">
            <Text class="text-[18px] font-bold text-fg">{tr("Changes")}</Text>
            <View class="flex-row items-center gap-1">
              <AddDel add={add()} del={del()} size={13} />
              <Text class="text-[13px] text-fg-2">· {trn(diff()?.files.length ?? 0, "1 file", "%d files", diff()?.files.length ?? 0)}</Text>
            </View>
          </View>
          <GlassButton icon="list-ordered" label={tr("Files")} onPress={() => list.set(!list())} testID="diff-files" />
        </View>
        <View class="px-4 pb-2 pt-1">
          <Seg
            testID="diff-scope"
            value={scope()}
            onChange={(s) => {
              selected.set(new Set());
              pinned.set(undefined);
              scope.set(s);
            }}
            options={scopes()}
          />
        </View>
        <Scroll class="flex-1">
          <If when={diff()} else={<If when={diff.error()} else={<Loading text={tr("Reading the changes…")} />}><View class="p-5"><Note tone="danger" text={String((diff.error() as Error)?.message ?? "")} action={{ label: tr("Retry"), onPress: () => diff.refetch() }} /></View></If>}>
            <If when={(diff()?.files.length ?? 0) > 0} else={<Text class="py-16 text-center text-[16px] text-fg-2">{tr("No changes")}</Text>}>
              <If when={list()}>
                <View class="mx-4 mb-2 gap-1 rounded-2xl bg-grouped px-4 py-3">
                  <Each of={diff()?.files ?? []} key={(f) => f.path}>
                    {(f) => (
                      <View class="flex-row items-center gap-2">
                        <Text class="flex-1 font-mono text-[12px] text-fg line-clamp-1">{f().path}</Text>
                        <Text class="text-[11px] text-fg-3">{statusWord(f().status)}</Text>
                        <AddDel add={f().add} del={f().del} size={12} mono />
                      </View>
                    )}
                  </Each>
                </View>
              </If>
              <View class="pb-40">
                <Each of={diff()?.files ?? []} key={(f) => f.path}>
                  {(f) => (
                    <File
                      file={f()}
                      open={!folded().has(f().path)}
                      onToggle={() => {
                        const next = new Set(folded());
                        if (next.has(f().path)) next.delete(f().path);
                        else next.add(f().path);
                        folded.set(next);
                      }}
                      selecting={selected().size > 0}
                      isSelected={(s) => selected().has(key(s))}
                      onSelect={select}
                    />
                  )}
                </Each>
                <If when={diff()?.truncated}>
                  <Text class="px-5 py-3 text-[13px] text-fg-2">{tr("The diff is cut short: it is too long for the phone")}</Text>
                </If>
              </View>
            </If>
          </If>
        </Scroll>
        <If when={selected().size > 0}>
          <View class="absolute bottom-0 left-0 right-0 flex-row justify-center gap-2 pb-[calc(env(safe-area-inset-bottom)+24px)]" style={{ pointerEvents: "box-none" }}>
            <Btn
              icon="message-square-text"
              label={trn(selected().size, "Comment on 1 line", "Comment on %d lines", selected().size)}
              testID="diff-comment"
              onPress={() => {
                const d = diff();
                if (d) props.onComment(quote(d, selected()));
                selected.set(new Set());
                props.onClose();
              }}
              class="self-center"
            />
            <View style={{ boxShadow: SHADOW.glass() }}>
              <GlassButton icon="x" label={tr("Clear the selection")} onPress={() => selected.set(new Set())} size={48} />
            </View>
          </View>
        </If>
      </View>
    </Dialog>
  );
}
