// Times as the phone shows them: a running turn's timer, when a thread last
// moved (today's hour, yesterday, the date).
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

/** `10:02` today, "Yesterday", `11-20` before. */
export function when(iso: string | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(today) - day(d)) / 86_400_000);
  if (diff <= 0) return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (diff === 1) return tr("Yesterday");
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
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
