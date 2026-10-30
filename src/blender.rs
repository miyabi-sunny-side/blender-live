//! Keeps one `blender -b` process alive and hands it one request at a time.
use crate::{
    basecamp::BaseCamp,
    protocol::{Message, Op, Reply, parse_message, request_line},
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    path::PathBuf,
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
    process::Command,
    sync::{Mutex, oneshot, watch},
};
use tracing::{debug, info, warn};

const BRIDGE: &str = include_str!("bridge.py");
const READY_TIMEOUT: Duration = Duration::from_mins(2);
const QUICK_TIMEOUT: Duration = Duration::from_mins(5);
const STILL_TIMEOUT: Duration = Duration::from_hours(1);
const LOG_TAIL: usize = 30;

pub struct Config {
    /// The Blender executable.
    pub bin: PathBuf,
    /// `nvidia-smi` indices handed to CUDA/OptiX; `None` leaves them all.
    pub gpus: Option<String>,
    pub camp: BaseCamp,
    /// Extra time after a Python timeout before the process is killed.
    pub grace: Duration,
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

struct Conn {
    generation: u64,
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    kill: Option<oneshot::Sender<()>>,
}

impl Conn {
    fn kill(&mut self) {
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }
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
    generation: AtomicU64,
    tail: Arc<StdMutex<VecDeque<String>>>,
    /// Private directory for the bridge script and the sockets.
    run_dir: PathBuf,
}

impl Drop for Blender {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.run_dir);
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
            generation: AtomicU64::new(0),
            tail: Arc::default(),
            run_dir,
        });
        let conn = blender.spawn().await?;
        *blender.conn.lock().await = Some(conn);
        Ok(blender)
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
        self.tail.lock().expect("log tail lock").clear();
        let started = Instant::now();
        let result = exchange(conn, id, &op, deadline, cancel, &mut progress).await;
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
                        self.tail_text()
                    )),
                    Exchange::Broken(e) => CallError::Crashed(e),
                })
            }
        }
    }

    fn display_path(&self, file: &str) -> String {
        self.cfg
            .camp
            .relative(std::path::Path::new(file))
            .unwrap_or_else(|| file.to_owned())
    }

    fn tail_text(&self) -> String {
        let tail = self.tail.lock().expect("log tail lock");
        tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    // Boxed so the restart path (spawn → exit watcher → spawn) stays `Send`.
    fn spawn(&self) -> Pin<Box<dyn Future<Output = Result<Conn, CallError>> + Send + '_>> {
        Box::pin(self.spawn_process())
    }

    async fn spawn_process(&self) -> Result<Conn, CallError> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let start_err = |e: std::io::Error| CallError::Start(format!("cannot start Blender: {e}"));
        let bridge = self.run_dir.join("bridge.py");
        let socket = self.run_dir.join(format!("{generation}.sock"));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).map_err(start_err)?;

        let mut command = Command::new(&self.cfg.bin);
        command
            .args(["-b", "--python-exit-code", "1", "--python"])
            .arg(&bridge)
            .arg("--")
            .arg("--socket")
            .arg(&socket)
            .current_dir(self.cfg.camp.root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(gpus) = &self.cfg.gpus {
            command
                .env("CUDA_DEVICE_ORDER", "PCI_BUS_ID")
                .env("CUDA_VISIBLE_DEVICES", gpus);
        }
        self.status.send_modify(|s| s.state = ProcState::Starting);
        let mut child = command.spawn().map_err(start_err)?;
        if let Some(out) = child.stdout.take() {
            self.forward(out);
        }
        if let Some(err) = child.stderr.take() {
            self.forward(err);
        }

        let accepted = tokio::select! {
            accepted = listener.accept() => accepted.map_err(start_err),
            exit = child.wait() => Err(CallError::Start(format!(
                "Blender exited before it was ready ({}):\n{}",
                exit.map_or_else(|e| e.to_string(), |s| s.to_string()),
                self.tail_text()
            ))),
            () = tokio::time::sleep(READY_TIMEOUT) => Err(CallError::Start(
                "Blender did not connect within 120 s".into(),
            )),
        };
        let _ = std::fs::remove_file(&socket);
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
        info!(%version, ?devices, ?device_type, pid = child.id(), "Blender ready");
        self.status.send_modify(|s| {
            s.state = ProcState::Ready;
            s.pid = child.id();
            s.version = Some(version);
            s.devices = devices;
            s.device_type = device_type;
            s.file = None;
            s.unsaved = false;
            s.busy_with = None;
        });

        let (kill, killed) = oneshot::channel();
        let me = self.me.clone();
        tokio::spawn(async move {
            let exit = tokio::select! {
                exit = child.wait() => exit,
                _ = killed => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            if let Some(blender) = me.upgrade() {
                blender.exited(generation, exit.ok()).await;
            }
        });
        Ok(Conn {
            generation,
            lines,
            writer,
            kill: Some(kill),
        })
    }

    fn forward(&self, stream: impl AsyncRead + Unpin + Send + 'static) {
        let tail = self.tail.clone();
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

    /// Records what the exit lost, then brings Blender back and reopens the
    /// last `.blend` so saved work is where it was.
    async fn exited(&self, generation: u64, exit: Option<ExitStatus>) {
        let mut slot = self.conn.lock().await;
        if self.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if slot.as_ref().is_some_and(|c| c.generation != generation) {
            return;
        }
        *slot = None;
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
        if let Some(file) = before.file
            && let Ok(path) = self.cfg.camp.existing(&file)
        {
            let op = Op::Open {
                path: path.to_string_lossy().into_owned(),
            };
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            match exchange(
                &mut conn,
                id,
                &op,
                Some(QUICK_TIMEOUT),
                None,
                &mut |_, _| {},
            )
            .await
            {
                Ok(reply) if reply.ok => {
                    self.status.send_modify(|s| s.file = Some(file));
                }
                _ => warn!(file, "could not reopen the last .blend"),
            }
        }
        *slot = Some(conn);
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
