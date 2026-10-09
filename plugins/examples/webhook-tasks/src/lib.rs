//! The tasks component of the `webhook` plugin (the `async-tasks` world): what the game side
//! hands over as jobs, done off the tick.
//!
//! - `fetch`: a `GET` of the URL in the payload; the answer is `<status> <body>`.
//! - `post`: a `POST` of `<url>\n<body>`; the answer is `<status>`.
//! - `sleep`: sleeps the number of ticks in the payload, then answers `slept <n>`.
//! - `remember` / `recall`: a small key-value store of its own (`key=value`, `key`).
//! - `count`: counts invocations in its storage and answers the new count.

use kiln_tasks_sdk::*;

struct Webhook;

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Tasks for Webhook {
    async fn run(job: Job) -> JobResult {
        match job.kind.as_str() {
            "fetch" => match http::get(&text(&job.payload)).await {
                Ok(r) => {
                    let mut out = format!("{} ", r.status).into_bytes();
                    out.extend_from_slice(&r.body);
                    JobResult::Ok(out)
                }
                Err(e) => JobResult::Failed(format!("{e:?}")),
            },
            "post" => {
                let payload = text(&job.payload);
                let (url, body) = payload.split_once('\n').unwrap_or((&payload, ""));
                match http::post(url, "text/plain", body.as_bytes().to_vec()).await {
                    Ok(r) => JobResult::Ok(r.status.to_string().into_bytes()),
                    Err(e) => JobResult::Failed(format!("{e:?}")),
                }
            }
            "sleep" => {
                let ticks: u32 = text(&job.payload).trim().parse().unwrap_or(1);
                timers::sleep(ticks).await;
                JobResult::Ok(format!("slept {ticks}").into_bytes())
            }
            "remember" => {
                let payload = text(&job.payload);
                let (k, v) = payload.split_once('=').unwrap_or((&payload, ""));
                if storage::put(k, v.as_bytes().to_vec()).await { JobResult::Ok(b"remembered".to_vec()) } else { JobResult::Failed("storage full".into()) }
            }
            "recall" => match storage::get(&text(&job.payload)).await {
                Some(v) => JobResult::Ok(v),
                None => JobResult::Failed("nothing there".into()),
            },
            "count" => {
                let n = storage::get("count").await.and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok()).map_or(0, u64::from_le_bytes) + 1;
                storage::put("count", n.to_le_bytes().to_vec()).await;
                JobResult::Ok(n.to_string().into_bytes())
            }
            other => JobResult::Failed(format!("unknown job kind {other}")),
        }
    }
}

export_tasks!(Webhook);
