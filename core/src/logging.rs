//! Rolling file log, installed alongside the existing stderr log.
//!
//! Diagnostics used to live only on stderr, which means they exist only while the app runs
//! under Xcode — a bug reproduced by a user is unrecoverable after the fact. The file layer
//! keeps the same events on disk, size-capped so it can never grow without bound.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

// Cap per file. One rotated backup is kept, so the logs occupy at most 2 × this on disk.
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

const LOG_FILE_NAME: &str = "bitrufus.log";

/// Absolute path of the log file: `~/Library/Logs/BitRufus/bitrufus.log`.
///
/// The app is sandboxed, so `$HOME` is its container and this actually resolves to
/// `~/Library/Containers/com.timrufus.BitRufus/Data/Library/Logs/BitRufus/`.
pub fn log_file_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    Some(
        base.home_dir()
            .join("Library")
            .join("Logs")
            .join("BitRufus")
            .join(LOG_FILE_NAME),
    )
}

/// Installs the tracing subscriber once so librqbit's diagnostics (tracker announces, peer
/// connections, disk-write errors, etc.) reach both stderr — visible in Console.app and the
/// Xcode console — and the rolling file above. Default verbosity keeps librqbit at info;
/// override with RUST_LOG, e.g. `RUST_LOG=librqbit=debug`. Safe to call repeatedly.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        use tracing_subscriber::EnvFilter;

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("librqbit=info,bitrufus_core=debug"));

        // A log file we can't open must not cost us the stderr log, so the failure is
        // carried past try_init and reported through the subscriber that did install.
        let path = log_file_path();
        let mut open_error = None;
        let file_layer = match path
            .as_ref()
            .map(|p| RotatingFile::open(p.clone(), MAX_FILE_BYTES))
        {
            Some(Ok(file)) => {
                let handle = LogHandle(Arc::new(Mutex::new(file)));
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || handle.clone()),
                )
            }
            Some(Err(err)) => {
                open_error = Some(err);
                None
            }
            None => None,
        };

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
            .with(file_layer)
            .try_init();

        match (open_error, path) {
            (Some(err), Some(p)) => {
                tracing::warn!(path = %p.display(), error = ?err, "file logging unavailable")
            }
            (_, Some(p)) => tracing::info!(path = %p.display(), "logging to file"),
            _ => tracing::warn!("file logging unavailable: no home directory"),
        }
    });
}

/// Append-only file that rotates to `<name>.1` before a write would push it past `max_bytes`.
struct RotatingFile {
    path: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
}

impl RotatingFile {
    fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        // Start from the existing length so a restart doesn't reset the cap.
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            written,
            max_bytes,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        // rename replaces any previous backup, so exactly one generation is kept.
        fs::rename(&self.path, backup_path(&self.path))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".1");
    path.with_file_name(name)
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Rotate before the write that would cross the cap rather than after, so no event
        // is split across two files. An oversized single event still lands whole.
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Cloneable writer handed to the fmt layer (`MakeWriter` is implemented for closures
/// returning an `io::Write`). Logging must never panic, so a poisoned lock silently drops
/// the event instead of propagating.
#[derive(Clone)]
struct LogHandle(Arc<Mutex<RotatingFile>>);

impl Write for LogHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut file) => file.write(buf),
            Err(_) => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.0.lock() {
            Ok(mut file) => file.flush(),
            Err(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_path_appends_suffix() {
        assert_eq!(
            backup_path(Path::new("/logs/bitrufus.log")),
            PathBuf::from("/logs/bitrufus.log.1")
        );
    }

    #[test]
    fn open_creates_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("app.log");
        RotatingFile::open(path.clone(), 1024).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn open_continues_counting_existing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        fs::write(&path, b"0123456789").unwrap();

        let mut file = RotatingFile::open(path.clone(), 12).unwrap();
        // 10 bytes already on disk + 5 more crosses the 12-byte cap → rotates.
        file.write_all(b"abcde").unwrap();

        assert_eq!(fs::read(backup_path(&path)).unwrap(), b"0123456789");
        assert_eq!(fs::read(&path).unwrap(), b"abcde");
    }

    #[test]
    fn rotates_once_cap_is_crossed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let mut file = RotatingFile::open(path.clone(), 10).unwrap();

        file.write_all(b"aaaaa").unwrap();
        file.write_all(b"bbbbb").unwrap();
        // Third write would reach 15 > 10, so the first two land in the backup.
        file.write_all(b"ccccc").unwrap();
        file.flush().unwrap();

        assert_eq!(fs::read(backup_path(&path)).unwrap(), b"aaaaabbbbb");
        assert_eq!(fs::read(&path).unwrap(), b"ccccc");
    }

    #[test]
    fn second_rotation_replaces_the_previous_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let mut file = RotatingFile::open(path.clone(), 4).unwrap();

        file.write_all(b"1111").unwrap();
        file.write_all(b"2222").unwrap();
        file.write_all(b"3333").unwrap();
        file.flush().unwrap();

        // Only two generations survive: the current file and one backup.
        assert_eq!(fs::read(backup_path(&path)).unwrap(), b"2222");
        assert_eq!(fs::read(&path).unwrap(), b"3333");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn oversized_event_is_written_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let mut file = RotatingFile::open(path.clone(), 4).unwrap();

        file.write_all(b"a_very_long_line").unwrap();
        file.flush().unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"a_very_long_line");
        assert!(!backup_path(&path).exists());
    }
}
