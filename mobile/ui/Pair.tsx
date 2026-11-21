// 添加机器 (phone-ui.md "10. 添加机器"): the camera card (phase 1 has no
// camera: the system's camera opens the link, which is this page on the
// web), or the address and the code typed; an error in red; then the
// machine, its agents and "Connected", or the code to approve it by.
import { effect, platform, signal, Text, untracked, View } from "nsh";
import type { Machine } from "../net/client";
import { normalizeAddress, pageUrl, pair } from "../net/pair";
import type { RpcError } from "../net/types";
import { addMachine, connState } from "../state/store";
import { tr } from "./i18n";
import { pairLink, pairOpen } from "./nav";
import { agentLogo, BottomSheet, Btn, GlassButton, Group, GroupHead, Icon, Logo, Note, RowInput, Spinner } from "./parts";

/** What went wrong, as the user can act on it. */
function explain(e: unknown, address: string): string {
  const code = (e as RpcError)?.code;
  if (code === "network" || code === "timeout") return tr("Couldn't reach %s. Is the phone on the same Wi-Fi or tailnet, and is sharing on?", address);
  if (code === "forbidden" || code === "unauthorized" || code === "expired" || code === "not_found") return tr("The pairing code is wrong or has expired: make a new one on the computer");
  return tr("Couldn't pair: %s", String((e as Error)?.message ?? e));
}

export function PairSheet() {
  const address = signal("");
  const code = signal("");
  const busy = signal(false);
  const error = signal<string | null>(null);
  const done = signal<Machine | null>(null);

  const go = async (base: string, secret: string) => {
    busy.set(true);
    error.set(null);
    try {
      const m = await pair(base, secret);
      addMachine(m);
      done.set(m);
    } catch (e) {
      error.set(explain(e, base.replace(/^https?:\/\//, "")));
    } finally {
      busy.set(false);
    }
  };

  // opened from the computer's QR code: pair at once
  effect(() => {
    const link = pairLink();
    if (!link || !pairOpen()) return;
    pairLink.set(null);
    address.set(link.base.replace(/^https?:\/\//, ""));
    code.set(link.secret);
    void go(link.base, link.secret);
  });

  // the web app is served by the machine itself: its address is the page's
  let prefilled = false;
  effect(() => {
    if (!pairOpen()) {
      prefilled = false;
      return;
    }
    if (prefilled || platform.os !== "web") return;
    prefilled = true;
    if (untracked(address)) return;
    const page = pageUrl();
    if (page && /^https?:\/\//.test(page)) address.set(page.replace(/^https?:\/\//, "").replace(/[/?#].*$/, ""));
  });

  effect(() => {
    if (!pairOpen()) {
      done.set(null);
      error.set(null);
      code.set("");
    }
  });

  const submit = () => {
    if (!address().trim() || !code().trim() || busy()) return;
    void go(normalizeAddress(address()), code().trim().replace(/^.*#pair=/, ""));
  };
  const close = () => pairOpen.set(false);
  const state = () => (done() ? connState(done()!.id) : null);

  return (
    <BottomSheet
      open={pairOpen()}
      onClose={close}
      title={tr("Add machine")}
      start={0.55}
      testID="pair-sheet"
      left={<GlassButton icon="x" label={tr("Close")} onPress={close} testID="pair-close" />}
      right={
        <If when={!done()}>
          <View role="button" testID="pair-connect" accessibilityLabel={tr("Connect")} onPress={submit} class="h-[38px] w-[38px] items-center justify-center rounded-full bg-primary active:scale-90">
            <If when={busy()} else={<Icon name="check" color="primary-foreground" />}>
              <Spinner color="primary-foreground" />
            </If>
          </View>
        </If>
      }
    >
      <View class="gap-[14px] px-5 pb-10 pt-[6px]">
        <If
          when={done()}
          else={
            <>
              <View class="h-[220px] items-center justify-center gap-3 rounded-[26px] bg-grouped" style={{ backgroundImage: "radial-gradient(circle at 50% 40%, rgba(127,127,127,0.06), rgba(127,127,127,0.18))" }}>
                <View class="h-[130px] w-[130px] items-center justify-center gap-[6px] rounded-[28px] border-[3px] border-fg bg-glass opacity-85">
                  <Icon name="qr-code" size={40} />
                  <Text class="text-[13px] font-medium text-fg">{tr("Scan with the camera")}</Text>
                </View>
                <Text class="px-6 text-center text-[13px] text-fg-2">
                  {platform.os === "web"
                    ? tr("Open pansy on the computer, Settings › Sharing, and scan its QR code with the phone's camera: this page opens paired")
                    : tr("Open pansy on the computer, Settings › Sharing, and scan its QR code with the system camera: it opens pansy")}
                </Text>
              </View>
              <GroupHead text={tr("Or type it in")} />
              <Group danger={!!error()}>
                <RowInput first label={tr("Address")} value={address()} onChange={address.set} placeholder="192.168.1.20:7268" mono testID="pair-address" />
                <RowInput label={tr("Code")} value={code()} onChange={code.set} placeholder="••••-••••" mono testID="pair-code" />
              </Group>
              <If when={error()}>
                <Text testID="pair-error" class="-mt-[6px] text-[14px] text-danger">
                  {error() ?? ""}
                </Text>
              </If>
              <Text class="text-[14px] text-fg-2">{tr("With the computer and the phone on the same Wi-Fi, or both on Tailscale, they connect directly.")}</Text>
              <Btn label={busy() ? tr("Connecting…") : tr("Connect")} onPress={submit} disabled={busy() || !address().trim() || !code().trim()} testID="pair-submit" />
            </>
          }
        >
          <View testID="pair-done" class="items-center gap-3 pt-6">
            <Icon name={done()?.approval === "pending" ? "shield-alert" : "circle-check"} color={done()?.approval === "pending" ? "warning" : "emerald"} size={40} />
            <Text class="text-[21px] font-medium text-fg">{done()?.name ?? ""}</Text>
            <View class="flex-row gap-2">
              <Each of={(done()?.capabilities ?? []).filter((c) => agentLogo(c === "claude" ? "claude-code" : c))} key={(c) => c}>
                {(c) => <Logo name={agentLogo(c() === "claude" ? "claude-code" : c())} size={22} />}
              </Each>
            </View>
            <If
              when={done()?.approval === "pending"}
              else={<Text class="text-[16px] text-fg-2">{state() === "online" ? tr("Connected") : tr("Paired · connecting…")}</Text>}
            >
              <Note tone="warning" text={tr("Approve this phone on the computer. Check that it shows this code:")} />
              <Text testID="pair-code-shown" class="font-mono text-[22px] font-bold tracking-[3px] text-fg">
                {done()?.code ?? ""}
              </Text>
            </If>
            <Btn label={tr("Done")} onPress={close} class="mt-4 self-stretch" testID="pair-done-button" />
          </View>
        </If>
      </View>
    </BottomSheet>
  );
}
