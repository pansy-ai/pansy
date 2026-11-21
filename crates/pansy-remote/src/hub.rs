//! The threads and the agents driving them, for every connected device at
//! once: one [`Hub`] per pansy. It starts an agent when a thread is
//! written to, keeps each running turn's events in memory (numbered, so a
//! device that reconnects asks for what it missed), queues what is sent
//! while a turn runs, and tells every device what changed.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use pansy_agents::sessions;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::codex::Codex;
use crate::store::{self, Thread};
use crate::{Decision, Event, Mode};

/// The events a running thread keeps for devices that reconnect.
const KEPT: usize = 2000;
/// A driven agent with no turn and nobody watching ends after this long.
const IDLE: Duration = Duration::from_secs(30 * 60);
/// The agents that can be driven.
pub const AGENTS: &[&str] = &["codex", "claude-code"];

/// An error the protocol names: `bad_request`, `not_found`, `conflict`,
/// `agent`.
#[derive(Debug)]
pub struct Coded(pub &'static str, pub String);

impl std::fmt::Display for Coded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

impl std::error::Error for Coded {}

fn coded(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Coded(code, message.into()))
}

/// A thread opened: its summary, its turns, the running turn's events after
/// the one asked for, and the last event's number.
pub type Opened = (Summary, Vec<sessions::Turn>, Vec<(u64, Event)>, u64);

/// What a device is told without asking (phone-protocol.md, 推送的事件).
#[derive(Debug, Clone)]
pub enum Push {
    Thread(Box<Summary>),
    Removed(String),
    Event {
        thread: String,
        seq: u64,
        event: Event,
    },
    Git {
        thread: String,
        step: &'static str,
        status: &'static str,
        detail: String,
    },
}

/// A thread as the list shows it (phone-protocol.md, ThreadSummary).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub agent: String,
    pub title: String,
    pub folder: String,
    pub workdir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub worktree: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub mode: Mode,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<store::PullRequest>,
    pub pinned: bool,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
    pub unread: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub queue: usize,
}

/// How a message goes to a thread (`message.dispatch`'s `dispatchMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    Start,
    Queue,
    Steer,
    Restart,
}

impl Dispatch {
    pub fn parse(word: &str) -> Option<Dispatch> {
        Some(match word {
            "start" => Dispatch::Start,
            "queue" => Dispatch::Queue,
            "steer" => Dispatch::Steer,
            "restart" => Dispatch::Restart,
            _ => return None,
        })
    }
}

/// What `thread.create` asks for.
#[derive(Debug, Clone)]
pub struct NewThread {
    pub folder: PathBuf,
    pub agent: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Mode,
    /// `current`, `branch` or `worktree`.
    pub workspace: String,
    pub base: Option<String>,
    pub text: String,
}

/// The changes `thread.update` makes.
#[derive(Debug, Clone, Default)]
pub struct Update {
    pub title: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<Mode>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
    pub snoozed_until: Option<Option<DateTime<Utc>>>,
    pub unread: Option<bool>,
}

struct Queued {
    text: String,
    model: Option<String>,
    effort: Option<String>,
}

/// An agent being driven.
enum Driver {
    Codex(Codex),
}

impl Driver {
    async fn start(
        agent: &str,
        launch: &pansy_agents::run::Launch,
        folder: &Path,
        resume: Option<&str>,
        mode: Mode,
    ) -> Result<(Driver, mpsc::UnboundedReceiver<Event>)> {
        match agent {
            "codex" => {
                let (c, rx) = Codex::start(launch, folder, resume, mode).await?;
                Ok((Driver::Codex(c), rx))
            }
            other => Err(coded("bad_request", format!("{other} can't be driven yet"))),
        }
    }

    async fn send(&self, text: &str, model: Option<&str>, effort: Option<&str>) -> Result<String> {
        match self {
            Driver::Codex(c) => c.send(text, model, effort).await,
        }
    }

    async fn steer(&self, turn: &str, text: &str) -> Result<()> {
        match self {
            Driver::Codex(c) => c.steer(turn, text).await,
        }
    }

    async fn interrupt(&self, turn: &str) -> Result<()> {
        match self {
            Driver::Codex(c) => c.interrupt(turn).await,
        }
    }

    async fn answer(&self, request: &str, decision: Decision) -> Result<()> {
        match self {
            Driver::Codex(c) => c.answer(request, decision).await,
        }
    }

    async fn stop(&self) {
        match self {
            Driver::Codex(c) => c.stop().await,
        }
    }
}

#[derive(Default)]
struct State {
    seq: u64,
    log: VecDeque<(u64, Event)>,
    turn: Option<String>,
    started: Option<DateTime<Utc>>,
    waiting: Vec<String>,
    queue: VecDeque<Queued>,
    used: Option<Instant>,
    ended: bool,
    /// What the agent's files held when the run started: the history shown
    /// beside the run's own events, which repeat what it writes since.
    history: Vec<sessions::Turn>,
}

struct Run {
    driver: Driver,
    /// The mode it was started in: another one restarts it.
    mode: Mode,
    state: Mutex<State>,
}

struct Inner {
    threads: Mutex<BTreeMap<String, Thread>>,
    runs: tokio::sync::Mutex<HashMap<String, Arc<Run>>>,
    /// A copy of the runs' states for summaries, which can't wait.
    live: Mutex<HashMap<String, Arc<Run>>>,
    push: broadcast::Sender<Push>,
    /// Threads a device watches, counted: an idle run with none ends.
    watched: Mutex<HashMap<String, usize>>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

impl Hub {
    pub fn load() -> Result<Hub> {
        let (push, _) = broadcast::channel(4096);
        let hub = Hub {
            inner: Arc::new(Inner {
                threads: Mutex::new(store::load()?),
                runs: Default::default(),
                live: Default::default(),
                push,
                watched: Default::default(),
            }),
        };
        let reaping = hub.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                reaping.reap().await;
            }
        });
        Ok(hub)
    }

    /// Everything that changes, for one device.
    pub fn listen(&self) -> broadcast::Receiver<Push> {
        self.inner.push.subscribe()
    }

    fn tell(&self, p: Push) {
        let _ = self.inner.push.send(p);
    }

    fn threads(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Thread>> {
        self.inner.threads.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self) {
        if let Err(e) = store::save(&self.threads()) {
            tracing::warn!("saving threads: {e:#}");
        }
    }

    fn run_of(&self, id: &str) -> Option<Arc<Run>> {
        self.inner
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn summary(&self, id: &str) -> Option<Summary> {
        let thread = self.threads().get(id).cloned()?;
        Some(self.summarize(&thread))
    }

    fn summarize(&self, t: &Thread) -> Summary {
        let run = self.run_of(&t.id);
        let (status, started, queue) = match run.as_ref() {
            Some(r) => {
                let s = r.state.lock().unwrap_or_else(|e| e.into_inner());
                let status = if !s.waiting.is_empty() {
                    "approval"
                } else if s.turn.is_some() {
                    "working"
                } else {
                    finished_status(t)
                };
                (
                    status,
                    s.started.filter(|_| s.turn.is_some()),
                    s.queue.len(),
                )
            }
            None => (finished_status(t), None, 0),
        };
        Summary {
            id: t.id.clone(),
            agent: t.agent.clone(),
            title: t.title.clone(),
            folder: t.folder.clone(),
            workdir: t.workdir.clone(),
            branch: t.branch.clone(),
            worktree: t.worktree,
            base: t.base.clone(),
            model: t.model.clone(),
            effort: t.effort.clone(),
            mode: t.mode,
            status,
            started_at: started,
            updated_at: t.updated_at,
            source: "pansy",
            session: t.session.clone(),
            pr: t.pr.clone(),
            pinned: t.pinned,
            archived: t.archived,
            snoozed_until: t.snoozed_until,
            unread: t.unread,
            error: t.error.clone(),
            queue,
        }
    }

    pub fn changed(&self, id: &str) {
        if let Some(s) = self.summary(id) {
            self.tell(Push::Thread(Box::new(s)));
        }
    }

    /// Every thread, then the agents' sessions on this computer that no
    /// thread holds yet (started in a terminal), the latest first.
    pub fn shell(&self) -> Vec<Summary> {
        let threads: Vec<Thread> = self.threads().values().cloned().collect();
        let held: std::collections::HashSet<String> =
            threads.iter().filter_map(|t| t.session.clone()).collect();
        let mut out: Vec<Summary> = threads.iter().map(|t| self.summarize(t)).collect();
        for s in sessions::list() {
            if !AGENTS.contains(&s.agent) || held.contains(&s.id) || s.cwd.is_empty() {
                continue;
            }
            out.push(Summary {
                id: format!("s:{}:{}", s.agent, s.id),
                agent: s.agent.to_owned(),
                title: s.title.clone(),
                folder: s.cwd.clone(),
                workdir: s.cwd.clone(),
                branch: None,
                worktree: false,
                base: None,
                model: s.models.last().cloned().unwrap_or_default(),
                effort: None,
                mode: Mode::Full,
                status: if s.active { "working" } else { "idle" },
                started_at: None,
                updated_at: s.last.unwrap_or_else(Utc::now),
                source: "terminal",
                session: Some(s.id.clone()),
                pr: None,
                pinned: false,
                archived: false,
                snoozed_until: None,
                unread: false,
                error: None,
                queue: 0,
            });
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        out
    }

    /// A thread's summary, what was said before (from the agent's files),
    /// and the running turn's events after `after`.
    pub fn open(&self, id: &str, after: Option<u64>) -> Result<Opened> {
        if let Some((agent, session)) = terminal_id(id) {
            let summary = self
                .shell()
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| coded("not_found", format!("no session {session}")))?;
            let history = sessions::transcript(agent, session, "").unwrap_or_default();
            return Ok((summary, history, Vec::new(), 0));
        }
        let thread = self
            .threads()
            .get(id)
            .cloned()
            .ok_or_else(|| coded("not_found", format!("no thread {id}")))?;
        if thread.unread {
            self.edit(id, |t| t.unread = false);
        }
        let running = self.run_of(id).map(|r| {
            let s = r.state.lock().unwrap_or_else(|e| e.into_inner());
            let after = after.unwrap_or(0);
            let live: Vec<(u64, Event)> =
                s.log.iter().filter(|(n, _)| *n > after).cloned().collect();
            (s.history.clone(), live, s.seq)
        });
        let (history, live, seq) = match running {
            Some(r) => r,
            None => (history_of(&thread), Vec::new(), 0),
        };
        Ok((self.summarize(&thread), history, live, seq))
    }

    /// A device watches `id` (or stops).
    pub fn watch(&self, id: &str, on: bool) {
        let mut w = self.inner.watched.lock().unwrap_or_else(|e| e.into_inner());
        let n = w.entry(id.to_owned()).or_default();
        if on {
            *n += 1;
        } else {
            *n = n.saturating_sub(1);
        }
    }

    /// A new thread, started with its first message.
    pub async fn create(&self, new: NewThread) -> Result<String> {
        if !AGENTS.contains(&new.agent.as_str()) {
            return Err(coded(
                "bad_request",
                format!("{} can't be driven", new.agent),
            ));
        }
        if !new.folder.is_dir() {
            return Err(coded(
                "bad_request",
                format!("no folder {}", new.folder.display()),
            ));
        }
        let folder = new.folder.to_string_lossy().into_owned();
        let (workdir, branch, worktree, base) =
            crate::workspace::prepare(&new.folder, &new.workspace, new.base.as_deref())?;
        let now = Utc::now();
        let id = store::new_id();
        let thread = Thread {
            id: id.clone(),
            agent: new.agent.clone(),
            title: title_of(&new.text),
            folder,
            workdir,
            branch,
            worktree,
            base,
            model: new.model.clone().unwrap_or_default(),
            effort: new.effort.clone(),
            mode: new.mode,
            session: None,
            created_at: now,
            updated_at: now,
            pinned: false,
            archived: false,
            snoozed_until: None,
            unread: false,
            pr: None,
            last: None,
            error: None,
            turns: 0,
            named: new.workspace != "worktree",
        };
        if thread.worktree {
            // Checkpoint 0: the worktree as made, what the first turn's diff
            // starts from.
            let (dir, tid) = (PathBuf::from(&thread.workdir), id.clone());
            if let Ok(Err(e)) =
                tokio::task::spawn_blocking(move || crate::git::checkpoint(&dir, &tid, 0)).await
            {
                tracing::warn!("checkpoint of {id}: {e:#}");
            }
        }
        self.threads().insert(id.clone(), thread);
        self.save();
        self.changed(&id);
        self.dispatch(&id, &new.text, Dispatch::Start, None, None)
            .await?;
        Ok(id)
    }

    /// Takes over a session started elsewhere: a thread that resumes it.
    fn adopt(&self, id: &str) -> Result<String> {
        let (agent, session) =
            terminal_id(id).ok_or_else(|| coded("not_found", "no such session"))?;
        let s = sessions::find(agent, session)
            .ok_or_else(|| coded("not_found", format!("no session {session}")))?;
        if s.active {
            return Err(coded(
                "conflict",
                "this session was written to in the last minute: it may be running elsewhere",
            ));
        }
        let now = Utc::now();
        let tid = store::new_id();
        let git = crate::git::is_repo(Path::new(&s.cwd));
        self.threads().insert(
            tid.clone(),
            Thread {
                id: tid.clone(),
                agent: agent.to_owned(),
                title: s.title.clone(),
                folder: s.cwd.clone(),
                workdir: s.cwd.clone(),
                branch: git
                    .then(|| crate::git::status(Path::new(&s.cwd)).ok().map(|g| g.branch))
                    .flatten(),
                worktree: false,
                base: None,
                model: s.models.last().cloned().unwrap_or_default(),
                effort: None,
                mode: Mode::Full,
                session: Some(s.id.clone()),
                created_at: s.start.unwrap_or(now),
                updated_at: now,
                pinned: false,
                archived: false,
                snoozed_until: None,
                unread: false,
                pr: None,
                last: None,
                error: None,
                turns: 0,
                named: true,
            },
        );
        self.save();
        self.tell(Push::Removed(id.to_owned()));
        self.changed(&tid);
        Ok(tid)
    }

    /// Sends `text` to a thread as `how` says.
    pub async fn dispatch(
        &self,
        id: &str,
        text: &str,
        how: Dispatch,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<String> {
        let id = if terminal_id(id).is_some() {
            self.adopt(id)?
        } else {
            id.to_owned()
        };
        if model.is_some() || effort.is_some() {
            self.edit(&id, |t| {
                if let Some(m) = &model {
                    t.model = m.clone();
                }
                if effort.is_some() {
                    t.effort = effort.clone();
                }
            });
        }
        let run = self.run(&id).await?;
        let queued = Queued {
            text: text.to_owned(),
            model: model.clone(),
            effort: effort.clone(),
        };
        let turn = run
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .turn
            .clone();
        match (turn, how) {
            (None, _) => self.send_now(&id, &run, queued).await?,
            (Some(_), Dispatch::Start | Dispatch::Queue) => {
                run.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .queue
                    .push_back(queued);
                self.changed(&id);
            }
            (Some(turn), Dispatch::Steer) => {
                self.log(
                    &id,
                    &run,
                    Event::User {
                        text: text.to_owned(),
                    },
                );
                run.driver.steer(&turn, text).await?;
            }
            (Some(turn), Dispatch::Restart) => {
                run.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .queue
                    .push_front(queued);
                run.driver.interrupt(&turn).await?;
            }
        }
        Ok(id)
    }

    async fn send_now(&self, id: &str, run: &Arc<Run>, q: Queued) -> Result<()> {
        self.log(
            id,
            run,
            Event::User {
                text: q.text.clone(),
            },
        );
        let thread = self.threads().get(id).cloned();
        let model = q.model.or_else(|| {
            thread
                .as_ref()
                .map(|t| t.model.clone())
                .filter(|m| !m.is_empty())
        });
        let effort = q
            .effort
            .or_else(|| thread.as_ref().and_then(|t| t.effort.clone()));
        {
            let mut s = run.state.lock().unwrap_or_else(|e| e.into_inner());
            s.started = Some(Utc::now());
            s.used = Some(Instant::now());
        }
        self.edit(id, |t| {
            t.error = None;
            if t.title.is_empty() {
                t.title = title_of(&q.text);
            }
        });
        let turn = run
            .driver
            .send(&q.text, model.as_deref(), effort.as_deref())
            .await?;
        // A turn's id may come before its `turnStarted`; either marks it.
        if !turn.is_empty() {
            run.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .turn
                .get_or_insert(turn);
        }
        self.changed(id);
        Ok(())
    }

    pub async fn respond(&self, id: &str, request: &str, decision: Decision) -> Result<()> {
        let run = self
            .run_of(id)
            .ok_or_else(|| coded("not_found", "nothing runs in this thread"))?;
        run.driver.answer(request, decision).await?;
        run.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting
            .retain(|r| r != request);
        self.log(
            id,
            &run,
            Event::Resolved {
                request: request.to_owned(),
            },
        );
        self.changed(id);
        Ok(())
    }

    pub async fn interrupt(&self, id: &str) -> Result<()> {
        let run = self
            .run_of(id)
            .ok_or_else(|| coded("not_found", "nothing runs in this thread"))?;
        let turn = run
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .turn
            .clone();
        match turn {
            Some(turn) => run.driver.interrupt(&turn).await,
            None => Err(coded("conflict", "no turn is running")),
        }
    }

    pub async fn update(&self, id: &str, u: Update) -> Result<Summary> {
        if !self.threads().contains_key(id) {
            return Err(coded("not_found", format!("no thread {id}")));
        }
        let restart = u
            .mode
            .is_some_and(|m| self.run_of(id).is_some_and(|r| r.mode != m));
        self.edit(id, |t| {
            if let Some(v) = &u.title {
                t.title = v.clone();
            }
            if let Some(v) = &u.model {
                t.model = v.clone();
            }
            if let Some(v) = &u.effort {
                t.effort = Some(v.clone()).filter(|e| !e.is_empty());
            }
            if let Some(v) = u.mode {
                t.mode = v;
            }
            if let Some(v) = u.pinned {
                t.pinned = v;
            }
            if let Some(v) = u.archived {
                t.archived = v;
            }
            if let Some(v) = u.snoozed_until {
                t.snoozed_until = v;
            }
            if let Some(v) = u.unread {
                t.unread = v;
            }
        });
        // Another mode is another way of starting the agent: the next
        // message starts it again, resumed.
        if restart
            && self.run_of(id).is_some_and(|r| {
                r.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .turn
                    .is_none()
            })
        {
            self.end(id).await;
        }
        self.summary(id).ok_or_else(|| coded("not_found", "gone"))
    }

    /// Forgets pansy's record of a thread; the agent's session stays.
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.end(id).await;
        if self.threads().remove(id).is_none() {
            return Err(coded("not_found", format!("no thread {id}")));
        }
        self.save();
        self.tell(Push::Removed(id.to_owned()));
        Ok(())
    }

    pub fn thread(&self, id: &str) -> Result<Thread> {
        self.threads()
            .get(id)
            .cloned()
            .ok_or_else(|| coded("not_found", format!("no thread {id}")))
    }

    pub fn set_pr(&self, id: &str, pr: store::PullRequest) {
        self.edit(id, |t| t.pr = Some(pr.clone()));
    }

    pub fn git_progress(&self, id: &str, step: &'static str, status: &'static str, detail: String) {
        self.tell(Push::Git {
            thread: id.to_owned(),
            step,
            status,
            detail,
        });
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut Thread)) {
        {
            let mut threads = self.threads();
            let Some(t) = threads.get_mut(id) else { return };
            f(t);
            t.updated_at = Utc::now();
        }
        self.save();
        self.changed(id);
    }

    /// The thread's run, started (or resumed) when there is none.
    async fn run(&self, id: &str) -> Result<Arc<Run>> {
        let mut runs = self.inner.runs.lock().await;
        if let Some(r) = runs.get(id)
            && !r.state.lock().unwrap_or_else(|e| e.into_inner()).ended
        {
            return Ok(Arc::clone(r));
        }
        let thread = self.thread(id)?;
        let agent = pansy_agents::run::by_name(&thread.agent)
            .ok_or_else(|| coded("bad_request", format!("no agent {}", thread.agent)))?;
        let workdir = PathBuf::from(&thread.workdir);
        // A project file counts only once the user said yes to it.
        let file = pansy_agents::run::find(&workdir)
            .and_then(|p| pansy_agents::run::read(&p).ok())
            .filter(|f| f.trusted);
        let model = Some(thread.model.as_str()).filter(|m| !m.is_empty());
        let launch = pansy_agents::run::launch(
            agent,
            &workdir,
            file.as_ref(),
            pansy_agents::run::Models {
                model,
                ..Default::default()
            },
            &[],
        )
        .map_err(|e| coded("agent", format!("{e:#}")))?;
        if thread.model.is_empty() {
            self.edit(id, |t| t.model = launch.model.clone());
        }
        let (driver, events) = Driver::start(
            &thread.agent,
            &launch,
            &workdir,
            thread.session.as_deref(),
            thread.mode,
        )
        .await
        .map_err(|e| coded("agent", format!("{e:#}")))?;
        let run = Arc::new(Run {
            driver,
            mode: thread.mode,
            state: Mutex::new(State {
                used: Some(Instant::now()),
                history: history_of(&thread),
                ..Default::default()
            }),
        });
        runs.insert(id.to_owned(), Arc::clone(&run));
        self.inner
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_owned(), Arc::clone(&run));
        tokio::spawn(self.clone().follow(id.to_owned(), Arc::clone(&run), events));
        Ok(run)
    }

    /// Numbers an event, keeps it, and tells the devices.
    fn log(&self, id: &str, run: &Run, event: Event) -> u64 {
        let seq = {
            let mut s = run.state.lock().unwrap_or_else(|e| e.into_inner());
            s.seq += 1;
            let seq = s.seq;
            s.log.push_back((seq, event.clone()));
            while s.log.len() > KEPT {
                s.log.pop_front();
            }
            seq
        };
        self.tell(Push::Event {
            thread: id.to_owned(),
            seq,
            event,
        });
        seq
    }

    /// Reads a run's events until its agent ends.
    async fn follow(self, id: String, run: Arc<Run>, mut events: mpsc::UnboundedReceiver<Event>) {
        while let Some(event) = events.recv().await {
            let mut status_changed = false;
            match &event {
                Event::Thread { thread, model } => {
                    let (thread, model) = (thread.clone(), model.clone());
                    self.edit(&id, |t| {
                        t.session = Some(thread);
                        if t.model.is_empty() {
                            t.model = model;
                        }
                    });
                }
                Event::TurnStarted { turn } => {
                    let mut s = run.state.lock().unwrap_or_else(|e| e.into_inner());
                    s.turn = Some(turn.clone());
                    s.started.get_or_insert_with(Utc::now);
                    status_changed = true;
                }
                Event::Approval { request, .. } => {
                    run.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .waiting
                        .push(request.clone());
                    status_changed = true;
                }
                Event::Resolved { request } => {
                    run.state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .waiting
                        .retain(|r| r != request);
                    status_changed = true;
                }
                Event::TurnFinished { status, error, .. } => {
                    {
                        let mut s = run.state.lock().unwrap_or_else(|e| e.into_inner());
                        s.turn = None;
                        s.started = None;
                        s.waiting.clear();
                        s.used = Some(Instant::now());
                    }
                    let watched = self.watched(&id);
                    let (status, error) = (status.clone(), error.clone());
                    self.edit(&id, |t| {
                        t.last = Some(status.clone());
                        t.error = Some(error.clone()).filter(|e| !e.is_empty());
                        t.turns += 1;
                        t.unread = !watched;
                    });
                    self.after_turn(&id).await;
                    crate::notify::turn_finished(&self.thread(&id).ok(), &status);
                }
                Event::Exited { .. } => {
                    run.state.lock().unwrap_or_else(|e| e.into_inner()).ended = true;
                }
                _ => {}
            }
            if let Event::Approval { title, .. } = &event {
                crate::notify::approval(&self.thread(&id).ok(), title);
            }
            self.log(&id, &run, event.clone());
            if status_changed {
                self.changed(&id);
            }
            if matches!(event, Event::TurnFinished { .. }) {
                let next = run
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .queue
                    .pop_front();
                if let Some(next) = next
                    && let Err(e) = self.send_now(&id, &run, next).await
                {
                    tracing::warn!("sending the queued message of {id}: {e:#}");
                }
            }
        }
        {
            let mut s = run.state.lock().unwrap_or_else(|e| e.into_inner());
            s.ended = true;
            s.turn = None;
            s.waiting.clear();
        }
        self.inner
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        self.changed(&id);
    }

    /// After each turn: a checkpoint of a worktree, and the branch named
    /// after what the thread is about.
    async fn after_turn(&self, id: &str) {
        let Ok(t) = self.thread(id) else { return };
        if !t.worktree {
            return;
        }
        let dir = PathBuf::from(&t.workdir);
        let (tid, n) = (t.id.clone(), t.turns);
        let checkpoint =
            tokio::task::spawn_blocking(move || crate::git::checkpoint(&dir, &tid, n)).await;
        if let Ok(Err(e)) = checkpoint {
            tracing::warn!("checkpoint of {id}: {e:#}");
        }
        if !t.named {
            if let Some(name) = crate::workspace::branch_name(&t).await {
                self.edit(id, |th| {
                    th.branch = Some(name.clone());
                    th.named = true;
                });
            } else {
                self.edit(id, |th| th.named = true);
            }
        }
    }

    fn watched(&self, id: &str) -> bool {
        self.inner
            .watched
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .is_some_and(|n| *n > 0)
    }

    async fn end(&self, id: &str) {
        let run = self.inner.runs.lock().await.remove(id);
        self.inner
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        if let Some(run) = run {
            run.driver.stop().await;
        }
    }

    /// Ends the runs nobody uses.
    async fn reap(&self) {
        let idle: Vec<String> = {
            let runs = self.inner.runs.lock().await;
            runs.iter()
                .filter(|(id, r)| {
                    let s = r.state.lock().unwrap_or_else(|e| e.into_inner());
                    s.ended
                        || (s.turn.is_none()
                            && s.queue.is_empty()
                            && !self.watched(id)
                            && s.used.is_some_and(|u| u.elapsed() > IDLE))
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in idle {
            self.end(&id).await;
        }
    }

    /// Ends every run: pansy is stopping.
    pub async fn shutdown(&self) {
        let ids: Vec<String> = self.inner.runs.lock().await.keys().cloned().collect();
        for id in ids {
            self.end(&id).await;
        }
    }
}

fn history_of(t: &Thread) -> Vec<sessions::Turn> {
    t.session
        .as_deref()
        .and_then(|s| sessions::transcript(&t.agent, s, "").ok())
        .unwrap_or_default()
}

fn finished_status(t: &Thread) -> &'static str {
    match t.last.as_deref() {
        Some("failed") => "failed",
        _ if t.unread => "done",
        _ => "idle",
    }
}

/// `s:<agent>:<session>`: a session no thread holds yet.
fn terminal_id(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix("s:")?;
    rest.split_once(':')
}

fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    if line.chars().count() <= 60 {
        return line.to_owned();
    }
    line.chars().take(59).collect::<String>() + "…"
}

/// A thread's id from a request's params.
pub fn thread_param(params: &Value) -> Result<String> {
    params["threadId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| coded("bad_request", "threadId is missing"))
}
