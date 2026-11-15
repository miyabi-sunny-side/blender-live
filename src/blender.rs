//! Keeps one `blender -b` process alive and hands it one request at a time.
use crate::{
    basecamp::BaseCamp,
    gpu::GpuRetry,
    protocol::{Message, Op, Reply, parse_message, request_line},
    scratch::{CAMP_DIR, free_bytes, roomy},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::{ExitStatus, Stdio},
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader, Lines},
    net::{
        UnixListener,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    process::{Child, Command},
    sync::{Mutex, oneshot, watch},
};
use tracing::{debug, info, warn};

const BRIDGE: &str = include_str!("bridge.py");
const READY_TIMEOUT: Duration = Duration::from_mins(2);
const QUICK_TIMEOUT: Duration = Duration::from_mins(5);
const STILL_TIMEOUT: Duration = Duration::from_hours(1);
const LOG_TAIL: usize = 30;
/// A stop signal usually reaches Blender and this process together; an exit
/// waits this long for it before it counts as a crash.
const STOP_SETTLE: Duration = Duration::from_millis(300);
const STOPPING: &str = "blender-live is stopping";

pub struct Config {
    /// The Blender executable.
    pub bin: PathBuf,
    /// `nvidia-smi` indices handed to CUDA/OptiX; `None` leaves them all.
    pub gpus: Option<String>,
    pub camp: BaseCamp,
    /// Extra time after a Python timeout before the process is killed.
    pub grace: Duration,
    /// Pacing of the checks made while Blender runs without a GPU.
    pub gpu_retry: GpuRetry,
    /// Free bytes the temp directory needs before a job keeps its frames
    /// there; with less they go under the base camp.
    pub scratch_reserve: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcState {
    #[default]
    Starting,
    Ready,
    Busy,
    Restarting,
    Failed,
}

/// What the last crash took with it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Lost {
    pub at_ms: i64,
    pub exit_code: Option<i32>,
    /// The scene had changes that were never saved.
    pub unsaved: bool,
    /// The `.blend` that was open, reopened after the restart.
    pub file: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub state: ProcState,
    pub pid: Option<u32>,
    pub version: Option<String>,
    pub device_type: Option<String>,
    pub devices: Vec<String>,
    /// Why a GPU that is available now is not in use yet.
    pub gpu_note: Option<String>,
    /// Base camp path of the open `.blend` (absolute when outside).
    pub file: Option<String>,
    pub unsaved: bool,
    pub busy_with: Option<String>,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub lost: Option<Lost>,
}

#[derive(Debug)]
pub struct Outcome {
    pub value: Value,
    pub stdout: String,
    pub file: Option<String>,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum CallError {
    Failed {
        error: String,
        traceback: Option<String>,
        stdout: String,
        cancelled: bool,
    },
    Crashed(String),
    Timeout(String),
    Start(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { error, .. } => f.write_str(error),
            Self::Crashed(m) | Self::Timeout(m) | Self::Start(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CallError {}

type Tail = Arc<StdMutex<VecDeque<String>>>;

struct Conn {
    /// Which launch this is; tells the session's process from older and spare ones.
    launch: u64,
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    kill: Option<oneshot::Sender<()>>,
    /// The last lines the process printed.
    tail: Tail,
}

/// What the bridge reported when its process came up.
struct Greeting {
    pid: Option<u32>,
    version: String,
    devices: Vec<String>,
    device_type: Option<String>,
}

/// The result of one look for a GPU.
enum Check {
    /// Blender now runs on the GPU.
    Recovered,
    /// A GPU is there, but the session cannot be replaced yet.
    Waiting,
    Missing,
}

impl Conn {
    fn kill(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }

    fn tail_text(&self) -> String {
        tail_text(&self.tail)
    }
}

fn tail_text(tail: &Tail) -> String {
    let tail = tail.lock().expect("log tail lock");
    tail.iter().cloned().collect::<Vec<_>>().join("\n")
}

async fn stopped(mut stopping: watch::Receiver<bool>) {
    let _ = stopping.wait_for(|stopping| *stopping).await;
}

enum Exchange {
    Eof,
    Timeout,
    Broken(String),
}

pub struct Blender {
    cfg: Config,
    me: Weak<Blender>,
    conn: Mutex<Option<Conn>>,
    status: watch::Sender<Status>,
    next_id: AtomicU64,
    launches: AtomicU64,
    /// The launch that owns the session.
    current: AtomicU64,
    /// When the last request ended; `None` for a process nobody has used.
    last_request: StdMutex<Option<Instant>>,
    stopping: watch::Sender<bool>,
    /// Private directory for the bridge script, the sockets and the jobs' frames.
    run_dir: PathBuf,
}

impl Drop for Blender {
    fn drop(&mut self) {
        self.remove_temp();
    }
}

static INSTANCES: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Exec { .. } => "python",
        Op::RunScript { .. } => "run_script",
        Op::Summary => "scene_summary",
        Op::Open { .. } => "open_blend",
        Op::New { .. } => "new_blend",
        Op::Save { .. } => "save_blend",
        Op::RenderStill { .. } => "render_still",
        Op::RenderFrames { .. } => "render_animation",
        Op::Cancel => "cancel",
    }
}

impl Blender {
    /// Starts Blender and returns once the bridge inside it is ready.
    pub async fn start(cfg: Config) -> Result<Arc<Self>, CallError> {
        let run_dir = std::env::temp_dir().join(format!(
            "blender-live-{}-{}",
            std::process::id(),
            INSTANCES.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&run_dir)
            .and_then(|()| std::fs::write(run_dir.join("bridge.py"), BRIDGE))
            .map_err(|e| CallError::Start(format!("cannot prepare {}: {e}", run_dir.display())))?;
        let blender = Arc::new_cyclic(|me| Self {
            cfg,
            me: me.clone(),
            conn: Mutex::new(None),
            status: watch::Sender::new(Status::default()),
            next_id: AtomicU64::new(1),
            launches: AtomicU64::new(0),
            current: AtomicU64::new(0),
            last_request: StdMutex::new(None),
            stopping: watch::Sender::new(false),
            run_dir,
        });
        let conn = blender.spawn().await?;
        *blender.conn.lock().await = Some(conn);
        tokio::spawn(recover_gpu(
            Arc::downgrade(&blender),
            blender.subscribe(),
            blender.stopping.subscribe(),
            blender.cfg.gpu_retry,
        ));
        Ok(blender)
    }

    /// Ends Blender for good: nothing starts it again and requests are refused.
    pub fn stop(&self) {
        self.stopping.send_replace(true);
    }

    /// Resolves once [`Self::stop`] was called.
    pub async fn stopped(&self) {
        stopped(self.stopping.subscribe()).await;
    }

    fn is_stopping(&self) -> bool {
        *self.stopping.borrow()
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    pub fn camp(&self) -> &BaseCamp {
        &self.cfg.camp
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Whether a job may keep its frames in the run directory.
    pub fn temp_has_room(&self) -> bool {
        roomy(&free_bytes(&self.run_dir), self.cfg.scratch_reserve)
    }

    /// Removes the run directory and what jobs left under the base camp.
    /// For the end of the process: nothing can use Blender afterwards.
    pub fn remove_temp(&self) {
        let _ = std::fs::remove_dir_all(&self.run_dir);
        let _ = std::fs::remove_dir_all(self.cfg.camp.root().join(CAMP_DIR));
    }

    pub async fn call(
        &self,
        op: Op,
        cancel: Option<watch::Receiver<bool>>,
    ) -> Result<Outcome, CallError> {
        self.call_with_progress(op, cancel, |_, _| {}).await
    }

    /// Sends one request and waits for its reply. A dead or hung Blender is
    /// killed and restarted in the background; the call reports what happened.
    pub async fn call_with_progress(
        &self,
        op: Op,
        cancel: Option<watch::Receiver<bool>>,
        mut progress: impl FnMut(u32, u32),
    ) -> Result<Outcome, CallError> {
        if self.is_stopping() {
            return Err(CallError::Start(STOPPING.into()));
        }
        let mut slot = self.conn.lock().await;
        if slot.is_none() {
            *slot = Some(self.spawn().await?);
        }
        let conn = slot.as_mut().expect("connection was just ensured");
        let deadline = match &op {
            Op::Exec { timeout_secs, .. } | Op::RunScript { timeout_secs, .. } => {
                Some(Duration::from_secs(*timeout_secs) + self.cfg.grace)
            }
            Op::RenderStill { .. } => Some(STILL_TIMEOUT),
            Op::RenderFrames { .. } => None,
            _ => Some(QUICK_TIMEOUT),
        };
        self.status.send_modify(|s| {
            s.state = ProcState::Busy;
            s.busy_with = Some(op_name(&op).to_owned());
            if op.unsaved_after() == Some(true) {
                s.unsaved = true;
            }
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // A crash report should show what this request printed, not older renders.
        conn.tail.lock().expect("log tail lock").clear();
        let started = Instant::now();
        let result = exchange(conn, id, &op, deadline, cancel, &mut progress).await;
        *self.last_request.lock().expect("last request lock") = Some(Instant::now());
        match result {
            Ok(reply) => {
                let file = reply.file.as_deref().map(|f| self.display_path(f));
                self.status.send_modify(|s| {
                    s.state = ProcState::Ready;
                    s.busy_with = None;
                    s.file.clone_from(&file);
                    if reply.ok && op.unsaved_after() == Some(false) {
                        s.unsaved = false;
                    }
                });
                outcome(reply, file, started.elapsed())
            }
            Err(failure) => {
                if matches!(failure, Exchange::Eof) {
                    // Let the log readers catch the last lines of the dead process.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                conn.kill();
                let tail = conn.tail_text();
                *slot = None;
                self.status.send_modify(|s| {
                    s.state = ProcState::Restarting;
                    s.busy_with = None;
                });
                Err(match failure {
                    Exchange::Timeout => CallError::Timeout(format!(
                        "no reply within {} s; Blender was killed and restarts with a fresh scene",
                        deadline.unwrap_or_default().as_secs()
                    )),
                    Exchange::Eof => CallError::Crashed(format!(
                        "Blender exited during {}; it restarts and the unsaved scene is lost. Last output:\n{}",
                        op_name(&op),
                        tail
                    )),
                    Exchange::Broken(e) => CallError::Crashed(e),
                })
            }
        }
    }

    fn display_path(&self, file: &str) -> String {
        self.cfg
            .camp
            .relative(Path::new(file))
            .unwrap_or_else(|| file.to_owned())
    }

    fn idle(&self) -> Option<Duration> {
        let last = *self.last_request.lock().expect("last request lock");
        last.map(|at| at.elapsed())
    }

    // Boxed so the restart path (spawn → exit watcher → spawn) stays `Send`.
    fn spawn(&self) -> Pin<Box<dyn Future<Output = Result<Conn, CallError>> + Send + '_>> {
        Box::pin(self.spawn_process())
    }

    /// Starts the process that owns the session.
    async fn spawn_process(&self) -> Result<Conn, CallError> {
        self.status.send_modify(|s| s.state = ProcState::Starting);
        let (conn, greeting) = self.launch().await?;
        Ok(self.adopt(conn, greeting, None))
    }

    /// Starts a Blender process and waits for its bridge. The process is not
    /// the session's until it is adopted, and dies with its `Conn`.
    async fn launch(&self) -> Result<(Conn, Greeting), CallError> {
        if self.is_stopping() {
            return Err(CallError::Start(STOPPING.into()));
        }
        let launch = self.launches.fetch_add(1, Ordering::SeqCst) + 1;
        let start_err = |e: std::io::Error| CallError::Start(format!("cannot start Blender: {e}"));
        let socket = self.run_dir.join(format!("{launch}.sock"));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).map_err(start_err)?;

        let mut child = self.command(&socket).spawn().map_err(start_err)?;
        let tail = Tail::default();
        if let Some(out) = child.stdout.take() {
            forward(out, tail.clone());
        }
        if let Some(err) = child.stderr.take() {
            forward(err, tail.clone());
        }

        let greeted = async {
            let accepted = tokio::select! {
                accepted = listener.accept() => accepted.map_err(start_err),
                exit = child.wait() => Err(CallError::Start(format!(
                    "Blender exited before it was ready ({}):\n{}",
                    exit.map_or_else(|e| e.to_string(), |s| s.to_string()),
                    tail_text(&tail)
                ))),
                () = tokio::time::sleep(READY_TIMEOUT) => Err(CallError::Start(
                    "Blender did not connect within 120 s".into(),
                )),
            };
            let (stream, _) = accepted?;
            let (read, writer) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            let ready = tokio::time::timeout(READY_TIMEOUT, lines.next_line()).await;
            let Ok(Ok(Some(line))) = ready else {
                return Err(CallError::Start("the bridge never said ready".into()));
            };
            let Ok(Message::Ready {
                version,
                devices,
                device_type,
            }) = parse_message(&line)
            else {
                return Err(CallError::Start(format!("unexpected first line: {line}")));
            };
            Ok((lines, writer, version, devices, device_type))
        };
        let greeted = tokio::select! {
            greeted = greeted => greeted,
            () = self.stopped() => Err(CallError::Start(STOPPING.into())),
        };
        let _ = std::fs::remove_file(&socket);
        let (lines, writer, version, devices, device_type) = greeted?;
        let greeting = Greeting {
            pid: child.id(),
            version,
            devices,
            device_type,
        };

        let conn = Conn {
            launch,
            lines,
            writer,
            kill: Some(self.watch(child, launch)),
            tail,
        };
        Ok((conn, greeting))
    }

    fn command(&self, socket: &Path) -> Command {
        let mut command = Command::new(&self.cfg.bin);
        command
            .args(["-b", "--python-exit-code", "1", "--python"])
            .arg(self.run_dir.join("bridge.py"))
            .arg("--")
            .arg("--socket")
            .arg(socket)
            .current_dir(self.cfg.camp.root())
            // Without a display EEVEE draws through headless EGL on a render GPU. An inherited
            // DISPLAY would put it on whatever GPU drives the screen, often an integrated one.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(gpus) = &self.cfg.gpus {
            command
                .env("CUDA_DEVICE_ORDER", "PCI_BUS_ID")
                .env("CUDA_VISIBLE_DEVICES", gpus);
        }
        command
    }

    /// Follows a process to its end. The returned handle ends it early, as
    /// does dropping the handle or stopping.
    fn watch(&self, mut child: Child, launch: u64) -> oneshot::Sender<()> {
        let (kill, killed) = oneshot::channel();
        let me = self.me.clone();
        let stopping = self.stopping.subscribe();
        tokio::spawn(async move {
            let unwanted = async {
                tokio::select! {
                    _ = killed => {}
                    () = stopped(stopping) => {}
                }
            };
            let exit = tokio::select! {
                exit = child.wait() => exit,
                () = unwanted => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            if let Some(blender) = me.upgrade() {
                blender.exited(launch, exit.ok()).await;
            }
        });
        kill
    }

    /// Makes a launched process the session's, with `file` open in it.
    fn adopt(&self, conn: Conn, greeting: Greeting, file: Option<String>) -> Conn {
        let Greeting {
            pid,
            version,
            devices,
            device_type,
        } = greeting;
        info!(%version, ?devices, ?device_type, pid, "Blender ready");
        self.current.store(conn.launch, Ordering::SeqCst);
        *self.last_request.lock().expect("last request lock") = None;
        self.status.send_modify(|s| {
            s.state = ProcState::Ready;
            s.pid = pid;
            s.version = Some(version);
            s.devices = devices;
            s.device_type = device_type;
            s.gpu_note = None;
            s.file = file;
            s.unsaved = false;
            s.busy_with = None;
        });
        conn
    }

    /// Opens the `.blend` an earlier process had, so saved work is where it was.
    async fn reopen(&self, conn: &mut Conn, file: &str) -> bool {
        let Ok(path) = self.cfg.camp.existing(file) else {
            return false;
        };
        let op = Op::Open {
            path: path.to_string_lossy().into_owned(),
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let reply = exchange(conn, id, &op, Some(QUICK_TIMEOUT), None, &mut |_, _| {}).await;
        reply.is_ok_and(|reply| reply.ok)
    }

    /// Looks for a GPU with a second Blender beside the session, because the
    /// running one never sees a GPU that came late. If it has one and nothing
    /// would be lost, it becomes the session; otherwise it is ended again.
    async fn check_gpu(&self) -> Check {
        let retry = self.cfg.gpu_retry;
        let before = self.status();
        let replaceable = retry.replaceable(&before, self.idle());
        // Once a GPU was seen, Blender is only started again when it can take over.
        let unusable = matches!(
            before.state,
            ProcState::Starting | ProcState::Restarting | ProcState::Failed
        );
        if unusable || (before.gpu_note.is_some() && !replaceable) {
            return Check::Waiting;
        }
        let (mut spare, greeting) = match self.launch().await {
            Ok(launched) => launched,
            Err(e) => {
                debug!("the GPU check could not start Blender: {e}");
                return Check::Missing;
            }
        };
        if greeting.devices.is_empty() {
            // A GPU seen by an earlier check is gone again.
            self.status
                .send_if_modified(|s| s.gpu_note.take().is_some());
            return Check::Missing;
        }

        let before = self.status();
        if !before.devices.is_empty() {
            return Check::Recovered;
        }
        if retry.replaceable(&before, self.idle()) {
            if let Some(file) = &before.file
                && !self.reopen(&mut spare, file).await
            {
                debug!(file, "the GPU check could not reopen the .blend");
                return Check::Missing;
            }
            if let Ok(mut slot) = self.conn.try_lock() {
                let now = self.status();
                if retry.replaceable(&now, self.idle())
                    && (now.pid, &now.file) == (before.pid, &before.file)
                {
                    *slot = Some(self.adopt(spare, greeting, before.file));
                    info!("Blender was replaced by a new process that sees the GPU");
                    return Check::Recovered;
                }
            }
        }
        self.status.send_if_modified(|s| {
            let waiting = s.devices.is_empty() && s.gpu_note.is_none();
            if waiting {
                s.gpu_note = Some(format!(
                    "A GPU is available now. Blender moves to it once the scene has no unsaved \
                     changes and no request has arrived for {} s.",
                    retry.quiet.as_secs()
                ));
            }
            waiting
        });
        Check::Waiting
    }

    /// Records what the exit lost, then brings Blender back and reopens the
    /// last `.blend` so saved work is where it was.
    async fn exited(&self, launch: u64, exit: Option<ExitStatus>) {
        let mut slot = self.conn.lock().await;
        if self.current.load(Ordering::SeqCst) != launch {
            return;
        }
        *slot = None;
        if tokio::time::timeout(STOP_SETTLE, self.stopped())
            .await
            .is_ok()
        {
            info!("Blender stopped");
            return;
        }
        let before = self.status();
        let exit_code = exit.and_then(|s| s.code());
        let message = format!(
            "Blender exited ({}) and was restarted. {}",
            exit.map_or_else(|| "unknown status".into(), |s| s.to_string()),
            if before.unsaved {
                "The unsaved scene was lost."
            } else {
                "No unsaved changes were lost."
            }
        );
        warn!("{message}");
        let lost = Lost {
            at_ms: now_ms(),
            exit_code,
            unsaved: before.unsaved,
            file: before.file.clone(),
            message: message.clone(),
        };
        self.status.send_modify(|s| {
            s.state = ProcState::Restarting;
            s.restarts += 1;
            s.lost = Some(lost);
            s.last_error = Some(message);
        });
        let mut conn = None;
        for attempt in 1..=5 {
            match self.spawn().await {
                Ok(c) => {
                    conn = Some(c);
                    break;
                }
                Err(_) if self.is_stopping() => return,
                Err(e) => {
                    warn!(attempt, "restart failed: {e}");
                    self.status.send_modify(|s| {
                        s.state = ProcState::Failed;
                        s.last_error = Some(e.to_string());
                    });
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
        let Some(mut conn) = conn else { return };
        if let Some(file) = before.file {
            if self.reopen(&mut conn, &file).await {
                self.status.send_modify(|s| s.file = Some(file));
            } else {
                warn!(file, "could not reopen the last .blend");
            }
        }
        *slot = Some(conn);
    }
}

fn forward(stream: impl AsyncRead + Unpin + Send + 'static, tail: Tail) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            debug!(target: "blender", "{line}");
            let mut tail = tail.lock().expect("log tail lock");
            if tail.len() == LOG_TAIL {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
}

/// While Blender runs without a GPU, looks for one again: often at first,
/// then rarely, so a machine without a GPU is left alone.
async fn recover_gpu(
    blender: Weak<Blender>,
    mut status: watch::Receiver<Status>,
    stopping: watch::Receiver<bool>,
    retry: GpuRetry,
) {
    let mut delay = retry.first;
    loop {
        if status.wait_for(|s| s.devices.is_empty()).await.is_err() {
            return;
        }
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = stopped(stopping.clone()) => return,
        }
        let Some(blender) = blender.upgrade() else {
            return;
        };
        delay = match blender.check_gpu().await {
            Check::Recovered | Check::Waiting => retry.first,
            Check::Missing => retry.after(delay),
        };
    }
}

fn outcome(reply: Reply, file: Option<String>, elapsed: Duration) -> Result<Outcome, CallError> {
    if reply.ok {
        Ok(Outcome {
            value: reply.value,
            stdout: reply.stdout,
            file,
            elapsed,
        })
    } else {
        Err(CallError::Failed {
            error: reply.error.unwrap_or_else(|| "failed".into()),
            traceback: reply.traceback,
            stdout: reply.stdout,
            cancelled: reply.cancelled,
        })
    }
}

async fn exchange(
    conn: &mut Conn,
    id: u64,
    op: &Op,
    deadline: Option<Duration>,
    mut cancel: Option<watch::Receiver<bool>>,
    progress: &mut impl FnMut(u32, u32),
) -> Result<Reply, Exchange> {
    let broken = |e: std::io::Error| Exchange::Broken(format!("bridge connection failed: {e}"));
    conn.writer
        .write_all(request_line(id, op).as_bytes())
        .await
        .map_err(broken)?;
    let timeout = async {
        match deadline {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(timeout);
    let mut cancel_sent = false;
    loop {
        let wants_cancel = async {
            match cancel.as_mut() {
                Some(rx) if !cancel_sent => {
                    if *rx.borrow_and_update() {
                        return true;
                    }
                    rx.changed().await.is_ok() && *rx.borrow()
                }
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            line = conn.lines.next_line() => {
                let Some(line) = line.map_err(broken)? else {
                    return Err(Exchange::Eof);
                };
                match parse_message(&line) {
                    Ok(Message::Progress { id: of, done, total }) if of == id => progress(done, total),
                    Ok(Message::Reply(reply)) if reply.id == id => return Ok(reply),
                    Ok(_) => {}
                    Err(e) => return Err(Exchange::Broken(format!("bad bridge line {line:?}: {e}"))),
                }
            }
            yes = wants_cancel => {
                if yes {
                    conn.writer
                        .write_all(request_line(id, &Op::Cancel).as_bytes())
                        .await
                        .map_err(broken)?;
                    cancel_sent = true;
                } else {
                    cancel = None;
                }
            }
            () = &mut timeout => return Err(Exchange::Timeout),
        }
    }
}
