// The Agents page's modals, as magpie's: Disconnect, which shows what goes
// back before it is done, and Manage agents, which hides or shows many at
// once by search and selection (only the page's list changes, never an
// agent's config). And the list of changes both the modal and an opened
// row show.
import { keymap, signal, Text, View } from "nsh";
import { Checkbox } from "nsh/base";
import { tr, trn } from "./i18n";
import { arrange, changes, disconnectIn, disconnectNow, disconnecting, hiddenIds, managing, saveArrangement } from "./agentState";
import { question, type FileChange } from "./model";
import { agentLogo, Icon, Input, Logo, Modal, PrimaryButton, TextButton } from "./parts";

interface Line {
  op: string;
  text: string;
  tone: "del" | "add";
}

function lines(f: FileChange, back: boolean): Line[] {
  const out: Line[] = [];
  for (const k of f.keys) {
    // back: what disconnecting does, pansy's value out and the old one in
    const [gone, come] = back ? [k.after, k.before] : [k.before, k.after];
    if (gone !== null) out.push({ op: "−", text: `${k.key} = ${gone}`, tone: "del" });
    if (come !== null) out.push({ op: "+", text: `${k.key} = ${come}`, tone: "add" });
  }
  return out;
}

/** The keys pansy set, file by file: what was there and what is now, or with `back`, what disconnecting puts back. */
export function ChangesList(props: { changes: FileChange[]; back?: boolean }) {
  return (
    <View class="gap-[10px]">
      <Each of={props.changes} key={(f) => f.path}>
        {(f) => (
          <View class="overflow-hidden rounded-[8px] border border-border bg-card">
            <View class="flex-row items-center gap-2 border-b border-line-2 px-[10px] py-[6px]">
              <Text class="min-w-0 flex-1 font-mono text-[11.5px] text-foreground line-clamp-1">{f().path}</Text>
              <If when={f().created}>
                <Text class="text-[11px] text-muted-foreground">{props.back ? tr("removed") : tr("new file")}</Text>
              </If>
            </View>
            <View class="py-1">
              <Each of={lines(f(), !!props.back)} key={(l, i) => `${i}${l.text}`}>
                {(l) => (
                  <View class="flex-row items-start gap-2 px-[10px] py-[1px]">
                    <Text class={`w-[10px] text-center font-mono text-[11.5px] leading-[18px] ${l().tone === "del" ? "text-destructive" : "text-success"}`}>{l().op}</Text>
                    <Text class={`min-w-0 flex-1 font-mono text-[11.5px] leading-[18px] ${l().tone === "del" ? "text-destructive" : "text-success"}`}>{l().text}</Text>
                  </View>
                )}
              </Each>
            </View>
          </View>
        )}
      </Each>
    </View>
  );
}

/** Disconnect's question, in the window it was asked in (`where`). */
export function DisconnectDialog(props: { where: "main" | "panel"; width?: number }) {
  const a = () => (disconnectIn() === props.where ? disconnecting() : null);
  const ch = () => (a() ? changes()[a()!.id] : undefined);
  const close = () => disconnecting.set(null);
  return (
    <If when={a()}>
      <Modal
        testID="disconnect"
        width={props.width ?? 560}
        icon={<Logo name={agentLogo(a()?.id ?? "")} size={18} />}
        title={tr("Disconnect %s from pansy?", a()?.name ?? "")}
        onClose={close}
        foot={
          <>
            <View class="flex-1" />
            <TextButton label={tr("Cancel")} testID="disconnect-cancel" onPress={close} />
            <PrimaryButton
              danger
              label={tr("Disconnect and restore")}
              testID="disconnect-confirm"
              onPress={() => {
                const agent = a();
                close();
                if (agent) disconnectNow(agent);
              }}
            />
          </>
        }
      >
        <Text class="text-[12.5px] text-fg-2">{tr("%s goes back to how it was before it was connected; these are the places put back.", a()?.name ?? "")}</Text>
        <If when={ch() !== undefined} else={<Text class="py-[14px] text-[12px] text-muted-foreground">{tr("Reading what changes…")}</Text>}>
          <If
            when={(ch() ?? []).length > 0}
            else={
              <Text class="text-[12.5px] text-fg-2">
                {tr("pansy takes out everything it wrote into %s's config — its endpoint, key and models — and puts back the settings %s had before. pansy's providers stay as they are.", a()?.name ?? "", a()?.name ?? "")}
              </Text>
            }
          >
            <ChangesList changes={ch() ?? []} back />
          </If>
        </If>
      </Modal>
    </If>
  );
}

function ManageAgents() {
  const selected = signal<string[]>([]);
  const search = signal("");
  const result = signal("");
  const saving = signal(false);
  const matching = () => {
    const q = search().trim().toLowerCase();
    return arrange().all.filter((a) => `${a.name} ${a.id}`.toLowerCase().includes(q));
  };
  const has = (id: string) => selected().includes(id);
  const set = (ids: string[]) => selected.set([...new Set(ids)]);
  const canHide = () => selected().some((id) => !hiddenIds().includes(id));
  const canShow = () => selected().some((id) => hiddenIds().includes(id));
  const save = async (hide: boolean) => {
    const ids = selected().filter((id) => hiddenIds().includes(id) !== hide);
    if (!ids.length || saving()) return;
    const next = new Set(hiddenIds());
    for (const id of ids) {
      if (hide) next.add(id);
      else next.delete(id);
    }
    saving.set(true);
    result.set("");
    const order = arrange().all.map((a) => a.id);
    if (await saveArrangement(order, [...next])) {
      selected.set([]);
      result.set(hide ? trn(ids.length, "%d agent hidden", "%d agents hidden", ids.length) : trn(ids.length, "%d agent unhidden", "%d agents unhidden", ids.length));
    }
    saving.set(false);
  };
  const close = () => {
    managing.set(false);
    selected.set([]);
    search.set("");
    result.set("");
  };
  return (
    <If when={managing()}>
      <Modal
        testID="agents-manage"
        width={520}
        title={tr("Manage agents")}
        onClose={close}
        foot={
          <>
            <TextButton label={tr("Hide selected")} testID="manage-hide" disabled={!canHide() || saving()} onPress={() => void save(true)} />
            <TextButton label={tr("Unhide selected")} testID="manage-unhide" disabled={!canShow() || saving()} onPress={() => void save(false)} />
            <View class="flex-1" />
            <TextButton label={tr("Done")} testID="manage-done" onPress={close} />
          </>
        }
      >
        <Text class="text-[12.5px] text-fg-2">{tr("Hiding only changes this list, not an agent's connection or settings. Unhidden agents may stay under Not set up.")}</Text>
        <Input value={search()} onChange={(v) => search.set(v)} placeholder={tr("Search agents…")} testID="manage-search" />
        <View class="-mx-[7px] flex-row flex-wrap">
          <TextButton label={tr("Select search results")} onPress={() => set([...selected(), ...matching().map((a) => a.id)])} />
          <TextButton
            label={tr("Invert search selection")}
            onPress={() => {
              const ids = matching().map((a) => a.id);
              set([...selected().filter((id) => !ids.includes(id)), ...ids.filter((id) => !has(id))]);
            }}
          />
          <TextButton label={tr("Select unconnected agents")} onPress={() => set(matching().filter((a) => !a.connected).map((a) => a.id))} />
          <TextButton label={tr("Clear selection")} onPress={() => selected.set([])} />
        </View>
        <Text class="text-[11.5px] text-muted-foreground">{trn(selected().length, "%d agent selected", "%d agents selected", selected().length)}</Text>
        <View class="overflow-hidden rounded-[10px] border border-border">
          <Each of={matching()} key={(a) => a.id}>
            {(a, i) => (
              <Checkbox
                checked={has(a().id)}
                accessibilityLabel={a().name}
                testID={`manage-${a().id}`}
                onChange={({ checked }) => set(checked ? [...selected(), a().id] : selected().filter((id) => id !== a().id))}
                class={`h-[38px] flex-row items-center gap-[10px] px-3 hover:bg-card-2 ${i() === 0 ? "" : "border-t border-line-2"}`}
                indicatorClass="h-[15px] w-[15px] items-center justify-center rounded-[4px] border border-border bg-card checked:border-primary checked:bg-primary"
                indicator={has(a().id) ? <Icon name="tick" color="primary-foreground" size={10} /> : undefined}
              >
                <Logo name={agentLogo(a().id)} size={16} />
                <Text class="flex-1 text-[12.5px] text-foreground">{a().name}</Text>
                <Text class="text-[11.5px] text-muted-foreground">{hiddenIds().includes(a().id) ? tr("Hidden") : tr("Not hidden")}</Text>
              </Checkbox>
            )}
          </Each>
          <If when={matching().length === 0}>
            <Text class="px-3 py-[10px] text-[12.5px] text-muted-foreground">{tr("No matching agents")}</Text>
          </If>
        </View>
        <If when={result()}>
          <Text testID="manage-result" class="text-[12px] text-muted-foreground">
            {result()}
          </Text>
        </If>
      </Modal>
    </If>
  );
}

/** Above the window, beside the other modals. */
export function AgentModals() {
  keymap.bind("escape", () => {
    if (question()) return;
    if (disconnecting()) disconnecting.set(null);
    else if (managing()) managing.set(false);
  });
  return (
    <>
      <DisconnectDialog where="main" />
      <ManageAgents />
    </>
  );
}
