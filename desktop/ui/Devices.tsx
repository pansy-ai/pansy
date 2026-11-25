// Phones and other devices controlling this computer's agents
// (docs/plans/phone.md): the remote interface on or off, adding a device by
// a link or its code, the devices paired, and pansy kept running without the
// window. Its rows are under Settings › Network and sharing; adding a device
// is a modal over the window.
import { Clipboard, Image, signal, Text, View, type JSX } from "nsh";
import { Remote } from "pansy/native";
import { tr } from "./i18n";
import { act, ask, reason, toast } from "./model";
import { Badge, Input, Modal, Mono, OnOff, PrimaryButton, TextButton } from "./parts";
import { ago } from "./Sessions";

interface Device {
  id: string;
  name: string;
  platform: string;
  scopes: string[];
  approval: "approved" | "pending";
  createdAt: string;
  lastSeen: string;
  code: string | null;
}

interface RemoteView {
  machine: string;
  name: string;
  hosts: string[];
  requireApproval: boolean;
  devices: Device[];
  service: { installed: boolean; answering: boolean };
  answering: boolean;
  settings: { enabled: boolean; lan: boolean; port: number; webhook: string; writerModel: string; name: string };
}

interface Pairing {
  secret: string;
  urls: string[];
  expiresAt: string;
  qr: string;
}

export const remote = signal<RemoteView | null>(null);
/** The pairing the modal shows, while it is open. */
const pairing = signal<Pairing | null>(null);
const pairingReadOnly = signal(false);

export async function loadRemote() {
  try {
    remote.set(JSON.parse(await Remote.status()) as RemoteView);
  } catch (error) {
    toast(reason(error), "error");
  }
}

function set(key: string, value: string) {
  return act(`remote-${key}`, async () => {
    await Remote.set(key, value);
    await loadRemote();
  });
}

async function turn(on: boolean) {
  if (
    on &&
    !(await ask({
      title: tr("Let other devices control your agents?"),
      description: tr(
        "A device you pair can start agents in your folders, answer their requests, run commands and push to your repositories, as you can at this computer. Pair only your own devices; you can revoke one at any time",
      ),
      confirm: tr("Turn on"),
      danger: true,
    }))
  )
    return;
  await set("enabled", String(on));
}

let watching: ReturnType<typeof setInterval> | null = null;

async function openPairing(readOnly = pairingReadOnly()) {
  pairingReadOnly.set(readOnly);
  try {
    pairing.set(JSON.parse(await Remote.pair(readOnly)) as Pairing);
  } catch (error) {
    toast(reason(error), "error");
    return;
  }
  // A device that pairs while the code shows closes it.
  const known = new Set((remote()?.devices ?? []).map((d) => d.id));
  if (watching) clearInterval(watching);
  watching = setInterval(() => {
    void loadRemote().then(() => {
      const added = (remote()?.devices ?? []).find((d) => !known.has(d.id));
      if (!added) return;
      closePairing();
      toast(tr("%s is paired", added.name));
    });
  }, 2000);
}

function closePairing() {
  if (watching) clearInterval(watching);
  watching = null;
  pairing.set(null);
  void loadRemote();
}

const on = () => !!remote()?.settings.enabled;

/** "Linux · paired Nov 8 · seen 5 min ago". */
function deviceLine(d: Device): string {
  const platform = d.platform === "ios" ? "iOS" : d.platform === "android" ? "Android" : tr("Browser");
  const access = d.scopes.includes("orchestration:operate") ? "" : ` · ${tr("Read only")}`;
  return `${platform}${access} · ${tr("seen %s", ago(d.lastSeen))}`;
}

function DeviceRow(props: { device: Device; first: boolean }) {
  const d = () => props.device;
  return (
    <View testID={`device-${d().id}`} class={`flex-row items-center gap-3 py-[7px] ${props.first ? "" : "border-t border-line-2"}`}>
      <View class="min-w-0 flex-1 gap-[2px]">
        <View class="flex-row items-center gap-2">
          <Text class="text-[13px] font-medium text-foreground line-clamp-1">{d().name}</Text>
          <If when={d().code}>
            <Mono text={d().code ?? ""} />
          </If>
          <If when={d().approval === "pending"}>
            <Badge tone="warning" label={tr("Waiting for approval")} />
          </If>
        </View>
        <Text class="text-[12px] text-muted-foreground line-clamp-1">{deviceLine(d())}</Text>
      </View>
      <If when={d().approval === "pending"}>
        <TextButton
          label={tr("Approve")}
          testID={`device-approve-${d().id}`}
          onPress={() => void act(`approve-${d().id}`, () => Remote.approve(d().id).then(loadRemote), tr("Approved"))}
        />
      </If>
      <TextButton
        label={tr("Revoke")}
        testID={`device-revoke-${d().id}`}
        onPress={async () => {
          const yes = await ask({
            title: tr("Revoke %s?", d().name),
            description: tr("It is signed out at once and must be paired again to control this computer."),
            confirm: tr("Revoke"),
            danger: true,
          });
          if (yes) void act(`revoke-${d().id}`, () => Remote.revoke(d().id).then(loadRemote), tr("Revoked"));
        }}
      />
    </View>
  );
}

/** The devices paired: each with its short code, Approve while it waits, and Revoke. */
export function DeviceList() {
  return (
    <If
      when={(remote()?.devices.length ?? 0) > 0}
      else={<Text class="text-[12px] text-faint">{tr("No device yet: Add a device, then open the link on it or scan the code.")}</Text>}
    >
      <View testID="device-list">
        <Each of={remote()?.devices ?? []} key={(d) => d.id}>
          {(d, i) => <DeviceRow device={d()} first={i() === 0} />}
        </Each>
      </View>
    </If>
  );
}

export function RemoteSwitch() {
  return <OnOff testID="setting-remote" on={on()} onChange={(v) => void turn(v)} />;
}

export function LanSwitch() {
  return <OnOff testID="setting-remote-lan" on={remote()?.settings.lan !== false} onChange={(v) => void set("lan", String(v))} />;
}

export function ApprovalSwitch() {
  return (
    <OnOff
      testID="setting-remote-approval"
      on={!!remote()?.requireApproval}
      onChange={(v) => void act("remote-approval", () => Remote.requireApproval(v).then(loadRemote))}
    />
  );
}

export function AddDevice() {
  return <PrimaryButton label={tr("Add a device")} icon="add" disabled={!on()} onPress={() => void openPairing(false)} testID="remote-add" />;
}

/** Whether the phone can reach this computer now: where it listens, or why not. */
export function remoteSaid(): string {
  const r = remote();
  if (!r) return "";
  if (!r.settings.enabled) return tr("Off: no other device can reach your agents.");
  if (!r.answering) return tr("On, but nothing answers on port %d: restart pansy, or see whether another program has the port", r.settings.port);
  const where = r.settings.lan && r.hosts.length ? r.hosts.join(" · ") : `127.0.0.1:${r.settings.port}`;
  return tr("Listening on %s, to this computer, its private networks and tailnet only", where);
}

/** A setting kept as text: a field and Save. */
export function RemoteText(props: { field: "webhook" | "writerModel" | "name"; placeholder: string; mono?: boolean }) {
  const draft = signal<string | null>(null);
  const saved = () => remote()?.settings[props.field] ?? "";
  const save = () => {
    const v = draft();
    if (v === null) return;
    void set(props.field, v).then((ok) => ok && draft.set(null));
  };
  return (
    <View class="flex-row items-center gap-2">
      <Input value={draft() ?? saved()} onChange={(v) => draft.set(v)} onSubmit={save} mono={props.mono} placeholder={props.placeholder} testID={`setting-remote-${props.field}`} class="flex-1" />
      <If when={draft() !== null && draft() !== saved()}>
        <PrimaryButton label={tr("Save")} onPress={save} testID={`setting-remote-${props.field}-save`} />
      </If>
    </View>
  );
}

export function serviceSaid(): string {
  const s = remote()?.service;
  if (!s) return "";
  if (!s.installed) return tr("Off: agents driven from your phone stop when this window closes.");
  return s.answering ? tr("Installed and running: your phone reaches your agents while you are signed in, the window open or not.") : tr("Installed, but not answering yet.");
}

export function ServiceControl() {
  const installed = () => !!remote()?.service.installed;
  return (
    <TextButton
      label={installed() ? tr("Remove") : tr("Install")}
      testID="remote-service"
      onPress={() =>
        void act(
          "remote-service",
          async () => {
            const said = installed() ? await Remote.uninstallService() : await Remote.installService();
            await loadRemote();
            if (said.trim()) toast(said.split("\n").pop() ?? "");
          },
        )
      }
    />
  );
}

function Link(props: { url: string; first: boolean }): JSX.Element {
  return (
    <View class={`flex-row items-center gap-2 py-[5px] ${props.first ? "" : "border-t border-line-2"}`}>
      <Text class="flex-1 font-mono text-[12px] text-fg-2 line-clamp-1">{props.url}</Text>
      <TextButton label={tr("Copy")} onPress={() => void Clipboard.writeText(props.url).then(() => toast(tr("Copied")))} />
    </View>
  );
}

/** Add a device: the code to scan, the links behind it, how long they last. */
export function DeviceModals() {
  return (
    <If when={pairing()}>
      <Modal
        testID="pair"
        width={460}
        title={tr("Add a device")}
        onClose={closePairing}
        foot={
          <>
            <OnOff on={pairingReadOnly()} onChange={(v) => void openPairing(v)} testID="pair-read-only" />
            <Text class="text-[12px] text-fg-2">{tr("Read only")}</Text>
            <View class="flex-1" />
            <TextButton label={tr("New code")} onPress={() => void openPairing()} testID="pair-again" />
            <PrimaryButton label={tr("Done")} onPress={closePairing} testID="pair-done" />
          </>
        }
      >
        <View class="items-center gap-[10px]">
          <View class="rounded-[10px] bg-white p-[8px]">
            <Image src={`data:image/svg+xml,${encodeURIComponent(pairing()?.qr ?? "")}`} style={{ width: 200, height: 200 }} />
          </View>
          <Text class="text-center text-[12px] text-muted-foreground">
            {tr("Scan it with your phone's camera, or open one of these links on the device. It works once, within 10 minutes.")}
          </Text>
        </View>
        <View>
          <Each of={pairing()?.urls ?? []} key={(u) => u}>
            {(u, i) => <Link url={u()} first={i() === 0} />}
          </Each>
        </View>
        <Text class="text-[12px] text-faint">
          {tr("Away from home? Put this computer and the phone on the same tailnet (Tailscale): its 100.x address is listed too.")}
        </Text>
      </Modal>
    </If>
  );
}
