// pansy updating itself, as magpie does. A little after it starts, and
// every six hours, it asks the release feed in nsh.json's "updater"
// (nsh/updater) for a newer version; once one is out, the title bar and the
// quick panel's head show an Update pill. A click downloads it, saying how
// far along it is, waits for the agents' requests through the gateway to
// finish (a restart would cut them off; a click then restarts at once) and
// restarts pansy into it. Where pansy can't replace itself (a folder only
// the system writes to), the click opens the download instead. Settings ›
// About has what the last look found, and Check for updates. A build
// without a feed (a development run) never looks.
//
// A release may be of the window alone: a signed package of this code
// (docs/release.md), which pansy as installed downloads and restarts into
// without being replaced. It is offered the same way, named by its build
// ("0.1.0 (2)"), when pansy itself is the newest and the package was built
// for this very version: its code calls this version's pansy/native.
import { openUrl, signal, Text } from "nsh";
import { Button as BaseButton } from "nsh/base";
import {
  checkForPackage,
  checkForUpdate,
  downloadPackage,
  downloadUpdate,
  installAndRestart,
  restartToPackage,
  runningPackage,
  shellInfo,
  type AppUpdate,
  type PackageUpdate,
} from "nsh/updater";
import { tr } from "./i18n";
import { loadTrace, reason, routes, toast } from "./model";
import { Hint, Icon, MOTION, TextButton } from "./parts";

/** The first look after pansy starts, and the next ones (ms). */
const FIRST = 15_000;
const EVERY = 6 * 3_600_000;

type Phase = "idle" | "checking" | "downloading" | "waiting" | "installing";
/** A newer pansy (`app`), or a newer build of its window alone (`ui`), and what to call it. */
type Found = { version: string; app: AppUpdate | null; ui: PackageUpdate | null };
/** What the updater is doing. */
const phase = signal<Phase>("idle");
/** The newer version the last look found. */
const found = signal<Found | null>(null);
/** How much of it is downloaded (0..1), null while that is unknown. */
const share = signal<number | null>(null);
/** What went wrong last: the look, or the update. */
const failure = signal<{ update: boolean; text: string } | null>(null);
/** The last look found pansy up to date. */
const current = signal(false);
/** A click while waiting: restart without waiting any more. */
let now = false;

/** Whether this build looks for updates: it was packaged with a feed (`nsh-updater.json`). */
export function updatesOn(): boolean {
  try {
    return shellInfo().version != null;
  } catch {
    return false;
  }
}

/** Looks a little after pansy starts, and every six hours after. */
export function startUpdates() {
  if (!updatesOn()) return;
  setTimeout(() => void look(), FIRST);
  setInterval(() => void look(), EVERY);
}

/** A version and the build of its window: "0.1.0 (2)". */
function named(version: string, build: number): string {
  return `${version} (${build})`;
}

/** pansy's version as About says it: with the build of the window's code once that came as a package. */
export function versionSaid(version: string): string {
  try {
    const running = runningPackage();
    return running ? named(version, running.build) : version;
  } catch {
    return version;
  }
}

/** What the feed has for this pansy: a newer one, else a newer build of its window made for this version. */
async function newer(): Promise<Found | null> {
  const app = await checkForUpdate();
  if (app) return { version: app.version, app, ui: null };
  const shell = shellInfo();
  if (!shell.packages) return null;
  const ui = await checkForPackage();
  return ui && ui.version === shell.version ? { version: named(ui.version, ui.build), app: null, ui } : null;
}

/** Asks the feed for a newer version; a failure is kept for Settings, not shown. */
export async function look(): Promise<void> {
  if (phase() !== "idle") return;
  phase.set("checking");
  try {
    const update = await newer();
    found.set(update);
    current.set(!update);
    failure.set(null);
  } catch (error) {
    failure.set({ update: false, text: reason(error) });
  } finally {
    phase.set("idle");
  }
}

/** The agents' requests through the gateway not done yet. */
function inFlight(): number {
  try {
    loadTrace();
  } catch {
    return 0;
  }
  return routes().filter((r) => !r.done).length;
}

/** Downloads the update found, waits for the gateway to be idle, and restarts into it; opens its download where pansy can't replace itself. */
export async function runUpdate(): Promise<void> {
  const update = found();
  if (!update) return;
  if (phase() === "waiting") {
    now = true;
    return;
  }
  if (phase() !== "idle") return;
  const { app } = update;
  if (app && !app.canInstall) {
    void openUrl(app.url).catch((error) => toast(reason(error), "error"));
    return;
  }
  failure.set(null);
  share.set(null);
  phase.set("downloading");
  try {
    const progress = ({ received, total }: { received: number; total?: number | null }) => share.set(total ? received / total : null);
    await (app ? downloadUpdate(progress) : downloadPackage(progress));
    phase.set("waiting");
    now = false;
    while (!now && inFlight() > 0) await new Promise((done) => setTimeout(done, 1000));
    phase.set("installing");
    // settles only when it fails: pansy quits, and the new version starts
    await (app ? installAndRestart() : restartToPackage());
  } catch (error) {
    failure.set({ update: true, text: reason(error) });
    toast(tr("Couldn't update to %s: %s", update.version, reason(error)), "error");
  } finally {
    phase.set("idle");
  }
}

/** The pill's word: Update, how far the download is, or what it waits for. */
function said(): string {
  const p = share();
  if (phase() === "downloading") return p === null ? tr("Downloading…") : tr("Downloading… %d%", Math.min(100, Math.floor(p * 100)));
  if (phase() === "waiting") return tr("Waiting to update");
  if (phase() === "installing") return tr("Updating…");
  return failure()?.update ? tr("Update failed") : tr("Update");
}

/** What the pill says under the pointer. */
function hint(update: Found): string {
  const out = tr("%s is out", update.version);
  if (phase() === "downloading" || phase() === "installing") return tr("Restart to update to %s", update.version);
  if (phase() === "waiting") {
    const n = routes().filter((r) => !r.done).length;
    return tr("pansy restarts to update to %s once the agents' requests through the gateway have finished: %s. Click to restart now.", update.version, tr("%d in flight", n));
  }
  if (update.app && !update.app.canInstall) return `${out} · ${tr("pansy can't replace itself where it is installed; click to open the download.")}`;
  const f = failure();
  if (f?.update) return `${tr("Couldn't update to %s: %s", update.version, f.text)} · ${tr("Click to try again")}`;
  return `${out} · ${tr("Click to download and restart")}`;
}

/**
 * magpie's Update pill, before the title bar's icons and in the quick
 * panel's head: an accent arrow and word on a pale accent pill, filled under
 * the pointer, a little smaller while pressed, faded while it downloads or
 * restarts. Shown only once a newer version is out.
 */
export function UpdatePill(props: { testID: string }) {
  const busy = () => phase() === "downloading" || phase() === "installing";
  return (
    <If when={found()}>
      <Hint text={hint(found()!)}>
        <BaseButton
          onPress={() => void runUpdate()}
          disabled={busy()}
          accessibilityLabel={said()}
          testID={props.testID}
          class={`group h-[22px] flex-row items-center gap-1 rounded-full bg-accent pl-[7px] pr-[9px] hover:bg-primary active:scale-96 disabled:opacity-60 ${MOTION}`}
        >
          <Icon name="up" color="primary" hover={busy() ? undefined : "primary-foreground"} size={11} />
          <Text class={`text-[11.5px] font-semibold text-primary ${busy() ? "" : "group-hover:text-primary-foreground"} ${MOTION}`}>{said()}</Text>
        </BaseButton>
      </Hint>
    </If>
  );
}

/** Settings' line about updates: what the last look found, or why there is none. */
export function updateSaid(): string {
  if (!updatesOn()) return tr("This build doesn't look for updates: it has no release feed");
  const update = found();
  const f = failure();
  if (phase() === "checking") return tr("Checking for updates…");
  if (update) return f?.update ? tr("Couldn't update to %s: %s", update.version, f.text) : tr("%s is out", update.version);
  if (f) return tr("Couldn't check for updates: %s", f.text);
  if (current()) return tr("Up to date");
  return tr("pansy looks for a newer version a little after it starts and every six hours");
}

/** Settings' control: the pill once an update is out, else Check for updates. */
export function UpdateControl() {
  return (
    <If when={updatesOn()}>
      <If when={found()} else={<TextButton label={tr("Check for updates")} disabled={phase() !== "idle"} testID="update-check" onPress={() => void look()} />}>
        <UpdatePill testID="update-settings" />
      </If>
    </If>
  );
}
