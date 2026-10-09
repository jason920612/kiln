//! Guest SDK for the `async-tasks` world of `kiln:api` (WASI 0.3 component-model async).
//!
//! A plugin may ship a second component, `tasks.wasm`, next to its `plugin.wasm`. It is built
//! for this world: it implements [`Tasks`], whose `run` is an `async fn` that may be in flight
//! many times at once, and can `await` [`http::fetch`], [`timers::sleep`] and
//! [`storage`] calls. It reaches the game only through its jobs: the plugin's game hooks
//! submit one (`kiln_plugin_sdk::jobs::submit`), `run` returns the answer, and the answer
//! arrives in the game hook `on_results` as an `OpResult` (`applied` true with the bytes in
//! `value`, or false with the failure text).
//!
//! ```ignore
//! use kiln_tasks_sdk::*;
//!
//! struct Fetcher;
//! impl Tasks for Fetcher {
//!     async fn run(job: Job) -> JobResult {
//!         match http::get(&String::from_utf8_lossy(&job.payload)).await {
//!             Ok(r) => JobResult::Ok(r.body),
//!             Err(e) => JobResult::Failed(format!("{e:?}")),
//!         }
//!     }
//! }
//! export_tasks!(Fetcher);
//! ```

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "async-tasks",
        path: "../../wit",
        async: true,
        pub_export_macro: true,
        export_macro_name: "export_raw",
        default_bindings_module: "kiln_tasks_sdk::bindings",
    });
}

pub use bindings::exports::kiln::api::job_hooks::{Job, JobResult};

/// HTTP from a task: only to the hosts of the plugin's `http:<host>` capabilities.
pub mod http {
    pub use crate::bindings::kiln::api::http::{HttpError, Request, Response, fetch};

    /// A `GET`.
    pub async fn get(url: &str) -> Result<Response, HttpError> {
        fetch(Request { method: "GET".to_owned(), url: url.to_owned(), headers: Vec::new(), body: None }).await
    }

    /// A `POST` with a body.
    pub async fn post(url: &str, content_type: &str, body: Vec<u8>) -> Result<Response, HttpError> {
        fetch(Request {
            method: "POST".to_owned(),
            url: url.to_owned(),
            headers: vec![("content-type".to_owned(), content_type.to_owned())],
            body: Some(body),
        })
        .await
    }
}

/// Sleeping in server ticks.
pub mod timers {
    pub use crate::bindings::kiln::api::timers::sleep;
}

/// The task's own small key-value store.
pub mod storage {
    pub use crate::bindings::kiln::api::storage::{delete, get, put};
}

/// Console logging.
pub mod log {
    pub use crate::bindings::kiln::api::log::{error, info, warn};
}

/// The jobs a plugin's tasks component runs.
#[allow(async_fn_in_trait)]
pub trait Tasks {
    /// Runs one job. Many may be in flight at once; the answer goes back to the game.
    async fn run(job: Job) -> JobResult;
}

/// Exports a [`Tasks`] implementation as the component's `job-hooks`.
#[macro_export]
macro_rules! export_tasks {
    ($t:ty) => {
        struct __KilnTasksExports;
        impl $crate::bindings::exports::kiln::api::job_hooks::Guest for __KilnTasksExports {
            async fn run(j: $crate::Job) -> $crate::JobResult {
                <$t as $crate::Tasks>::run(j).await
            }
        }
        $crate::bindings::export_raw!(__KilnTasksExports with_types_in $crate::bindings);
    };
}
