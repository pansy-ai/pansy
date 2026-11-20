// The gateway, as magpie's Gateway page shows it: whether it runs and on
// what, sharing it on the local network, its port, the keys resting after a
// failure, and every model it serves (the ids agents ask for), each copied
// in a click.
import { Clipboard, signal, Text, View } from "nsh";
import { Gateway, Settings } from "pansy/native";
import { tr, trn } from "./i18n";
import { act, agents, choices, contextOf, gateway, groupProviders, groups, isBusy, loadGateway, loadSettings, providerOf, settings, toast, type Choice, type SettingsView } from "./model";
import { Badge, Icon, IconButton, Input, List, Logo, LogoStack, Mono, Page, PillButton, PrimaryButton, providerLogo, Row, RowHead, Segs, TextButton } from "./parts";

function set(key: keyof SettingsView, value: string | boolean, restart = false) {
  void act(`setting:${key}`, async () => {
    await Settings.set(key, String(value));
    if (restart) {
      await Gateway.restart();
      toast(tr("The gateway restarted"));
    }
  }).then(() => {
    void loadSettings();
    void loadGateway();
  });
}

function copy(text: string) {
  void Clipboard.writeText(text).then(() => toast(tr("Copied")));
}

function ModelRow(props: { choice: Choice; first: boolean }) {
  const c = () => props.choice;
  const group = () => groups().find((g) => `group/${g.id}` === c().id);
  const members = () => (group() ? groupProviders(group()!) : []);
  const over = signal(false);
  return (
    <Row first={props.first} class="min-h-[46px] py-[6px]" onHover={(on) => over.set(on)}>
      <View class="w-[30px] items-center">
        <If when={c().group} else={<Logo name={providerLogo(providerOf(c().id)?.preset)} size={18} />}>
          <LogoStack names={members().map((p) => providerLogo(p.preset))} size={16} />
        </If>
      </View>
      <View class="min-w-0 flex-1 gap-[1px]">
        <Mono text={c().id} class="text-[13px] line-clamp-1" />
        <Text class="text-[12px] text-muted-foreground line-clamp-1">
          {c().group ? [tr("%s · routing group", c().name), ...(members().length ? [members().map((p) => p.name).join(", ")] : [])].join(" · ") : `${c().name} · ${c().provider}`}
        </Text>
      </View>
      <If when={contextOf(c().id)}>
        <Badge label={contextOf(c().id) ?? ""} />
      </If>
      <View class={over() ? "" : "opacity-0"}>
        <IconButton icon="copy-s" label={tr("Copy the id")} onPress={() => copy(c().id)} />
      </View>
    </Row>
  );
}

export function GatewayPage() {
  const g = () => gateway();
  const s = () => settings();
  const find = signal("");
  const port = signal<string | null>(null);
  const models = () => {
    const q = find().trim().toLowerCase();
    // Routing groups first, as the picker lists them.
    const found = choices().filter((c) => !q || c.id.toLowerCase().includes(q) || c.name.toLowerCase().includes(q));
    return [...found.filter((c) => c.group), ...found.filter((c) => !c.group)];
  };
  const routed = () => agents().filter((a) => a.connected).length;
  return (
    <Page testID="page-gateway">
      <List>
        <Row first class="min-h-[58px] py-[10px]">
          <View class="h-[30px] w-[30px] items-center justify-center rounded-[8px] bg-pill">
            <Icon name="nav-gateway" color="fg-2" size={16} />
          </View>
          <View class="min-w-0 flex-1 gap-[1px]">
            <View class="flex-row items-center gap-[7px]">
              <Text class="text-[13px] font-medium text-foreground">{tr("Gateway")}</Text>
              <Text testID="gateway-status" class={`text-[12px] ${g()?.running ? "text-success" : "text-warning"}`}>
                {g()?.running ? (g()?.own ? tr("running") : tr("running in another pansy")) : tr("stopped")}
              </Text>
            </View>
            <Text class="text-[12px] text-muted-foreground">{[trn(choices().length, "%d model", "%d models", choices().length), trn(routed(), "%d agent routed through it", "%d agents routed through it", routed()), tr("four APIs, one URL")].join(" · ")}</Text>
          </View>
          <PillButton label={tr("Restart gateway")} testID="gateway-restart" disabled={isBusy("gateway")} onPress={() => void act("gateway", () => Gateway.restart()).then(() => loadGateway())} />
        </Row>
        <If when={g()?.error}>
          <Row class="bg-destructive-soft">
            <Icon name="triangle-alert" color="destructive" size={14} />
            <Text class="flex-1 text-[12px] text-destructive">{tr("The gateway could not start: %s", g()?.error ?? "")}</Text>
          </Row>
        </If>
        <If when={s()}>
          <Row class="min-h-[44px]">
            <Text class="flex-1 text-[13px] text-foreground">{tr("Share on the local network")}</Text>
            <Segs
              testID="setting-lan"
              value={s()!.lan ? "lan" : "local"}
              onChange={(v) => set("lan", v === "lan", true)}
              options={[
                { value: "local", label: tr("This computer") },
                { value: "lan", label: tr("Local network") },
              ]}
            />
          </Row>
          <Row class="min-h-[44px]">
            <Text class="w-[110px] text-[12px] text-muted-foreground">{s()!.lan ? tr("Every computer") : tr("This computer")}</Text>
            <Mono text={g()?.url ?? ""} testID="gateway-url" />
            <IconButton icon="copy-s" label={tr("Copy")} onPress={() => copy(g()?.url ?? "")} />
            <View class="flex-1" />
          </Row>
          <If when={s()!.lan}>
            <Row class="min-h-[44px]">
              <Text class="w-[110px] text-[12px] text-muted-foreground">{tr("Gateway key")}</Text>
              <Mono text={s()!.gatewayKey} testID="gateway-key" />
              <IconButton icon="copy-s" label={tr("Copy")} onPress={() => copy(s()!.gatewayKey)} />
              <View class="flex-1" />
            </Row>
          </If>
          <Row class="min-h-[44px]">
            <Text class="w-[110px] text-[12px] text-muted-foreground">{tr("Port")}</Text>
            <Input value={port() ?? String(s()!.port)} onChange={(v) => port.set(v)} onSubmit={() => port() && set("port", port()!, true)} mono testID="setting-port" class="w-[90px]" />
            <If when={port() !== null && port() !== String(s()!.port)}>
              <PrimaryButton
                label={tr("Save")}
                testID="setting-port-save"
                onPress={() => {
                  set("port", port()!, true);
                  port.set(null);
                }}
              />
              <TextButton label={tr("Undo")} onPress={() => port.set(null)} />
            </If>
            <Text class="flex-1 text-[12px] text-muted-foreground">{tr("Agents connected before a change are connected again with the new port.")}</Text>
          </Row>
        </If>
      </List>
      <Text class="px-1 text-[12px] text-faint">
        {tr("Other computers send the gateway key as their API key once Share on the local network is on. Agents on this computer need none.")}
      </Text>
      <If when={(g()?.resting.length ?? 0) > 0}>
        <RowHead label={tr("Keys resting after a failure")} />
        <List>
          <Each of={g()?.resting ?? []} key={(r) => r.key}>
            {(r, i) => (
              <Row first={i() === 0}>
                <Mono text={r().key} class="flex-1" />
                <Badge label={tr("%ds", r().secs)} tone="warning" />
              </Row>
            )}
          </Each>
        </List>
      </If>
      <RowHead label={tr("Models")} note={String(choices().length)} class="pt-2">
        <Input value={find()} onChange={(v) => find.set(v)} testID="gateway-find" placeholder={tr("Find a model…")} class="h-[24px] w-[180px]" />
        <TextButton label={tr("Copy all ids")} onPress={() => copy(choices().map((c) => c.id).join("\n"))} />
      </RowHead>
      <If when={models().length > 0}>
        <List testID="gateway-models">
          <Each of={models()} key={(c) => c.id}>
            {(c, i) => <ModelRow choice={c()} first={i() === 0} />}
          </Each>
        </List>
      </If>
    </Page>
  );
}
