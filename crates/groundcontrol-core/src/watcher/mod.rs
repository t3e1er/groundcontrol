//! File system watcher: debounced change detection for markdown files.
//!
//! Uses [`notify`] to watch a corpus directory recursively and emits
//! classified [`FileEvent`]s through a tokio mpsc channel.

use std::path::{Path, PathBuf};

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use groundcontrol_common::{Error, Result};

// ─── Public Types ────────────────────────────────────────────────────────────

/// A file system event classified for the indexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    /// A new indexable file was created.
    Created(PathBuf),
    /// An existing indexable file was modified.
    Modified(PathBuf),
    /// An indexable file was deleted.
    Deleted(PathBuf),
    /// An indexable file was renamed (from, to).
    Renamed {
        /// Original path before the rename.
        from: PathBuf,
        /// New path after the rename.
        to: PathBuf,
    },
}

impl FileEvent {
    /// Get the primary affected file path.
    pub fn path(&self) -> &Path {
        match self {
            FileEvent::Created(p) | FileEvent::Modified(p) | FileEvent::Deleted(p) => p,
            FileEvent::Renamed { to, .. } => to,
        }
    }
}

/// Watches a directory for indexable documentation and code file changes.
///
/// Events are delivered through an internal tokio mpsc channel. The watcher
/// runs in the background on an OS thread managed by `notify`; dropping the
/// struct stops the watcher.
pub struct CorpusWatcher {
    /// Channel receiver for file events.
    receiver: mpsc::Receiver<FileEvent>,
    /// Handle to the watcher (kept alive to prevent drop).
    _watcher: RecommendedWatcher,
}

// ─── Implementation ──────────────────────────────────────────────────────────

impl CorpusWatcher {
    /// Start watching a directory for indexable markdown and code file changes.
    pub fn start(watch_path: &Path) -> Result<Self> {
        Self::start_with_matcher(watch_path, None)
    }

    /// Start watching a directory with an optional ExcludeMatcher for gitignore-style exclusions.
    pub fn start_with_matcher(
        watch_path: &Path,
        matcher: Option<std::sync::Arc<crate::index::exclude::ExcludeMatcher>>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<FileEvent>(256);

        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if let Some(file_event) =
                        classify_event_with_matcher(&event, matcher.as_deref())
                    {
                        // Best-effort send; if the receiver is gone we silently drop.
                        let _ = tx.blocking_send(file_event);
                    }
                }
            })
            .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;

        watcher
            .watch(watch_path, RecursiveMode::Recursive)
            .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;

        Ok(Self { receiver: rx, _watcher: watcher })
    }

    /// Receive the next file event (async). Returns `None` if the watcher stopped.
    pub async fn recv(&mut self) -> Option<FileEvent> {
        self.receiver.recv().await
    }

    /// Receive a debounced batch of file events.
    ///
    /// When an initial event is received, waits up to `debounce_duration`
    /// for subsequent rapid events to coalesce into a single deduplicated batch.
    pub async fn recv_debounced(
        &mut self,
        debounce_duration: std::time::Duration,
    ) -> Option<Vec<FileEvent>> {
        let first_event = self.receiver.recv().await?;
        let mut batch = vec![first_event];

        loop {
            tokio::select! {
                biased;
                Some(next_event) = self.receiver.recv() => {
                    batch.push(next_event);
                }
                _ = tokio::time::sleep(debounce_duration) => {
                    break;
                }
            }
        }

        Some(batch)
    }

    /// Non-blocking attempt to receive an event.
    ///
    /// Returns `None` if no event is currently available or the channel closed.
    pub fn try_recv(&mut self) -> Option<FileEvent> {
        self.receiver.try_recv().ok()
    }
}

/// Spawn a background debounced file watcher task for a corpus root.
///
/// Whenever indexable files (.md or supported source code) are created, modified,
/// or deleted, events are debounced and incrementally synced via
/// [`crate::corpus_manager::CorpusManager::sync_delta_paths`].
pub fn spawn_corpus_watcher(
    corpus_name: String,
    root_path: PathBuf,
    manager: std::sync::Arc<tokio::sync::RwLock<crate::corpus_manager::CorpusManager>>,
    debounce_duration: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let matcher = {
            let mgr = manager.read().await;
            mgr.get_engine(&corpus_name).ok().map(|e| std::sync::Arc::clone(e.exclude_matcher()))
        };
        let mut watcher = match CorpusWatcher::start_with_matcher(&root_path, matcher) {
            Ok(w) => {
                tracing::info!(corpus = %corpus_name, path = %root_path.display(), "started continuous file watcher");
                w
            }
            Err(e) => {
                tracing::error!(corpus = %corpus_name, path = %root_path.display(), error = %e, "failed to start file watcher");
                return;
            }
        };

        while let Some(events) = watcher.recv_debounced(debounce_duration).await {
            let mut paths: Vec<PathBuf> = Vec::new();
            for event in events {
                match event {
                    FileEvent::Created(p) | FileEvent::Modified(p) | FileEvent::Deleted(p) => {
                        paths.push(p);
                    }
                    FileEvent::Renamed { from, to } => {
                        paths.push(from);
                        paths.push(to);
                    }
                }
            }
            paths.sort();
            paths.dedup();

            if paths.is_empty() {
                continue;
            }

            tracing::info!(
                corpus = %corpus_name,
                files_count = paths.len(),
                "debounced file change event, incrementally syncing files"
            );

            let mut mgr = manager.write().await;
            match mgr.sync_delta_paths(Some(&corpus_name), &paths) {
                Ok(result) => {
                    tracing::info!(
                        corpus = %corpus_name,
                        new = result.new_files.len(),
                        modified = result.modified_files.len(),
                        deleted = result.deleted_files.len(),
                        "incremental sync complete"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        corpus = %corpus_name,
                        error = %e,
                        "incremental sync failed"
                    );
                }
            }
        }

        tracing::info!(corpus = %corpus_name, "file watcher stopped");
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Returns `true` if the path has a `.md` extension.
fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("md")
}

/// Returns `true` if the path is an indexable documentation or code file and not in an ignored dir.
pub fn is_indexable(path: &Path) -> bool {
    is_indexable_with_matcher(path, None)
}

/// Returns `true` if the path is an indexable documentation or code file and not excluded by matcher.
pub fn is_indexable_with_matcher(
    path: &Path,
    matcher: Option<&crate::index::exclude::ExcludeMatcher>,
) -> bool {
    if let Some(m) = matcher {
        if m.is_excluded(path, path.is_dir()) {
            return false;
        }
    } else {
        for component in path.components() {
            if let std::path::Component::Normal(c) = component {
                let s = c.to_string_lossy();
                if s == ".git"
                    || s == "target"
                    || s == "node_modules"
                    || s == ".index"
                    || s == ".fastembed_cache"
                    || (s.starts_with('.') && s != "." && s != "..")
                {
                    return false;
                }
            }
        }
    }

    if is_markdown(path) {
        return true;
    }
    crate::parser::code::is_code_file(path)
}

/// Classify a raw notify [`Event`] into an optional [`FileEvent`].
///
/// Only events affecting indexable files produce a result.
#[cfg(test)]
fn classify_event(event: &Event) -> Option<FileEvent> {
    classify_event_with_matcher(event, None)
}

/// Classify a raw notify [`Event`] with an optional ExcludeMatcher.
fn classify_event_with_matcher(
    event: &Event,
    matcher: Option<&crate::index::exclude::ExcludeMatcher>,
) -> Option<FileEvent> {
    let indexable_paths: Vec<&PathBuf> =
        event.paths.iter().filter(|p| is_indexable_with_matcher(p, matcher)).collect();

    if indexable_paths.is_empty() {
        return None;
    }

    match event.kind {
        EventKind::Create(_) => Some(FileEvent::Created(indexable_paths[0].clone())),

        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Any) => {
            Some(FileEvent::Modified(indexable_paths[0].clone()))
        }

        EventKind::Remove(_) => Some(FileEvent::Deleted(indexable_paths[0].clone())),

        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if indexable_paths.len() >= 2 => {
            Some(FileEvent::Renamed {
                from: indexable_paths[0].clone(),
                to: indexable_paths[1].clone(),
            })
        }

        _ => None,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, RemoveKind};

    /// Helper to build a notify Event with given kind and paths.
    fn make_event(kind: EventKind, paths: Vec<PathBuf>) -> Event {
        Event { kind, paths, attrs: Default::default() }
    }

    #[test]
    fn test_classify_create_md() {
        let event =
            make_event(EventKind::Create(CreateKind::File), vec![PathBuf::from("/docs/note.md")]);
        let result = classify_event(&event);
        assert!(
            matches!(result, Some(FileEvent::Created(p)) if p == std::path::Path::new("/docs/note.md"))
        );
    }

    #[test]
    fn test_classify_create_code() {
        let event =
            make_event(EventKind::Create(CreateKind::File), vec![PathBuf::from("/src/engine.rs")]);
        let result = classify_event(&event);
        assert!(
            matches!(result, Some(FileEvent::Created(p)) if p == std::path::Path::new("/src/engine.rs"))
        );
    }

    #[test]
    fn test_classify_ignores_git_dir() {
        let event = make_event(
            EventKind::Modify(ModifyKind::Any),
            vec![PathBuf::from(".git/hooks/pre-commit.rs")],
        );
        assert!(classify_event(&event).is_none());
    }

    #[test]
    fn test_classify_modify_md() {
        let event = make_event(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            vec![PathBuf::from("/docs/note.md")],
        );
        let result = classify_event(&event);
        assert!(
            matches!(result, Some(FileEvent::Modified(p)) if p == std::path::Path::new("/docs/note.md"))
        );
    }

    #[test]
    fn test_classify_delete_md() {
        let event =
            make_event(EventKind::Remove(RemoveKind::File), vec![PathBuf::from("/docs/note.md")]);
        let result = classify_event(&event);
        assert!(
            matches!(result, Some(FileEvent::Deleted(p)) if p == std::path::Path::new("/docs/note.md"))
        );
    }

    #[test]
    fn test_classify_rename_md() {
        let event = make_event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            vec![PathBuf::from("/docs/old.md"), PathBuf::from("/docs/new.md")],
        );
        let result = classify_event(&event);
        assert!(matches!(
            result,
            Some(FileEvent::Renamed { from, to })
                if from == std::path::Path::new("/docs/old.md") && to == std::path::Path::new("/docs/new.md")
        ));
    }

    #[test]
    fn test_classify_ignores_non_md() {
        let event =
            make_event(EventKind::Create(CreateKind::File), vec![PathBuf::from("/docs/image.png")]);
        assert!(classify_event(&event).is_none());
    }

    #[test]
    fn test_classify_modify_any() {
        let event =
            make_event(EventKind::Modify(ModifyKind::Any), vec![PathBuf::from("/docs/note.md")]);
        let result = classify_event(&event);
        assert!(matches!(result, Some(FileEvent::Modified(_))));
    }

    #[test]
    fn test_watcher_start_stop() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let watcher = CorpusWatcher::start(dir.path());
        assert!(watcher.is_ok(), "watcher should start without error");
        // Dropping the watcher stops it cleanly.
        drop(watcher);
    }
}
