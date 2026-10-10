//! SQL history (31-query-results.md §6). Every statement a driver sends goes through [`logged`]:
//! it is kept in memory for the history log viewer and appended to `QueryExec.log` on a
//! background thread, so callers never wait on the file.

use std::collections::VecDeque;
use std::future::Future;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::driver::DbResult;

/// Entries the viewer can show. ponytail: older ones are only in the file.
pub const KEEP: usize = 10_000;
pub const FILE_NAME: &str = "QueryExec.log";

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: chrono::DateTime<chrono::Local>,
    pub connection: String,
    /// 0 when the server has no session ID (SQLite)
    pub session: u64,
    /// The driver name in capitals: MYSQL, MARIADB, POSTGRESQL, SQLITE
    pub kind: String,
    pub elapsed: Duration,
    pub sql: String,
    pub error: Option<String>,
}

/// Which header fields the viewer shows; the file always has all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub date: bool,
    pub time: bool,
    pub server: bool,
    pub session: bool,
    pub kind: bool,
    pub elapsed: bool,
}

impl Header {
    pub const ALL: Self = Self {
        date: true,
        time: true,
        server: true,
        session: true,
        kind: true,
        elapsed: true,
    };
}

impl LogEntry {
    pub fn header(&self, h: Header) -> String {
        let mut out = String::new();
        let stamp = match (h.date, h.time) {
            (true, true) => self.time.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            (true, false) => self.time.format("%Y-%m-%d").to_string(),
            (false, true) => self.time.format("%H:%M:%S%.3f").to_string(),
            (false, false) => String::new(),
        };
        let fields = [
            (h.date || h.time, stamp),
            (h.server, self.connection.clone()),
            (h.session, self.session.to_string()),
            (h.kind, self.kind.clone()),
            (h.elapsed, format!("{:.3}s", self.elapsed.as_secs_f64())),
        ];
        for (shown, text) in fields {
            if shown {
                out.push('[');
                out.push_str(&text);
                out.push(']');
            }
        }
        out
    }

    /// The statement record, then an error record with the same header when it failed.
    pub fn render(&self, h: Header) -> String {
        let header = self.header(h);
        let mut out = format!("{header}\n{}\n\n", self.sql.trim_end());
        if let Some(e) = &self.error {
            out.push_str(&format!("{header}\n{e}\n\n"));
        }
        out
    }
}

struct Ring {
    entries: VecDeque<LogEntry>,
    /// Sequence number of `entries[0]`
    first: u64,
}

static RING: LazyLock<Mutex<Ring>> = LazyLock::new(|| {
    Mutex::new(Ring {
        entries: VecDeque::new(),
        first: 0,
    })
});
static NEXT: AtomicU64 = AtomicU64::new(0);

enum Msg {
    Entry(Box<LogEntry>),
    Dir(PathBuf),
}

static WRITER: LazyLock<Mutex<Option<mpsc::Sender<Msg>>>> = LazyLock::new(|| Mutex::new(None));

/// Points `QueryExec.log` at `dir`, starting an empty file there. The first call (at startup)
/// also starts the writer; before it, entries stay in memory only.
pub fn set_dir(dir: PathBuf) {
    let mut writer = WRITER.lock().unwrap_or_else(|p| p.into_inner());
    let tx = writer.get_or_insert_with(|| {
        let (tx, rx) = mpsc::channel::<Msg>();
        std::thread::Builder::new()
            .name("navidog-sql-log".into())
            .spawn(move || write_loop(rx))
            .expect("start the SQL log thread");
        tx
    });
    let _ = tx.send(Msg::Dir(dir));
}

fn write_loop(rx: mpsc::Receiver<Msg>) {
    let mut file: Option<std::fs::File> = None;
    for msg in rx {
        match msg {
            Msg::Dir(dir) => {
                file = std::fs::create_dir_all(&dir)
                    .and_then(|_| std::fs::File::create(dir.join(FILE_NAME)))
                    .inspect_err(|e| tracing::warn!("cannot open {FILE_NAME}: {e}"))
                    .ok();
            }
            Msg::Entry(entry) => {
                if let Some(f) = file.as_mut() {
                    let _ = f.write_all(entry.render(Header::ALL).as_bytes());
                }
            }
        }
    }
}

pub fn record(entry: LogEntry) {
    if let Some(tx) = WRITER.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tx.send(Msg::Entry(Box::new(entry.clone())));
    }
    let mut ring = RING.lock().unwrap_or_else(|p| p.into_inner());
    ring.entries.push_back(entry);
    if ring.entries.len() > KEEP {
        ring.entries.pop_front();
        ring.first += 1;
    }
    NEXT.fetch_add(1, Ordering::Release);
}

/// Sequence number the next entry gets; it changes whenever an entry is recorded.
pub fn revision() -> u64 {
    NEXT.load(Ordering::Acquire)
}

/// Entries from sequence number `from` on (older ones that fell out of memory are skipped), and
/// the number to ask from next time.
pub fn since(from: u64) -> (Vec<LogEntry>, u64) {
    let ring = RING.lock().unwrap_or_else(|p| p.into_inner());
    let skip = from.saturating_sub(ring.first) as usize;
    let entries: Vec<_> = ring.entries.iter().skip(skip).cloned().collect();
    (entries, ring.first + ring.entries.len() as u64)
}

/// Who sent a statement, stamped on each entry.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tag {
    pub connection: String,
    pub kind: String,
    pub session: u64,
}

impl Tag {
    pub fn new(connection: &str, driver_name: &str) -> Self {
        Self {
            connection: connection.to_string(),
            kind: driver_name.to_ascii_uppercase(),
            session: 0,
        }
    }
}

/// Runs `fut`, which sends `sql`, and records it with its time and error.
pub(crate) async fn logged<T>(
    tag: &Tag,
    sql: &str,
    fut: impl Future<Output = DbResult<T>>,
) -> DbResult<T> {
    let time = chrono::Local::now();
    let started = Instant::now();
    let result = fut.await;
    record(LogEntry {
        time,
        connection: tag.connection.clone(),
        session: tag.session,
        kind: tag.kind.clone(),
        elapsed: started.elapsed(),
        sql: sql.to_string(),
        error: result.as_ref().err().map(|e| e.to_string()),
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sql: &str, error: Option<&str>) -> LogEntry {
        LogEntry {
            time: chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 10, 10, 23, 49, 1)
                .unwrap(),
            connection: "sakila 本地".into(),
            session: 42,
            kind: "MYSQL".into(),
            elapsed: Duration::from_millis(2),
            sql: sql.into(),
            error: error.map(String::from),
        }
    }

    #[test]
    fn renders_navicat_style_records() {
        let ok = entry("SELECT * FROM actor", None);
        assert_eq!(
            ok.render(Header::ALL),
            "[2026-10-10 23:49:01.000][sakila 本地][42][MYSQL][0.002s]\nSELECT * FROM actor\n\n"
        );
        let failed = entry("SELECT * FROM nope", Some("1146 - Table doesn't exist"));
        let text = failed.render(Header {
            date: false,
            session: false,
            ..Header::ALL
        });
        assert_eq!(
            text,
            "[23:49:01.000][sakila 本地][MYSQL][0.002s]\nSELECT * FROM nope\n\n\
             [23:49:01.000][sakila 本地][MYSQL][0.002s]\n1146 - Table doesn't exist\n\n"
        );
    }

    #[test]
    fn since_returns_what_came_after() {
        let start = revision();
        record(entry("SELECT 1", None));
        record(entry("SELECT 2", None));
        let (got, next) = since(start);
        let sqls: Vec<_> = got.iter().map(|e| e.sql.as_str()).collect();
        // Other tests may record in parallel; ours are there in order
        let a = sqls.iter().position(|s| *s == "SELECT 1").unwrap();
        assert!(sqls[a..].contains(&"SELECT 2"));
        assert!(next >= start + 2);
        assert!(since(next).0.is_empty() || revision() > next);
    }
}
