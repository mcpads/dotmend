use crate::server::{SharedWorkspace, web_router};
use dotmend::{art::*, workbench_protocol::*, workspace::ToolOutput};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions, TryLockError},
    future::IntoFuture,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::watch, task::JoinHandle};

fn storage(error: impl std::fmt::Display) -> ArtError {
    ArtError::new("storage_error", error.to_string())
}
fn open_lock_file(path: &Path) -> ArtResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(storage)
}
fn locked_file(path: &Path) -> ArtResult<Option<File>> {
    let file = open_lock_file(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(storage(error)),
    }
}
fn runtime_directory() -> ArtResult<PathBuf> {
    // Share the existing default slot directory with earlier server installations.
    let directory = std::env::var_os("DOTMEND_RUNTIME_DIR")
        .or_else(|| std::env::var_os("RETRO_ART_RUNTIME_DIR"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("retro-art/workbenches"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache/retro-art/workbenches"))
        })
        .ok_or_else(|| {
            ArtError::new(
                "storage_error",
                "Set DOTMEND_RUNTIME_DIR; no default runtime directory is available",
            )
        })?;
    fs::create_dir_all(&directory).map_err(storage)?;
    Ok(directory)
}
struct Activity {
    active: bool,
    last: Instant,
    work_state: WorkState,
}
pub struct WorkbenchAccess {
    pub instance: WorkbenchInstance,
    control_id: String,
    activity: Mutex<Activity>,
    cancelled_actions: Mutex<HashSet<String>>,
}
impl WorkbenchAccess {
    pub fn apply<T>(&self, id: &str, operation: impl FnOnce() -> ArtResult<T>) -> ArtResult<T> {
        let mut activity = self.activity.lock().map_err(storage)?;
        if id != self.instance.workbench_id || !activity.active {
            return Err(ArtError::new(
                "workbench_conflict",
                "This workbench instance has ended. Reopen it through open_workbench",
            ));
        }
        // Check shutdown and storage under the same lock.
        // Failed actions do not extend the idle deadline.
        let result = operation()?;
        activity.last = Instant::now();
        Ok(result)
    }
    pub fn check_instance(&self, id: &str) -> ArtResult<()> {
        let activity = self.activity.lock().map_err(storage)?;
        if id != self.instance.workbench_id || !activity.active {
            return Err(ArtError::new(
                "workbench_conflict",
                "This workbench has ended. Reopen it through open_workbench",
            ));
        }
        Ok(())
    }
    fn active(&self) -> bool {
        self.activity.lock().is_ok_and(|a| a.active)
    }
    fn end(&self) {
        if let Ok(mut activity) = self.activity.lock() {
            activity.active = false;
        }
    }
    fn expired(&self) -> bool {
        let Ok(mut activity) = self.activity.lock() else {
            return true;
        };
        if !activity.active {
            return true;
        }
        if activity.work_state == WorkState::Working {
            return false;
        }
        if activity.last.elapsed() >= Duration::from_secs(self.instance.idle_timeout_seconds) {
            activity.active = false;
        }
        !activity.active
    }
}
struct WorkbenchReservation {
    access: Arc<WorkbenchAccess>,
    _workspace_lock: File,
    _slot: File,
}
impl Drop for WorkbenchReservation {
    fn drop(&mut self) {
        self.access.end();
    }
}
struct RunningWorkbench {
    access: Arc<WorkbenchAccess>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl RunningWorkbench {
    async fn finish(&mut self) {
        self.access.end();
        let _ = self.shutdown.send(true);
        // Storage completes under the control lock above.
        // Slow HTTP connections must not prevent shutdown indefinitely.
        if tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}
impl Drop for RunningWorkbench {
    fn drop(&mut self) {
        self.access.end();
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}
pub struct Workbench {
    root: PathBuf,
    running: Option<RunningWorkbench>,
}
impl Workbench {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            running: None,
        }
    }
    fn result(
        &self,
        state: WorkbenchState,
        instance: Option<WorkbenchInstance>,
    ) -> ArtResult<ToolOutput> {
        Ok(ToolOutput::new(
            serde_json::to_value(WorkbenchStatus {
                state,
                instance,
                max_workbenches: MAX_WORKBENCHES,
                work_state: None,
            })
            .map_err(storage)?,
        ))
    }
    fn descriptor(&self) -> ArtResult<Descriptor> {
        serde_json::from_slice(
            &fs::read(self.root.join(".dotmend/workbench.json")).map_err(storage)?,
        )
        .map_err(storage)
    }
    fn available(&self) -> ArtResult<bool> {
        Ok(locked_file(&self.root.join(".dotmend/workbench.lock"))?.is_some())
    }
    async fn forward(&self, tool: &str, arguments: serde_json::Value) -> ArtResult<ToolOutput> {
        let descriptor = self.descriptor().map_err(|_| {
            ArtError::new(
                "workbench_busy",
                "The workbench host is starting or stopping. Inspect again before retrying",
            )
        })?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(6))
            .build()
            .map_err(storage)?;
        let mut response = client.post(format!("http://127.0.0.1:{}/internal/control", descriptor.port))
            .header("x-retro-art-control", "mcp")
            .json(&serde_json::json!({"tool":tool,"arguments":arguments}))
            .send().await.map_err(|_| ArtError::new("workbench_unavailable", "Workbench host is unavailable. Inspect before retrying; do not replay a mutation blindly"))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(storage)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_SOURCE_BYTES {
                return Err(ArtError::new(
                    "limit_exceeded",
                    "Workbench response exceeds the size limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(storage)?;
        if value["ok"] == false {
            return Err(serde_json::from_value(value["error"].clone()).map_err(storage)?);
        }
        Ok(ToolOutput::new(value))
    }
    pub async fn inspect(&self, control_id: &str) -> ArtResult<ToolOutput> {
        validate_control(control_id)?;
        if self.available()? {
            return self.result(WorkbenchState::Closed, None);
        }
        if let Some(running) = &self.running
            && !running.task.is_finished()
        {
            return running.access.inspect(control_id);
        }
        self.forward(
            "inspect_workbench",
            serde_json::json!({"control_id":control_id}),
        )
        .await
    }
    pub async fn present(
        &self,
        input: ManagedPresentArt,
        workspace: SharedWorkspace,
    ) -> ArtResult<ToolOutput> {
        validate_control(&input.control_id)?;
        if let Some(running) = &self.running
            && !running.task.is_finished()
        {
            return running.access.present(input, workspace).await;
        }
        if self.available()? {
            return Err(ArtError::new(
                "workbench_conflict",
                "Workbench instance has ended. Open and inspect before presenting",
            ));
        }
        self.forward("present_art", serde_json::to_value(input).map_err(storage)?)
            .await
    }
    pub async fn open(
        &mut self,
        input: OpenWorkbench,
        workspace: SharedWorkspace,
    ) -> ArtResult<ToolOutput> {
        validate_control(&input.control_id)?;
        let idle = input.idle_timeout_seconds.unwrap_or(DEFAULT_IDLE_SECONDS);
        if !(1..=DEFAULT_IDLE_SECONDS).contains(&idle) {
            return Err(invalid("idle_timeout_seconds must be 1..1800"));
        }
        if let Some(running) = &mut self.running {
            if running.access.active() && !running.task.is_finished() {
                return running.access.reopen(&input.control_id, input.work_state);
            }
            if !running.task.is_finished() {
                running.finish().await;
            }
        }
        let Some(workspace_lock) = locked_file(&self.root.join(".dotmend/workbench.lock"))? else {
            return self
                .forward(
                    "open_workbench",
                    serde_json::to_value(input).map_err(storage)?,
                )
                .await.map_err(|error| if error.code == "workbench_unavailable" {
                    ArtError::new("workbench_busy", "A workbench host holds this workspace while starting or stopping. Inspect again before retrying")
                } else {error});
        };
        let directory = runtime_directory()?;
        let mut slot = None;
        for index in 0..MAX_WORKBENCHES {
            if let Some(file) = locked_file(&directory.join(format!("slot-{index}.lock")))? {
                slot = Some(file);
                break;
            }
        }
        let slot = slot.ok_or_else(|| ArtError::new("workbench_limit", "The shared limit is four workbenches. Close a finished instance using its control_id"))?;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(storage)?;
        let port = listener.local_addr().map_err(storage)?.port();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(storage)?
            .as_nanos();
        let instance = WorkbenchInstance {
            workbench_id: format!(
                "workbench_{}",
                digest(
                    format!(
                        "{}:{}:{nonce}:{port}",
                        self.root.display(),
                        std::process::id()
                    )
                    .as_bytes()
                )
            ),
            url: format!("http://127.0.0.1:{port}"),
            idle_timeout_seconds: idle,
        };
        let descriptor = Descriptor {
            port,
            workbench_id: instance.workbench_id.clone(),
            control_hash: digest(input.control_id.as_bytes()),
        };
        let mut file =
            tempfile::NamedTempFile::new_in(self.root.join(".dotmend")).map_err(storage)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&descriptor).map_err(storage)?)
            .map_err(storage)?;
        file.as_file().sync_all().map_err(storage)?;
        file.persist(self.root.join(".dotmend/workbench.json"))
            .map_err(storage)?;
        let work_state = input.work_state.unwrap_or(WorkState::Waiting);
        let access = Arc::new(WorkbenchAccess {
            control_id: input.control_id,
            instance: instance.clone(),
            activity: Mutex::new(Activity {
                active: true,
                last: Instant::now(),
                work_state,
            }),
            cancelled_actions: Mutex::new(HashSet::new()),
        });
        let app = web_router(workspace, port, access.clone());
        let (shutdown, mut requested) = watch::channel(false);
        let lifetime = access.clone();
        let task = tokio::spawn(async move {
            // Keep the lock file in place: the OS lock on this inode protects the entire execution.
            let _reservation = WorkbenchReservation {
                access: lifetime.clone(),
                _workspace_lock: workspace_lock,
                _slot: slot,
            };
            let (stop, mut stopping) = watch::channel(false);
            let server = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopping.changed().await;
                })
                .into_future();
            tokio::pin!(server);
            let expire = async {
                loop {
                    if *requested.borrow() || lifetime.expired() {
                        break;
                    }
                    tokio::select! {
                        _ = requested.changed() => {},
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                    }
                }
            };
            tokio::select! {
                _ = &mut server => {},
                _ = expire => {
                    lifetime.end();
                    let _ = stop.send(true);
                    let _ = tokio::time::timeout(Duration::from_secs(4), &mut server).await;
                }
            }
            lifetime.end();
        });
        let output = access.inspect(&access.control_id)?;
        self.running = Some(RunningWorkbench {
            access,
            shutdown,
            task,
        });
        Ok(output)
    }
    pub async fn close(&mut self, input: CloseWorkbench) -> ArtResult<ToolOutput> {
        validate_control(&input.control_id)?;
        if self.available()? {
            let descriptor = self.descriptor()?;
            if descriptor.workbench_id != input.workbench_id
                || descriptor.control_hash != digest(input.control_id.as_bytes())
            {
                return Err(ArtError::new(
                    "workbench_conflict",
                    "No matching workbench instance exists",
                ));
            }
            return self.result(WorkbenchState::Closed, None);
        }
        if let Some(running) = &mut self.running
            && !running.task.is_finished()
        {
            running.access.check_control(&input.control_id)?;
            if running.access.instance.workbench_id != input.workbench_id {
                return Err(ArtError::new(
                    "workbench_conflict",
                    "An old instance ID cannot close the current workbench",
                ));
            }
            running.finish().await;
            return self.result(WorkbenchState::Closed, None);
        }
        self.forward(
            "close_workbench",
            serde_json::to_value(&input).map_err(storage)?,
        )
        .await?;
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if self.available()? || self.descriptor()?.workbench_id != input.workbench_id {
                return self.result(WorkbenchState::Closed, None);
            }
            if Instant::now() >= deadline {
                return self.result(WorkbenchState::Closing, None);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Descriptor {
    port: u16,
    workbench_id: String,
    control_hash: String,
}
fn validate_control(id: &str) -> ArtResult<()> {
    if !(16..=128).contains(&id.len())
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid(
            "control_id must contain 16..128 ASCII letters, digits, underscores or hyphens. Use a fresh random ID per independent task",
        ));
    }
    Ok(())
}
impl WorkbenchAccess {
    fn check_control(&self, id: &str) -> ArtResult<()> {
        validate_control(id)?;
        if self.control_id != id {
            return Err(ArtError::new(
                "workbench_not_owned",
                "control_id does not own this workbench",
            ));
        }
        Ok(())
    }
    pub fn inspect(&self, id: &str) -> ArtResult<ToolOutput> {
        validate_control(id)?;
        let owned = self.control_id == id;
        let state = if !owned {
            WorkbenchState::Busy
        } else if self.active() {
            WorkbenchState::Owned
        } else {
            WorkbenchState::Closing
        };
        Ok(ToolOutput::new(
            serde_json::to_value(WorkbenchStatus {
                state,
                instance: owned.then(|| self.instance.clone()),
                max_workbenches: MAX_WORKBENCHES,
                work_state: if owned {
                    Some(self.activity.lock().map_err(storage)?.work_state)
                } else {
                    None
                },
            })
            .map_err(storage)?,
        ))
    }
    pub fn reopen(&self, id: &str, work_state: Option<WorkState>) -> ArtResult<ToolOutput> {
        validate_control(id)?;
        if id != self.control_id {
            return Err(ArtError::new(
                "workbench_busy",
                "Another control_id owns this workspace's workbench. Its controller must close it first",
            ));
        }
        {
            let mut activity = self.activity.lock().map_err(storage)?;
            if !activity.active {
                return Err(ArtError::new(
                    "workbench_conflict",
                    "This workbench has ended. Inspect and reopen it",
                ));
            }
            if let Some(work_state) = work_state {
                activity.work_state = work_state;
            }
            activity.last = Instant::now();
        }
        self.inspect(id)
    }
    pub fn human_action(
        &self,
        id: &str,
        action_id: Option<&str>,
        input: dotmend::presentation::HumanAction,
        recover: bool,
        workspace: SharedWorkspace,
    ) -> ArtResult<ToolOutput> {
        if let Some(action_id) = action_id {
            if !(16..=128).contains(&action_id.len())
                || !action_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid(
                    "x-dotmend-action must contain 16..128 ASCII letters, digits, underscores or hyphens",
                ));
            }
        } else if recover {
            return Err(invalid(
                "Recovery requires the original x-dotmend-action identifier",
            ));
        }
        self.apply(id, || {
            let mut cancelled = self.cancelled_actions.lock().map_err(storage)?;
            let mut workspace = workspace.lock().map_err(storage)?;
            if recover {
                let output = workspace.recover_human_action(&input)?;
                cancelled.insert(action_id.unwrap().to_owned());
                Ok(output)
            } else {
                if action_id.is_some_and(|id| cancelled.contains(id)) {
                    return Err(ArtError::new("action_cancelled", "This action was settled during recovery and cannot execute again. Read the current presentation before making a new edit"));
                }
                workspace.human_action(input)
            }
        })
    }
    pub async fn present(
        self: &Arc<Self>,
        input: ManagedPresentArt,
        workspace: SharedWorkspace,
    ) -> ArtResult<ToolOutput> {
        self.check_control(&input.control_id)?;
        let access = self.clone();
        tokio::task::spawn_blocking(move || {
            access.apply(&input.workbench_id, || {
                workspace.lock().map_err(storage)?.call(
                    "present_art",
                    serde_json::to_value(input.view).map_err(storage)?,
                )
            })
        })
        .await
        .map_err(storage)?
    }
    pub fn request_close(&self, input: CloseWorkbench) -> ArtResult<ToolOutput> {
        self.check_control(&input.control_id)?;
        self.apply(&input.workbench_id, || Ok(()))?;
        self.end();
        self.inspect(&input.control_id)
    }
}
