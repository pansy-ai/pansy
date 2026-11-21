// What the phone and the home pansy say to each other: protocol v1, as
// docs/plans/phone-protocol.md writes it. Change that page first.

/** `GET /.well-known/pansy/environment`: a machine, before any credential goes to it. */
export type Environment = {
  id: string;
  name: string;
  version: string;
  protocol: number;
  capabilities: string[];
  routes: Route[];
  /** Not in v1's page, but sent by the mock: the system it runs on. */
  os?: string;
};

export type Route = { kind: string; url: string };

/** `POST /api/auth/pair`'s answer. */
export type Paired = {
  deviceId: string;
  accessToken?: string;
  expiresIn?: number;
  scopes: string[];
  approval: "approved" | "pending";
  code?: string;
};

export type ThreadStatus = "working" | "approval" | "input" | "failed" | "done" | "idle" | "unsent";
export type Mode = "ask" | "edits" | "auto" | "full";

export type ThreadSummary = {
  id: string;
  agent: string;
  title: string;
  folder: string;
  workdir: string;
  branch?: string;
  worktree?: boolean;
  base?: string;
  model: string;
  effort?: string;
  mode: Mode;
  status: ThreadStatus;
  startedAt?: string;
  updatedAt: string;
  source: "pansy" | "terminal";
  session?: string;
  pr?: { number: number; url: string; state: "open" | "closed" | "merged" };
  diff?: { add: number; del: number };
  pinned?: boolean;
  archived?: boolean;
  snoozedUntil?: string;
  unread?: boolean;
  error?: string;
  queue?: number;
};

export type Turn = {
  role: "user" | "assistant" | "reasoning" | "tool" | "compact";
  name?: string;
  text: string;
  input?: string;
  output?: string;
  status?: string;
  at?: string;
};

export type Event =
  | { type: "thread"; thread: string; model: string }
  | { type: "turnStarted"; turn: string }
  | { type: "delta"; item: string; text: string }
  | { type: "message"; item: string; text: string }
  | { type: "user"; text: string }
  | { type: "toolStarted"; item: string; kind: string; title: string }
  | { type: "toolFinished"; item: string; kind: string; title: string; status: string; output?: string }
  | { type: "approval"; request: string; kind: string; title: string; detail: string; reason?: string }
  | { type: "question"; request: string; questions: Question[] }
  | { type: "resolved"; request: string }
  | { type: "turnFinished"; turn: string; status: string; error?: string }
  | { type: "exited"; code?: number };

/** One of a question's choices (Claude Code's AskUserQuestion). */
export type Question = { question: string; header: string; multiSelect: boolean; options: { label: string; description: string }[] };

export type Seq<T> = { seq: number; event: T };

export type Subscribed = { thread: ThreadSummary; history: Turn[]; live: Seq<Event>[]; seq: number };

export type Group = { id: string; name: string; members: number; routing: string };
export type Model = { id: string; name: string; context?: number; reasoning?: boolean; vision?: boolean };
export type Models = { groups: Group[]; providers: { id: string; name: string; models: Model[] }[] };

export type Project = { folder: string; name: string; git: boolean; branch?: string };

export type GitFile = { path: string; status: "added" | "modified" | "deleted" | "renamed" | "untracked"; add: number; del: number };
export type GitStatus = {
  branch: string;
  upstream?: string;
  ahead: number;
  behind: number;
  defaultBranch: string;
  onDefault: boolean;
  worktree: boolean;
  files: GitFile[];
  pr?: { number: number; url: string; state: string };
};

export type DiffLine = { kind: "+" | "-" | " "; old?: number; new?: number; text: string };
export type DiffFile = {
  path: string;
  oldPath?: string;
  status: string;
  add: number;
  del: number;
  binary?: boolean;
  hunks: { header: string; lines: DiffLine[] }[];
};
export type Diff = { files: DiffFile[]; truncated?: boolean };
export type DiffScope = "turn" | "lastTurn" | "uncommitted" | "base";

export type GitAction = "commit" | "push" | "commit_push" | "create_pr" | "commit_push_pr";
export type GitProgress = { threadId: string; step: "commit" | "push" | "pr"; status: "running" | "done" | "failed"; detail?: string };

export type Device = {
  id: string;
  name: string;
  platform: string;
  scopes: string[];
  approval: "approved" | "pending";
  createdAt: string;
  lastSeen?: string;
  current?: boolean;
};

export type Decision = "accept" | "acceptForSession" | "decline" | "cancel";
export type DispatchMode = "start" | "queue" | "steer" | "restart";
export type Workspace = { kind: "current" | "branch" | "worktree"; base?: string };

/** An error the home pansy answered with (`{"error": {code, message}}`), or the connection's own. */
export class RpcError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}
