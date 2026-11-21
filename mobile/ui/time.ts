// Times as the phone shows them: a running turn's timer, how long ago a
// thread last moved (T3's `5m`, `3h`, `2d`).
import { signal } from "nsh";
import { tr } from "./i18n";

/** The time now, a tick a second while something shows a timer. */
export const now = signal(Date.now());
let users = 0;
let ticker: ReturnType<typeof setInterval> | undefined;

/** Starts the second's tick for a view; returns what stops it. */
export function tick(): () => void {
  users++;
  if (!ticker) ticker = setInterval(() => now.set(Date.now()), 1000);
  return () => {
    users--;
    if (users <= 0 && ticker) {
      clearInterval(ticker);
      ticker = undefined;
    }
  };
}

/** `3:12`, `1:02:05`. */
export function timer(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** T3's list time (`relativeTime`): `<1m`, `5m`, `3h`, `2d`. */
export function since(iso: string | undefined, at = Date.now()): string {
  const s = Math.max(0, Math.floor((at - Date.parse(iso ?? "")) / 1000));
  if (!(s >= 60)) return tr("<1m");
  if (s < 3600) return tr("%dm", Math.floor(s / 60));
  if (s < 86_400) return tr("%dh", Math.floor(s / 3600));
  return tr("%dd", Math.floor(s / 86_400));
}

/** `10:02`. */
export const clock = (iso: string | undefined) => {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : `${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

/** "just now", "3 min ago", "2 days ago". */
export function ago(iso: string | undefined): string {
  if (!iso) return "";
  const s = (Date.now() - Date.parse(iso)) / 1000;
  if (!(s >= 0)) return "";
  if (s < 60) return tr("Just now");
  if (s < 3600) return tr("%d min ago", Math.floor(s / 60));
  if (s < 86_400) return tr("%d h ago", Math.floor(s / 3600));
  return tr("%d days ago", Math.floor(s / 86_400));
}
