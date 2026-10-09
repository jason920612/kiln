//! The `async-tasks` world is not in this build (cargo feature `async-tasks` off): plugins
//! that ship a tasks component are not loaded, and the hot path keeps wasmtime without
//! component-model async. The types here let the rest of the host compile unchanged.

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::sync::Arc;
use wasmtime::Engine;
use wasmtime::component::Component;

pub(crate) struct JobIn {
    pub plugin: usize,
    pub generation: u32,
    pub ticket: u64,
    pub id: u64,
    pub kind: String,
    pub payload: Vec<u8>,
}

pub(crate) struct JobDone {
    pub generation: u32,
    pub ticket: u64,
    pub result: Result<Vec<u8>, String>,
}

#[derive(Clone)]
pub(crate) struct TaskGrants {
    pub id: Arc<str>,
    pub hosts: Vec<String>,
    pub timers: bool,
    pub storage: bool,
    pub data_dir: Option<PathBuf>,
}

pub(crate) struct AsyncTasks_ {
    pub(crate) engine: Engine,
}

impl AsyncTasks_ {
    pub(crate) fn start() -> Result<AsyncTasks_> {
        bail!("this build has no async-tasks support (cargo feature `async-tasks`)")
    }
    pub(crate) fn load(&self, _: usize, _: u32, _: Component, _: TaskGrants) {}
    pub(crate) fn unload(&self, _: usize) {}
    pub(crate) fn submit(&self, _: JobIn) {}
    pub(crate) fn set_tick(&self, _: u64) {}
    pub(crate) fn take_done(&self) -> Vec<JobDone> {
        Vec::new()
    }
}
