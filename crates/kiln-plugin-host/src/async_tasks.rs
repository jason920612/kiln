//! The `async-tasks` world (`wit/async-tasks.wit`, WASI 0.3 component-model async).
//!
//! A plugin may ship a second component (`tasks.wasm`) for this world. It runs on a worker
//! thread of its own, on its own engine (component-model async and concurrency on, which the
//! game's hot path keeps off), one instance per plugin, not bound to any region. The game
//! never waits for it: the plugin's hooks hand it jobs (`jobs.submit`, committed like every
//! effect when the call returns normally), and a finished job comes back as an `op-result`
//! in the first B0 after it finished.
//!
//! What a task may do is what its manifest grants: HTTP to the hosts of its `http:<host>`
//! capabilities, `timers` (sleeping in server ticks, counted by the tick the host feeds in),
//! `storage` (a key-value store of its own in the plugin's data directory). A reload drops
//! the instance with its jobs in flight; the interrupted jobs are reported to the new
//! generation (`on-cancelled`, reason reload) so that it can submit them again.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};
use wasmtime::component::{Accessor, Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        world: "async-tasks",
        path: "../../wit",
        imports: { default: trappable },
    });
}

use bindings::AsyncTasks;
use bindings::exports::kiln::api::job_hooks as hooks;
use bindings::kiln::api::{http, storage, timers};

/// Largest response body, and the most a stored value or key may hold.
const MAX_BODY: usize = 1 << 20;
const MAX_KEY: usize = 256;
const MAX_KEYS: usize = 4096;
/// Memory a task instance may use.
const MEMORY_LIMIT: usize = 64 << 20;

/// A job for a plugin's tasks component.
pub(crate) struct JobIn {
    pub plugin: usize,
    pub generation: u32,
    pub ticket: u64,
    pub id: u64,
    pub kind: String,
    pub payload: Vec<u8>,
}

/// A finished job.
pub(crate) struct JobDone {
    pub plugin: usize,
    pub generation: u32,
    pub ticket: u64,
    /// The result bytes, or the failure text.
    pub result: Result<Vec<u8>, String>,
}

/// What a plugin's tasks component is granted.
#[derive(Clone)]
pub(crate) struct TaskGrants {
    pub id: Arc<str>,
    pub hosts: Vec<String>,
    pub timers: bool,
    pub storage: bool,
    pub data_dir: Option<PathBuf>,
}

enum Cmd {
    Load { plugin: usize, generation: u32, component: Component, grants: TaskGrants },
    /// Drops the instance (and with it every job in flight).
    Unload { plugin: usize },
    Job(JobIn),
    Stop,
}

/// The worker: its thread, the channel to it, and what it finished.
pub(crate) struct AsyncTasks_ {
    tx: tokio::sync::mpsc::UnboundedSender<Cmd>,
    pub(crate) engine: Engine,
    done: Arc<Mutex<Vec<JobDone>>>,
    tick: tokio::sync::watch::Sender<u64>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop_epoch: Arc<AtomicBool>,
    epoch_thread: Option<std::thread::JoinHandle<()>>,
}

pub(crate) fn engine() -> Result<Engine> {
    let mut c = wasmtime::Config::new();
    c.wasm_component_model(true);
    c.wasm_component_model_async(true);
    c.concurrency_support(true);
    // Tasks yield to each other every few milliseconds of compute instead of running to the end.
    c.epoch_interruption(true);
    Engine::new(&c).map_err(anyhow::Error::from)
}

impl AsyncTasks_ {
    pub(crate) fn start() -> Result<AsyncTasks_> {
        let engine = engine()?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (tick, tick_rx) = tokio::sync::watch::channel(0u64);
        let done = Arc::new(Mutex::new(Vec::new()));
        let (e, d) = (engine.clone(), done.clone());
        let thread = std::thread::Builder::new()
            .name("kiln-async-tasks".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, worker(e, rx, tick_rx, d));
            })
            .context("async task thread")?;
        let stop_epoch = Arc::new(AtomicBool::new(false));
        let (stop, ticker_engine) = (stop_epoch.clone(), engine.clone());
        let epoch_thread = std::thread::Builder::new()
            .name("kiln-async-epoch".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                    ticker_engine.increment_epoch();
                }
            })
            .context("async epoch thread")?;
        Ok(AsyncTasks_ { tx, engine, done, tick, thread: Some(thread), stop_epoch, epoch_thread: Some(epoch_thread) })
    }

    pub(crate) fn load(&self, plugin: usize, generation: u32, component: Component, grants: TaskGrants) {
        let _ = self.tx.send(Cmd::Load { plugin, generation, component, grants });
    }

    pub(crate) fn unload(&self, plugin: usize) {
        let _ = self.tx.send(Cmd::Unload { plugin });
    }

    pub(crate) fn submit(&self, job: JobIn) {
        let _ = self.tx.send(Cmd::Job(job));
    }

    /// The server tick: sleeping tasks count it.
    pub(crate) fn set_tick(&self, tick: u64) {
        self.tick.send_replace(tick);
    }

    /// The jobs finished since the last call.
    pub(crate) fn take_done(&self) -> Vec<JobDone> {
        std::mem::take(&mut *self.done.lock().unwrap())
    }
}

impl Drop for AsyncTasks_ {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        self.stop_epoch.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.epoch_thread.take() {
            let _ = t.join();
        }
    }
}

/// A plugin's key-value store: in memory, and in a file of the plugin's data directory.
struct Storage {
    path: Option<PathBuf>,
    kv: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl Storage {
    fn open(dir: Option<&PathBuf>) -> Storage {
        let path = dir.map(|d| d.join("tasks.kv"));
        let mut kv = BTreeMap::new();
        if let Some(bytes) = path.as_ref().and_then(|p| std::fs::read(p).ok()) {
            let mut r = &bytes[..];
            let mut take = |n: usize| -> Option<&[u8]> {
                (r.len() >= n).then(|| {
                    let (a, b) = r.split_at(n);
                    r = b;
                    a
                })
            };
            while let Some(kl) = take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) {
                let Some(k) = take(kl).map(<[u8]>::to_vec) else { break };
                let Some(vl) = take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else { break };
                let Some(v) = take(vl).map(<[u8]>::to_vec) else { break };
                if let Ok(k) = String::from_utf8(k) {
                    kv.insert(k, v);
                }
            }
        }
        Storage { path, kv: Mutex::new(kv) }
    }

    fn save(&self, kv: &BTreeMap<String, Vec<u8>>) {
        let Some(path) = &self.path else { return };
        let mut out = Vec::new();
        for (k, v) in kv {
            out.extend_from_slice(&(k.len() as u32).to_le_bytes());
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v);
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, out).and_then(|()| std::fs::rename(&tmp, path)).is_err() {
            warn!("cannot save {}", path.display());
        }
    }
}

struct TaskState {
    id: Arc<str>,
    hosts: Vec<String>,
    timers: bool,
    storage: Option<Arc<Storage>>,
    tick: tokio::sync::watch::Receiver<u64>,
    limits: StoreLimits,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for TaskState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

/// The host of a URL (`https://user@Host:8080/path` is `host`).
fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let host = if let Some(v6) = hostport.strip_prefix('[') { v6.split(']').next()? } else { hostport.split(':').next()? };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn do_fetch(req: http::Request) -> Result<http::Response, http::HttpError> {
    let method = req.method.to_ascii_uppercase();
    let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false).build();
    let agent: ureq::Agent = config.into();
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(req.url.as_str());
    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let body = req.body.unwrap_or_default();
    let request = builder.body(body).map_err(|e| http::HttpError::Denied(format!("bad request: {e}")))?;
    let mut response = agent.run(request).map_err(|e| match e {
        ureq::Error::Timeout(_) => http::HttpError::Timeout,
        other => http::HttpError::Failed(other.to_string()),
    })?;
    let status = response.status().as_u16();
    let headers = response.headers().iter().map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY as u64 + 1)
        .read_to_vec()
        .map_err(|e| http::HttpError::Failed(e.to_string()))?;
    if body.len() > MAX_BODY {
        return Err(http::HttpError::Failed("the response is longer than 1 MiB".into()));
    }
    Ok(http::Response { status, headers, body })
}

impl bindings::kiln::api::log::Host for TaskState {
    fn info(&mut self, msg: String) -> wasmtime::Result<()> {
        info!("[{}/tasks] {msg}", self.id);
        Ok(())
    }
    fn warn(&mut self, msg: String) -> wasmtime::Result<()> {
        warn!("[{}/tasks] {msg}", self.id);
        Ok(())
    }
    fn error(&mut self, msg: String) -> wasmtime::Result<()> {
        tracing::error!("[{}/tasks] {msg}", self.id);
        Ok(())
    }
}

impl http::Host for TaskState {}

impl<U: 'static> http::HostWithStore<U> for HasSelf<TaskState> {
    async fn fetch(store: &Accessor<U, Self>, req: http::Request) -> wasmtime::Result<Result<http::Response, http::HttpError>> {
        let allowed = store.with(|mut a| {
            let s = a.get();
            match url_host(&req.url) {
                Some(h) if s.hosts.iter().any(|x| *x == h) => Ok(()),
                Some(h) => Err(format!("no http:{h} capability")),
                None => Err("not an http(s) URL".to_owned()),
            }
        });
        if let Err(why) = allowed {
            return Ok(Err(http::HttpError::Denied(why)));
        }
        let result = tokio::task::spawn_blocking(move || do_fetch(req)).await.map_err(|e| wasmtime::format_err!("fetch: {e}"))?;
        Ok(result)
    }
}

impl timers::Host for TaskState {}

impl<U: 'static> timers::HostWithStore<U> for HasSelf<TaskState> {
    async fn sleep(store: &Accessor<U, Self>, ticks: u32) -> wasmtime::Result<()> {
        let (granted, mut rx) = store.with(|mut a| {
            let s = a.get();
            (s.timers, s.tick.clone())
        });
        if !granted {
            wasmtime::bail!("the plugin has no `timers` capability");
        }
        let target = *rx.borrow() + ticks.max(1) as u64;
        while *rx.borrow() < target {
            if rx.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }
}

impl storage::Host for TaskState {}

impl<U: 'static> storage::HostWithStore<U> for HasSelf<TaskState> {
    async fn get(store: &Accessor<U, Self>, key: String) -> wasmtime::Result<Option<Vec<u8>>> {
        let st = store.with(|mut a| a.get().storage.clone()).ok_or_else(|| wasmtime::format_err!("the plugin has no `storage` capability"))?;
        let v = st.kv.lock().unwrap().get(&key).cloned();
        Ok(v)
    }

    async fn put(store: &Accessor<U, Self>, key: String, val: Vec<u8>) -> wasmtime::Result<bool> {
        let st = store.with(|mut a| a.get().storage.clone()).ok_or_else(|| wasmtime::format_err!("the plugin has no `storage` capability"))?;
        if key.len() > MAX_KEY || val.len() > MAX_BODY {
            return Ok(false);
        }
        let mut kv = st.kv.lock().unwrap();
        if !kv.contains_key(&key) && kv.len() >= MAX_KEYS {
            return Ok(false);
        }
        kv.insert(key, val);
        st.save(&kv);
        Ok(true)
    }

    async fn delete(store: &Accessor<U, Self>, key: String) -> wasmtime::Result<()> {
        let st = store.with(|mut a| a.get().storage.clone()).ok_or_else(|| wasmtime::format_err!("the plugin has no `storage` capability"))?;
        let mut kv = st.kv.lock().unwrap();
        if kv.remove(&key).is_some() {
            st.save(&kv);
        }
        Ok(())
    }
}

/// One plugin's instance: its channel of jobs.
struct Running {
    jobs: tokio::sync::mpsc::UnboundedSender<JobIn>,
    handle: tokio::task::JoinHandle<()>,
}

async fn worker(
    engine: Engine,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Cmd>,
    tick: tokio::sync::watch::Receiver<u64>,
    done: Arc<Mutex<Vec<JobDone>>>,
) {
    let mut running: HashMap<usize, Running> = HashMap::new();
    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::Load { plugin, generation, component, grants } => {
                if let Some(old) = running.remove(&plugin) {
                    old.handle.abort();
                }
                let (jobs, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
                let (e, t, d) = (engine.clone(), tick.clone(), done.clone());
                let handle = tokio::task::spawn_local(async move {
                    if let Err(err) = instance_loop(e, component, grants.clone(), generation, jobs_rx, t, d).await {
                        warn!("plugin {}: tasks component stopped: {err:#}", grants.id);
                    }
                });
                running.insert(plugin, Running { jobs, handle });
            }
            Cmd::Unload { plugin } => {
                if let Some(old) = running.remove(&plugin) {
                    old.handle.abort();
                }
            }
            Cmd::Job(job) => match running.get(&job.plugin) {
                Some(r) => {
                    let _ = r.jobs.send(job);
                }
                None => done.lock().unwrap().push(JobDone {
                    plugin: job.plugin,
                    generation: job.generation,
                    ticket: job.ticket,
                    result: Err("the plugin has no tasks component running".into()),
                }),
            },
            Cmd::Stop => break,
        }
    }
    for (_, r) in running {
        r.handle.abort();
    }
}

async fn instance_loop(
    engine: Engine,
    component: Component,
    grants: TaskGrants,
    generation: u32,
    mut jobs: tokio::sync::mpsc::UnboundedReceiver<JobIn>,
    tick: tokio::sync::watch::Receiver<u64>,
    done: Arc<Mutex<Vec<JobDone>>>,
) -> Result<()> {
    use futures::stream::{FuturesUnordered, StreamExt};
    let mut linker: Linker<TaskState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    http::add_to_linker::<TaskState, HasSelf<TaskState>>(&mut linker, |s| s)?;
    timers::add_to_linker::<TaskState, HasSelf<TaskState>>(&mut linker, |s| s)?;
    storage::add_to_linker::<TaskState, HasSelf<TaskState>>(&mut linker, |s| s)?;
    bindings::kiln::api::log::add_to_linker::<TaskState, HasSelf<TaskState>>(&mut linker, |s| s)?;
    let storage = grants.storage.then(|| Arc::new(Storage::open(grants.data_dir.as_ref())));
    let state = TaskState {
        id: grants.id.clone(),
        hosts: grants.hosts.clone(),
        timers: grants.timers,
        storage,
        tick,
        limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(32).tables(32).memories(4).table_elements(1 << 16).build(),
        wasi: wasmtime_wasi::WasiCtxBuilder::new().build(),
        table: ResourceTable::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limits);
    store.epoch_deadline_async_yield_and_update(1);
    let instance = AsyncTasks::instantiate_async(&mut store, &component, &linker).await?;
    let plugin_id = grants.id.clone();
    store
        .run_concurrent(async |accessor| {
            let mut inflight = FuturesUnordered::new();
            let mut open = true;
            loop {
                tokio::select! {
                    job = jobs.recv(), if open => match job {
                        Some(job) => {
                            let (plugin, ticket, id) = (job.plugin, job.ticket, job.id);
                            let call = hooks::Job { id: job.id, kind: job.kind, payload: job.payload };
                            let fut = instance.kiln_api_job_hooks().call_run(accessor, call);
                            inflight.push(async move { (plugin, ticket, id, fut.await) });
                        }
                        None => open = false,
                    },
                    Some((plugin, ticket, _id, outcome)) = inflight.next(), if !inflight.is_empty() => {
                        let result = match outcome {
                            Ok(hooks::JobResult::Ok(bytes)) => Ok(bytes),
                            Ok(hooks::JobResult::Failed(why)) => Err(why),
                            Err(e) => Err(format!("the task trapped: {e:#}")),
                        };
                        done.lock().unwrap().push(JobDone { plugin, generation, ticket, result });
                    }
                    else => break,
                }
            }
        })
        .await?;
    let _ = plugin_id;
    Ok(())
}
