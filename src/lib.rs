#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod basecamp;
pub mod blender;
pub mod encode;
pub mod events;
pub mod files;
pub mod gpu;
pub mod http;
pub mod job;
pub mod mcp;
pub mod naming;
pub mod protocol;
pub mod render;
pub mod scratch;

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
    /// An outside page that shows renders instead of this server's own.
    pub viewer: Option<Arc<str>>,
}

impl AppState {
    pub fn new(blender: Arc<blender::Blender>, bus: events::Bus) -> Self {
        Self {
            blender,
            jobs: Arc::default(),
            cancels: Arc::default(),
            bus,
            viewer: None,
        }
    }

    /// Hands the viewing of renders to the page at `viewer`.
    #[must_use]
    pub fn with_viewer(mut self, viewer: Option<String>) -> Self {
        self.viewer = viewer.map(Arc::from);
        self
    }

    #[must_use]
    pub fn camp(&self) -> &basecamp::BaseCamp {
        self.blender.camp()
    }
}
