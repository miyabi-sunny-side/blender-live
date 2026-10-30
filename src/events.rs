//! Changes pushed to browsers: process status, jobs and base camp files.
use crate::{blender::Status, job::Job};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::{path::Path, time::Duration};
use tokio::sync::{broadcast, mpsc, watch};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    Status(Box<Status>),
    Jobs(Vec<Job>),
    /// Something under the base camp changed; lists should be fetched again.
    Files,
}

pub type Bus = broadcast::Sender<Event>;

#[must_use]
pub fn bus() -> Bus {
    broadcast::Sender::new(64)
}

/// Forwards every status change of the Blender process.
pub fn forward_status(mut status: watch::Receiver<Status>, bus: Bus) {
    tokio::spawn(async move {
        while status.changed().await.is_ok() {
            let snapshot = status.borrow_and_update().clone();
            let _ = bus.send(Event::Status(Box::new(snapshot)));
        }
    });
}

/// Watches the base camp and sends one `Files` event per burst of changes,
/// so a 48-frame render does not flood the browsers.
pub fn watch_files(root: &Path, bus: Bus) -> notify::Result<RecommendedWatcher> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|e| !e.kind.is_access()) {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_millis(300)).await;
            while rx.try_recv().is_ok() {}
            let _ = bus.send(Event::Files);
        }
    });
    Ok(watcher)
}
