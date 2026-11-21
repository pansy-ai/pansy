// 设置 (phone-ui.md "12. 设置"): the machines, each with its routes in the
// order they are tried, what this phone may do there, device approval and
// the other devices (devices.*), and removing it; the appearance, the
// language; about.
import { appInfo, effect, resource, Scroll, signal, Text, View, type JSX } from "nsh";
import { AlertDialog, NavPage } from "nsh/base";
import type { Machine } from "../net/client";
import { normalizeAddress } from "../net/pair";
import type { Device } from "../net/types";
import { appearance, setAppearance, type Appearance } from "../state/prefs";
import { conn, connState, machine, machines, online, removeMachine, setRoutes } from "../state/store";
import { language, setLanguage, tr, type Choice } from "./i18n";
import { pairOpen, settingsNav } from "./nav";
import { Btn, Field, GlassButton, Group, GroupHead, GroupRow, Icon, Note, Seg, toast, Toggle } from "./parts";
import { ago } from "./time";

/** A page of the Settings tab: its header on the web and the desktops, the system's elsewhere. */
function Page(props: { title: string; subtitle?: string; back?: boolean; children?: JSX.Element; testID?: string }) {
  return (
    <NavPage
      title={props.title}
      header={
        <View class="h-[52px] flex-row items-center gap-2 bg-screen px-4">
          <View class="w-[38px]">
            <If when={props.back}>
              <GlassButton icon="chevron-left" label={tr("Back")} onPress={() => settingsNav.pop()} testID="settings-back" />
            </If>
          </View>
          <View class="min-w-0 flex-1 items-center">
            <Text class="text-[18px] font-bold text-fg line-clamp-1">{props.title}</Text>
            <If when={props.subtitle}>
              <Text class="text-[12px] text-fg-2 line-clamp-1">{props.subtitle ?? ""}</Text>
            </If>
          </View>
          <View class="w-[38px]" />
        </View>
      }
    >
      <Scroll class="flex-1 bg-screen">
        <View testID={props.testID} class="gap-[14px] px-5 pb-40 pt-2">
          {props.children}
        </View>
      </Scroll>
    </NavPage>
  );
}

const stateWord = (id: string) =>
  ({ online: tr("Online"), connecting: tr("Connecting…"), offline: tr("Offline"), pending: tr("Waiting for approval"), revoked: tr("Signed out: pair again") })[connState(id)];

function Dot(props: { id: string }) {
  const color = () => ({ online: "bg-emerald", connecting: "bg-sky", offline: "bg-fg-3", pending: "bg-warning", revoked: "bg-danger" })[connState(props.id)];
  return <View class={`h-2 w-2 rounded-full ${color()}`} />;
}

export function SettingsRoot() {
  return (
    <Page title={tr("Settings")} testID="settings">
      <GroupHead text={tr("Machines")} />
      <Group>
        <Each of={machines()} key={(m) => m.id}>
          {(m, i) => (
            <GroupRow
              first={i() === 0}
              icon={m().os === "linux" ? "server" : "laptop"}
              title={m().name}
              sub={[stateWord(m().id), m().os, m().version ? `pansy ${m().version}` : ""].filter(Boolean).join(" · ")}
              trailing={<Dot id={m().id} />}
              chevron
              testID={`machine-${m().id}`}
              onPress={() => settingsNav.push({ name: "machine", id: m().id })}
            />
          )}
        </Each>
        <GroupRow first={machines().length === 0} icon="plus" tone="primary" title={tr("Add machine")} onPress={() => pairOpen.set(true)} testID="settings-add-machine" />
      </Group>
      <GroupHead text={tr("Appearance")} />
      <Seg<Appearance>
        testID="appearance"
        value={appearance()}
        onChange={setAppearance}
        options={[
          { value: "system", label: tr("System") },
          { value: "light", label: tr("Light") },
          { value: "dark", label: tr("Dark") },
        ]}
      />
      <GroupHead text={tr("Language")} />
      <Seg<Choice>
        testID="language"
        value={language()}
        onChange={setLanguage}
        options={[
          { value: "system", label: tr("System") },
          { value: "en", label: "English" },
          { value: "zh-Hans", label: "简体中文" },
        ]}
      />
      <GroupHead text={tr("Notifications")} />
      <Group>
        <GroupRow first icon="info" title={tr("Push notifications")} sub={tr("Phase 2: add the web app to the home screen to get them, or give an ntfy or Bark address")} tone="muted" />
      </Group>
      <Group>
        <GroupRow first icon="info" title={tr("About")} chevron onPress={() => settingsNav.push({ name: "about" })} testID="settings-about" />
      </Group>
    </Page>
  );
}

const SCOPE_NAMES: Record<string, () => string> = {
  "access:read": () => tr("see devices"),
  "access:write": () => tr("pair and revoke devices"),
  "orchestration:read": () => tr("read threads"),
  "orchestration:operate": () => tr("run agents"),
  "source-control:write": () => tr("commit and push"),
  "filesystem:read": () => tr("read files"),
  "terminal:read": () => tr("see terminals"),
  "terminal:operate": () => tr("use terminals"),
  "settings:write": () => tr("change settings"),
  "providers:manage": () => tr("manage providers"),
};

const routeKind = (k: string) => ({ lan: tr("Local network"), tailnet: tr("Tailscale"), tunnel: tr("Tunnel"), paired: tr("Paired at") })[k] ?? k;
const routeIcon = (k: string) => (k === "tailnet" ? "network" : k === "tunnel" ? "globe" : "wifi") as "network" | "globe" | "wifi";

export function MachinePage(props: { id: string }) {
  const m = (): Machine | undefined => machine(props.id);
  const c = () => conn(props.id);
  const newRoute = signal("");
  const adding = signal(false);
  const canWrite = () => m()?.scopes.includes("access:write") ?? false;
  const canRead = () => (m()?.scopes.includes("access:read") ?? false) || canWrite();
  const devices = resource(
    () => (online(props.id) && canRead() ? props.id : null),
    async () => c()!.call<{ devices: Device[]; requireApproval: boolean }>("devices.list"),
  );
  const routes = () => m()?.routes ?? [];
  const move = (i: number, by: number) => {
    const rs = [...routes()];
    const j = i + by;
    if (j < 0 || j >= rs.length) return;
    [rs[i], rs[j]] = [rs[j], rs[i]];
    setRoutes(props.id, rs);
  };
  const addRoute = () => {
    const url = normalizeAddress(newRoute());
    if (!newRoute().trim() || routes().some((r) => r.url === url)) return;
    setRoutes(props.id, [...routes(), { kind: "manual", url }]);
    newRoute.set("");
    adding.set(false);
  };
  const setApproval = async (on: boolean) => {
    try {
      await c()!.call("devices.setRequireApproval", { requireApproval: on });
      devices.mutate({ devices: devices()?.devices ?? [], requireApproval: on });
    } catch (e) {
      toast(tr("Couldn't change it: %s", String((e as Error).message ?? e)), { tone: "error" });
    }
  };
  const device = async (d: Device, method: "devices.revoke" | "devices.approve") => {
    try {
      await c()!.call(method, { deviceId: d.id });
      devices.refetch();
    } catch (e) {
      toast(tr("Couldn't change it: %s", String((e as Error).message ?? e)), { tone: "error" });
    }
  };
  const remove = async () => {
    const r = await AlertDialog.show({
      title: tr("Remove %s?", m()?.name ?? ""),
      description: tr("This phone forgets it. Pair again to come back; the computer keeps the device until you revoke it there."),
      actions: [
        { label: tr("Cancel"), role: "cancel", id: "cancel" },
        { label: tr("Remove"), role: "destructive", id: "remove" },
      ],
    });
    if (r.action !== "remove") return;
    // the computer forgets this phone too, when it can still be told
    if (online(props.id) && m()) await c()!.call("devices.revoke", { deviceId: m()!.deviceId }).catch(() => {});
    settingsNav.pop();
    removeMachine(props.id);
  };
  effect(() => {
    if (!m()) settingsNav.popToRoot();
  });
  return (
    <Page title={m()?.name ?? ""} subtitle={[m()?.os, m()?.version ? `pansy ${m()!.version}` : "", stateWord(props.id)].filter(Boolean).join(" · ")} back testID="machine-page">
      <If when={connState(props.id) === "revoked"}>
        <Note tone="danger" text={tr("This machine no longer knows this phone: pair it again")} action={{ label: tr("Pair"), onPress: () => pairOpen.set(true) }} />
      </If>
      <GroupHead text={tr("Routes")} />
      <Group>
        <Each of={routes()} key={(r) => r.url}>
          {(r, i) => (
            <GroupRow
              first={i() === 0}
              icon={routeIcon(r().kind)}
              title={routeKind(r().kind)}
              sub={r().url.replace(/^https?:\/\//, "")}
              testID={`route-${i()}`}
              trailing={
                <View class="flex-row items-center gap-3">
                  <If when={c()?.route() === r().url} else={<Text class="text-[14px] text-fg-2">{r().ok === false ? tr("No answer") : r().ms !== undefined ? `${r().ms} ms` : ""}</Text>}>
                    <View class="flex-row items-center gap-1">
                      <Icon name="circle-check" color="emerald" size={14} />
                      <Text class="text-[14px] text-emerald">{tr("In use")}</Text>
                    </View>
                  </If>
                  <View class="gap-1">
                    <View role="button" accessibilityLabel={tr("Move up")} onPress={() => move(i(), -1)} hitSlop={4} testID={`route-up-${i()}`}>
                      <Icon name="chevron-up" color={i() === 0 ? "line" : "fg-3"} size={14} />
                    </View>
                    <View role="button" accessibilityLabel={tr("Move down")} onPress={() => move(i(), 1)} hitSlop={4}>
                      <Icon name="chevron-down" color={i() === routes().length - 1 ? "line" : "fg-3"} size={14} />
                    </View>
                  </View>
                </View>
              }
              onLongPress={() => routes().length > 1 && setRoutes(props.id, routes().filter((x) => x.url !== r().url))}
            />
          )}
        </Each>
        <If when={adding()} else={<GroupRow icon="plus" tone="primary" title={tr("Add route")} onPress={() => adding.set(true)} testID="route-add" />}>
          <View class="flex-row items-center gap-2 px-4 py-3">
            <View class="flex-1">
              <Field value={newRoute()} onChange={newRoute.set} placeholder="mac.tail1234.ts.net:7268" mono testID="route-field" class="bg-card" onSubmit={addRoute} />
            </View>
            <GlassButton icon="check" label={tr("Add route")} onPress={addRoute} testID="route-save" />
          </View>
        </If>
      </Group>
      <Text class="px-4 text-[13px] text-fg-3">{tr("Tried in this order; the first that answers is used. Hold one to remove it.")}</Text>
      <GroupHead text={tr("This phone")} />
      <Group>
        <GroupRow first title={tr("What it may do")} sub={(m()?.scopes ?? []).map((s) => SCOPE_NAMES[s]?.() ?? s).join(", ")} testID="machine-scopes" />
        <If when={canWrite()}>
          <GroupRow
            title={tr("Device approval")}
            sub={tr("New devices wait for approval on the computer or a connected phone")}
            trailing={<Toggle on={devices()?.requireApproval ?? false} onChange={(v) => void setApproval(v)} label={tr("Device approval")} testID="device-approval" disabled={!devices()} />}
          />
        </If>
      </Group>
      <If when={canWrite() && devices() && !devices()!.requireApproval}>
        <Note tone="warning" icon="shield" text={tr("Device approval is off: whoever pairs with a code can control this computer.")} />
      </If>
      <If when={devices()}>
        <GroupHead text={tr("Devices on this computer")} />
        <Group testID="devices">
          <Each of={devices()?.devices ?? []} key={(d) => d.id}>
            {(d, i) => (
              <GroupRow
                first={i() === 0}
                icon={d().platform === "web" ? "globe" : "smartphone"}
                title={d().current ? tr("This phone (%s)", d().name) : d().name}
                sub={d().approval === "pending" ? tr("Waiting for approval") : ago(d().lastSeen ?? d().createdAt)}
                testID={`device-${d().id}`}
                trailing={
                  <If when={!d().current && canWrite()}>
                    <View class="flex-row gap-3">
                      <If when={d().approval === "pending"}>
                        <Text role="button" class="text-[14px] font-medium text-primary" onPress={() => void device(d(), "devices.approve")} testID={`approve-${d().id}`}>
                          {tr("Approve")}
                        </Text>
                      </If>
                      <Text role="button" class="text-[14px] font-medium text-danger" onPress={() => void device(d(), "devices.revoke")} testID={`revoke-${d().id}`}>
                        {tr("Revoke")}
                      </Text>
                    </View>
                  </If>
                }
              />
            )}
          </Each>
        </Group>
      </If>
      <Btn tone="danger" label={tr("Remove this machine")} onPress={() => void remove()} testID="machine-remove" />
    </Page>
  );
}

export function AboutPage() {
  const info = appInfo();
  return (
    <Page title={tr("About")} back testID="about">
      <Group>
        <GroupRow first title="pansy" sub={tr("Version %s (%s)", info.version ?? "?", String(info.build ?? "?"))} />
        <GroupRow title={tr("License")} sub="MIT · the pansy contributors" />
      </Group>
      <GroupHead text={tr("Open source in this app")} />
      <Group>
        <GroupRow first title="DM Sans" sub="SIL Open Font License 1.1 · The DM Sans Project Authors" />
        <GroupRow title="Lucide" sub="ISC · Lucide Contributors" />
        <GroupRow title="LobeHub Icons" sub={tr("MIT · the marks are their owners'")} />
        <GroupRow title="Native Shell" sub="nsh 0.6.1" />
      </Group>
    </Page>
  );
}
