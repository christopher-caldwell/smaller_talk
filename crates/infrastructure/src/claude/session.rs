use std::{
    collections::HashMap,
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use serde_json::Value;
use smaller_talk_application::ExecutionError;
use tokio::{
    process::{Child, ChildStdin},
    sync::{Mutex, OwnedMutexGuard, mpsc},
    task::AbortHandle,
    time::{Instant, sleep_until},
};
use uuid::Uuid;

const IDLE_LIMIT: Duration = Duration::from_secs(120);
const FRESHNESS_LIMIT: Duration = Duration::from_secs(15 * 60);

#[derive(PartialEq, Eq)]
pub(super) struct Startup {
    pub executable: PathBuf,
    pub revision: u64,
    pub workspace: PathBuf,
    pub arguments: Vec<OsString>,
}

pub(super) struct ClaudeProcess {
    pub child: Child,
    pub stdin: ChildStdin,
    pub incoming: mpsc::Receiver<Result<Value, ExecutionError>>,
    pub commands: Vec<String>,
    pub started_at: Instant,
}

impl ClaudeProcess {
    async fn terminate(mut self) -> Result<(), ExecutionError> {
        if self.child.try_wait().map_err(process_error)?.is_none() {
            self.child.kill().await.map_err(process_error)?;
        }
        Ok(())
    }
}

fn process_error(error: std::io::Error) -> ExecutionError {
    ExecutionError::with_source("Could not stop the previous Claude process", error)
}

#[derive(Default)]
pub(crate) struct ClaudeSessions {
    idle: Mutex<Option<IdleSession>>,
    leases: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}

struct IdleSession {
    token: Uuid,
    chat: Uuid,
    native_id: String,
    startup: Startup,
    process: ClaudeProcess,
    expiry: AbortHandle,
}

/// The conversation service serializes execution and compaction for each chat.
/// Keep just one idle child across chats and reap it before opening the same
/// native session with different startup settings or for compaction.
impl ClaudeSessions {
    pub(super) async fn lease(&self, native_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut leases = self.leases.lock().await;
            leases.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = leases.get(native_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                leases.insert(native_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    pub(super) async fn take(
        &self,
        chat: Uuid,
        native_id: Option<&str>,
        startup: Option<&Startup>,
    ) -> Result<Option<ClaudeProcess>, ExecutionError> {
        let mut idle = self.idle.lock().await;
        if !idle.as_ref().is_some_and(|session| {
            session.chat == chat || native_id == Some(session.native_id.as_str())
        }) {
            return Ok(None);
        }

        let mut session = idle.take().expect("matching session");
        session.expiry.abort();
        if session.chat == chat
            && native_id == Some(session.native_id.as_str())
            && startup == Some(&session.startup)
            && session.process.started_at.elapsed() < FRESHNESS_LIMIT
            && session
                .process
                .child
                .try_wait()
                .map_err(process_error)?
                .is_none()
        {
            return Ok(Some(session.process));
        }

        session.process.terminate().await?;
        Ok(None)
    }

    pub(super) async fn park(
        self: &Arc<Self>,
        chat: Uuid,
        native_id: String,
        startup: Startup,
        process: ClaudeProcess,
    ) {
        let mut idle = self.idle.lock().await;
        if let Some(previous) = idle.take() {
            previous.expiry.abort();
            let _ = previous.process.terminate().await;
        }
        let cache = Arc::downgrade(self);
        let token = Uuid::new_v4();
        let expires_at = Instant::now() + IDLE_LIMIT;
        // Any take or replacement aborts this task while holding the same lock.
        let expiry = tokio::spawn(async move {
            sleep_until(expires_at).await;
            if let Some(cache) = cache.upgrade() {
                let mut idle = cache.idle.lock().await;
                if idle.as_ref().is_some_and(|session| session.token == token) {
                    let session = idle.take().expect("matching idle session");
                    let _ = session.process.terminate().await;
                }
            }
        })
        .abort_handle();
        *idle = Some(IdleSession {
            token,
            chat,
            native_id,
            startup,
            process,
            expiry,
        });
    }
}
