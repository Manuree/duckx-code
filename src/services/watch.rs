use std::path::Path;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::UnboundedSender;

use crate::jobs::AppEvent;

/// Owns the OS watch so it stays alive until the workspace folder changes.
pub struct RootWatch {
    _inner: RecommendedWatcher,
}

/// Watches `root` recursively. Creates, removes, and renames become `FsChanged`.
/// A failure after the watch starts becomes `FsWatchFailed`. Access events are ignored.
/// The callback runs on the watcher's thread and only sends on `tx`.
pub fn watch_root(root: &Path, tx: UnboundedSender<AppEvent>) -> Result<RootWatch, ()> {
    let watched = root.to_path_buf();
    let report = watched.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            let event = match result {
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
                Ok(_) => AppEvent::FsChanged {
                    root: report.clone(),
                },
                Err(_) => AppEvent::FsWatchFailed {
                    root: report.clone(),
                },
            };
            let _ = tx.send(event);
        })
        .map_err(|_| ())?;
    watcher
        .watch(&watched, RecursiveMode::Recursive)
        .map_err(|_| ())?;
    Ok(RootWatch { _inner: watcher })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

    use super::*;

    fn recv_change(rx: &mut UnboundedReceiver<AppEvent>, root: &Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match rx.try_recv() {
                Ok(AppEvent::FsChanged { root: got }) if got == root => return true,
                Ok(AppEvent::FsWatchFailed { .. }) => return false,
                Ok(_) => {}
                Err(TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    #[test]
    fn creating_a_file_reports_the_watched_root() {
        let root = std::env::temp_dir().join(format!("duckx-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let watcher = watch_root(&root, tx).unwrap();
        let settle = Instant::now() + Duration::from_millis(150);
        while Instant::now() < settle {
            let _ = rx.try_recv();
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(root.join("new.txt"), b"x").unwrap();
        assert!(
            recv_change(&mut rx, &root),
            "watch did not report the new file"
        );
        drop(watcher);
        let _ = std::fs::remove_dir_all(&root);
    }
}
