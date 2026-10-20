//! The server's log as vanilla's: `[HH:MM:SS] [Server thread/INFO]: message` on the console and in
//! `logs/latest.log`; the previous `latest.log` is compressed to `logs/<date>-<n>.log.gz` at startup.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// `YYYY-MM-DD` of `now`, in UTC (see [`local_parts`]).
pub fn date_stamp(now: SystemTime) -> String {
    let (y, mo, d, ..) = crate::javaprops::utc_parts(now);
    format!("{y:04}-{mo:02}-{d:02}")
}

/// Hours, minutes and seconds on the wall clock of the machine's time zone (UTC where it cannot be found).
pub fn local_parts(now: SystemTime) -> (u32, u32, u32) {
    #[cfg(unix)]
    {
        let secs = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
        // SAFETY: `localtime_r` writes only into the `tm` it is given.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&secs, &mut tm).is_null() {
                None
            } else {
                Some(tm)
            }
        };
        if let Some(tm) = tm {
            return (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32);
        }
    }
    let (_, _, _, h, mi, s) = crate::javaprops::utc_parts(now);
    (h as u32, mi as u32, s as u32)
}

/// Log4j's name for the thread: the simulation is vanilla's "Server thread".
pub fn thread_label(name: Option<&str>) -> String {
    match name {
        Some("sim") => "Server thread".to_owned(),
        Some("main") | None => "ServerMain".to_owned(),
        Some(other) => other.to_owned(),
    }
}

/// One line as vanilla prints it.
pub fn format_line(time: (u32, u32, u32), thread: &str, level: &Level, message: &str) -> String {
    let level = match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    };
    format!("[{:02}:{:02}:{:02}] [{thread}/{level}]: {message}", time.0, time.1, time.2)
}

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

struct VanillaFormat;

impl<S, N> FormatEvent<S, N> for VanillaFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> std::fmt::Result {
        let mut m = Message(String::new());
        event.record(&mut m);
        let thread = std::thread::current();
        let line = format_line(local_parts(SystemTime::now()), &thread_label(thread.name()), event.metadata().level(), &m.0);
        writeln!(writer, "{line}")
    }
}

/// Where the lines go: standard output and `logs/latest.log`.
#[derive(Clone)]
struct Tee {
    file: Arc<Mutex<std::fs::File>>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stdout().write_all(buf);
        let _ = self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner).write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stdout().flush();
        let _ = self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner).flush();
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Tee {
    type Writer = Tee;
    fn make_writer(&'a self) -> Tee {
        self.clone()
    }
}

/// Compresses `logs/latest.log` to the first free `<date>-<n>.log.gz` and removes it (`RollingRandomAccessFile` with an
/// on-startup trigger). Returns the archive's name, if there was a log to archive.
pub fn rotate(logs: &Path, now: SystemTime) -> std::io::Result<Option<String>> {
    let latest = logs.join("latest.log");
    let data = match std::fs::read(&latest) {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let date = date_stamp(now);
    let name = (1..).map(|n| format!("{date}-{n}.log.gz")).find(|n| !logs.join(n).exists()).expect("a free name");
    let mut gz = GzEncoder::new(std::fs::File::create(logs.join(&name))?, Compression::default());
    gz.write_all(&data)?;
    gz.finish()?;
    std::fs::remove_file(&latest)?;
    Ok(Some(name))
}

/// Starts logging the vanilla way into `logs/` under the working directory.
pub fn init_server_log(filter: tracing_subscriber::EnvFilter) -> std::io::Result<()> {
    let logs = Path::new("logs");
    std::fs::create_dir_all(logs)?;
    rotate(logs, SystemTime::now())?;
    let file = std::fs::OpenOptions::new().create(true).append(true).open(logs.join("latest.log"))?;
    let tee = Tee { file: Arc::new(Mutex::new(file)) };
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).event_format(VanillaFormat).with_writer(tee).init();
    Ok(())
}

#[cfg(test)]
mod tests;
