#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod basecamp;
pub mod blender;
pub mod encode;
pub mod events;
pub mod files;
pub mod http;
pub mod job;
pub mod mcp;
pub mod naming;
pub mod protocol;
pub mod render;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

pub use http::app;

/// Everything the HTTP handlers and MCP tools share.
#[derive(Clone)]
pub struct AppState {
    pub blender: Arc<blender::Blender>,
    pub jobs: Arc<Mutex<job::JobBook>>,
    /// Cancel switches of the running jobs.
    pub cancels: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    pub bus: events::Bus,
}

impl AppState {
    pub fn new(blender: Arc<blender::Blender>, bus: events::Bus) -> Self {
        Self {
            blender,
            jobs: Arc::default(),
            cancels: Arc::default(),
            bus,
        }
    }

    #[must_use]
    pub fn camp(&self) -> &basecamp::BaseCamp {
        self.blender.camp()
    }
}
