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
// for this very version: its code calls this version's pansy/native. A
// package is small, so the look that finds one downloads it too: the pill
// then says Restart, and a pansy never restarted from it still starts as
// the new build the next time it is opened.
//
// The pill in the title bar has magpie's × on its corner, shown under the
// pointer: it puts the pill away for three days, Settings still offering
// the update. What a release says changed (the feed's notes) is kept when
// its update is put in, and shown once pansy has restarted into it
// (ui/WhatsNew.tsx); Settings › About › What's new opens it again.
import { openUrl, signal, Storage, Text, View } from "nsh";
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
import { Settings } from "pansy/native";
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
/** The window's package found is downloaded: a restart runs it. */
const ready = signal(false);

/** How long the pill's × puts it away (ms). */
const AWAY = 3 * 24 * 3_600_000;
/** Kept across runs (NSH's storage): when the pill comes back, and the notes of the update last put in. */
const AWAY_KEY = "update.away";
const NOTES_KEY = "update.notes";
/** The pill is away until then (ms since 1970); 0: it shows. */
const awayUntil = signal(0);

/** A release's notes: the version they are of, what it says changed, and whether What's new has shown them. */
export type Notes = { version: string; text: string; seen?: boolean };
/** The notes of the version running, kept from when its update was put in. */
export const installed = signal<Notes | null>(null);
/** The notes What's new shows now, and whether they are of an update not put in yet. */
export const reading = signal<{ notes: Notes; pending: boolean } | null>(null);

/** Whether this build looks for updates: it was packaged with a feed (`nsh-updater.json`). */
export function updatesOn(): boolean {
  try {
    return shellInfo().version != null;
  } catch {
    return false;
  }
}

/** The version running, as an update to it was named. */
function running(): string {
  const version = Settings.version();
  try {
    return versionSaid(shellInfo().version ?? version);
  } catch {
    return version;
  }
}

/** Reads what was kept: the pill put away, and the notes of the update pansy restarted into, shown once. */
async function restore(): Promise<void> {
  try {
    awayUntil.set(Number(await Storage.getItem(AWAY_KEY)) || 0);
    const kept = JSON.parse((await Storage.getItem(NOTES_KEY)) ?? "null") as Notes | null;
    // notes of another version: an update that never went in, or one long past
    if (!kept || kept.version !== running()) return;
    installed.set(kept);
    if (kept.seen) return;
    reading.set({ notes: kept, pending: false });
    void Storage.setItem(NOTES_KEY, JSON.stringify({ ...kept, seen: true }));
  } catch {
    // nothing kept that reads: the pill shows, and there are no notes
  }
}

/** The pill's ×: puts it away for three days; Settings › About still has the update. */
export function hideUpdate() {
  const until = Date.now() + AWAY;
  awayUntil.set(until);
  void Storage.setItem(AWAY_KEY, String(until)).catch(() => {});
  toast(tr("Update hidden for 3 days; Settings still has it"));
}

/** What the update found says changed, if it says. */
function notesOf(update: Found): Notes | null {
  const text = (update.app ?? update.ui)?.notes?.trim();
  return text ? { version: update.version, text } : null;
}

/** Whether Settings has notes to open: the waiting update's, else the running version's. */
export function hasNotes(): boolean {
  const update = found();
  return !!((update && notesOf(update)) || installed());
}

/** Settings' What's new: the waiting update's notes, else the running version's. */
export function openNotes() {
  const update = found();
  const pending = update && notesOf(update);
  if (pending) reading.set({ notes: pending, pending: true });
  else if (installed()) reading.set({ notes: installed()!, pending: false });
}

/** Looks a little after pansy starts, and every six hours after. */
export function startUpdates() {
  void restore();
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

/** What the update says changed, kept for What's new once pansy is back as it. */
async function keepNotes(update: Found): Promise<void> {
  const notes = notesOf(update);
  await (notes ? Storage.setItem(NOTES_KEY, JSON.stringify(notes)) : Storage.removeItem(NOTES_KEY)).catch(() => {});
}

/** Asks the feed for a newer version, and downloads a package of the window it finds; a failure is kept for Settings, not shown. */
export async function look(): Promise<void> {
  if (phase() !== "idle") return;
  phase.set("checking");
  try {
    // with the window's package in already, only a newer pansy changes what is offered
    const update = ready() ? await checkForUpdate().then((app) => (app ? { version: app.version, app, ui: null } : found())) : await newer();
    if (update?.app) ready.set(false);
    if (update?.ui && !ready()) {
      try {
        await downloadPackage();
        await keepNotes(update);
        ready.set(true);
      } catch {
        // not now: the pill's click downloads it
      }
    }
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
  phase.set(ready() ? "waiting" : "downloading");
  try {
    const progress = ({ received, total }: { received: number; total?: number | null }) => share.set(total ? received / total : null);
    if (!ready()) await (app ? downloadUpdate(progress) : downloadPackage(progress));
    phase.set("waiting");
    now = false;
    while (!now && inFlight() > 0) await new Promise((done) => setTimeout(done, 1000));
    phase.set("installing");
    await keepNotes(update);
    // settles only when it fails: pansy quits, and the new version starts
    await (app ? installAndRestart() : restartToPackage());
  } catch (error) {
    failure.set({ update: true, text: reason(error) });
    toast(tr("Couldn't update to %s: %s", update.version, reason(error)), "error");
  } finally {
    phase.set("idle");
  }
}

/** The pill's word: Update (Restart, with the window's package in), how far the download is, or what it waits for. */
function said(): string {
  const p = share();
  if (phase() === "downloading") return p === null ? tr("Downloading…") : tr("Downloading… %d%", Math.min(100, Math.floor(p * 100)));
  if (phase() === "waiting") return tr("Waiting to update");
  if (phase() === "installing") return tr("Updating…");
  if (failure()?.update) return tr("Update failed");
  return ready() ? tr("Restart") : tr("Update");
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
  return ready() ? tr("Restart to update to %s", update.version) : `${out} · ${tr("Click to download and restart")}`;
}

/**
 * magpie's Update pill, before the title bar's icons and in the quick
 * panel's head: an accent arrow and word on a pale accent pill, filled under
 * the pointer, a little smaller while pressed, faded while it downloads or
 * restarts. Shown only once a newer version is out. With `hide` (the title
 * bar's), its × sits off its top right corner while the pointer is over
 * it, and the pill stays away while it was put away; Settings' always shows.
 */
export function UpdatePill(props: { testID: string; hide?: boolean; always?: boolean }) {
  const busy = () => phase() === "downloading" || phase() === "installing" || phase() === "waiting";
  const over = signal(false);
  return (
    <If when={found() && (props.always || awayUntil() <= Date.now())}>
      {/* room for the × inside what the pointer is over, taken back from the bar */}
      <View class="relative -my-[9px] -mr-[3px] py-[9px] pr-[3px]" onMouseEnter={() => over.set(true)} onMouseLeave={() => over.set(false)}>
        <If when={props.hide && over() && !busy()}>
          <BaseButton
            onPress={hideUpdate}
            accessibilityLabel={tr("Hide for 3 days")}
            testID={`${props.testID}-hide`}
            class={`group absolute right-0 top-0 h-[14px] w-[14px] items-center justify-center rounded-full border border-border bg-card ${MOTION}`}
          >
            <Icon name="close" color="muted-foreground" hover="foreground" size={8} />
          </BaseButton>
        </If>
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
      </View>
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
        <UpdatePill testID="update-settings" always />
      </If>
    </If>
  );
}
