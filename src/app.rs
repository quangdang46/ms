use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cli::OutputFormat;
use crate::config::Config;
use crate::error::{MsError, Result};
use crate::search::{CacheLayer, SearchIndex};
use crate::storage::{Database, GitArchive};

#[derive(Clone)]
pub struct AppContext {
    pub ms_root: PathBuf,
    pub config_path: PathBuf,
    pub config: Config,
    pub db: Arc<Database>,
    pub git: Arc<GitArchive>,
    pub search: Arc<SearchIndex>,
    pub cache: Arc<CacheLayer>,
    /// Deprecated: use output_format instead
    pub robot_mode: bool,
    pub output_format: OutputFormat,
    pub quiet: bool,
    pub verbosity: u8,
}

/// Inode-level identity of a single filesystem path (device + inode on Unix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InodeId {
    dev: u64,
    ino: u64,
}

/// Cheap identity of the on-disk backing store.
///
/// Combines the SQLite db file and the search-index directory — used by the
/// long-running MCP server to detect that the state directory was
/// rebuilt/replaced underneath it and reopen before serving (issue #135).
///
/// Only inode identity is used (never mtime/len), so ordinary writes never
/// change the fingerprint: SQLite updates the main db file in place, and the
/// index directory keeps its inode as segment files come and go. The
/// fingerprint changes only when the files are *replaced* — a fresh state dir
/// swapped in for the old one — which is exactly the rebuild that otherwise
/// strands a running server following the renamed (orphaned) inodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoreIdentity {
    db: Option<InodeId>,
    index: Option<InodeId>,
}

impl StoreIdentity {
    /// Whether the SQLite database is currently present on disk. Used to avoid
    /// reopening onto a half-built store mid-rebuild (e.g. the directory has
    /// been swapped in but the fresh db file has not been created yet).
    #[must_use]
    pub const fn db_present(&self) -> bool {
        self.db.is_some()
    }
}

#[cfg(unix)]
fn inode_of(meta: &std::fs::Metadata) -> InodeId {
    use std::os::unix::fs::MetadataExt;
    InodeId {
        dev: meta.dev(),
        ino: meta.ino(),
    }
}

#[cfg(not(unix))]
fn inode_of(meta: &std::fs::Metadata) -> InodeId {
    // No portable stable inode off-Unix. Approximate a regular file's identity
    // from (len, mtime-nanos); directories get a constant so the index dir does
    // not churn as segment files are written. The rename-under-open-handles bug
    // this guards against is POSIX-specific, so this fallback is best-effort.
    if meta.is_dir() {
        return InodeId { dev: 0, ino: 0 };
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0u64, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    InodeId {
        dev: meta.len(),
        ino: mtime,
    }
}

fn path_inode(path: &Path) -> Option<InodeId> {
    std::fs::metadata(path).ok().map(|m| inode_of(&m))
}

impl AppContext {
    pub fn from_cli(cli: &crate::cli::Cli) -> Result<Self> {
        let ms_root = Self::find_ms_root()?;
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(|| default_config_path(&ms_root));
        let mut config = Config::load(cli.config.as_deref(), &ms_root)?;
        apply_cli_output_overrides(&mut config, cli);

        Ok(Self {
            ms_root: ms_root.clone(),
            config_path,
            config,
            db: Arc::new(Database::open(ms_root.join("ms.db"))?),
            git: Arc::new(GitArchive::open(ms_root.join("archive"))?),
            cache: Arc::new(CacheLayer::new()),
            search: Arc::new({
                let index_path = ms_root.join("index");
                // Try writable first; if the write lock is busy (another process),
                // fall back to read-only mode so concurrent MCP servers and CLI
                // commands can coexist without "LockBusy" errors.
                SearchIndex::open(&index_path)
                    .or_else(|_| SearchIndex::open_readonly(&index_path))?
            }),
            robot_mode: cli.robot,
            output_format: cli.output_format(),
            quiet: cli.quiet,
            verbosity: cli.verbose,
        })
    }

    /// Path of the SQLite database backing this context.
    fn db_path(&self) -> PathBuf {
        self.ms_root.join("ms.db")
    }

    /// Path of the search-index directory backing this context.
    fn index_path(&self) -> PathBuf {
        self.ms_root.join("index")
    }

    /// Cheap snapshot of the on-disk backing store's identity.
    ///
    /// The long-running MCP server records this at startup and re-checks it
    /// before serving each request; a change means the state directory was
    /// rebuilt/replaced and the server must [`reopen_stores`](Self::reopen_stores)
    /// so it stops following the stale (renamed) inodes (issue #135).
    #[must_use]
    pub fn store_identity(&self) -> StoreIdentity {
        StoreIdentity {
            db: path_inode(&self.db_path()),
            index: path_inode(&self.index_path()),
        }
    }

    /// Reopen the SQLite database, git archive, and search index from
    /// `ms_root`, replacing the handles this context currently holds.
    ///
    /// Called by the long-running MCP server after [`store_identity`](Self::store_identity)
    /// shows the on-disk state directory was rebuilt/replaced, so it stops
    /// serving reads and landing writes on the stale (renamed) inodes. Fresh
    /// handles are built into locals first and only swapped in once all three
    /// succeed, so a failure (e.g. a half-written rebuild) leaves the context
    /// on its current, consistent handles rather than half-reopened. The
    /// previous `Arc`s — and their open file descriptors / Tantivy writer lock
    /// on the now-renamed directory — are released once their last in-flight
    /// reference is dropped.
    ///
    /// Note: the in-memory search cache is intentionally retained across
    /// reopens; its generation can be bumped by callers if a full invalidation
    /// is required.
    pub fn reopen_stores(&mut self) -> Result<()> {
        let index_path = self.index_path();
        let db = Arc::new(Database::open(self.db_path())?);
        let git = Arc::new(GitArchive::open(self.ms_root.join("archive"))?);
        // Match `from_cli`: prefer a writable index, fall back to read-only if
        // the writer lock is held (e.g. by a concurrent rebuild still running).
        let search = Arc::new(
            SearchIndex::open(&index_path).or_else(|_| SearchIndex::open_readonly(&index_path))?,
        );
        self.db = db;
        self.git = git;
        self.search = search;
        Ok(())
    }

    /// Ensure the search index was opened for writing.
    ///
    /// [`AppContext::from_cli`] transparently falls back to a **read-only**
    /// search index when the writable open fails, so that read-only commands
    /// (`search`, `load`, `list`, …) keep working alongside a live
    /// `ms mcp serve` that holds the Tantivy writer lock. Commands that mutate
    /// the index (`index`, and the other re-indexing paths) must call this
    /// first: without it they would perform partial SQLite/Git writes and then
    /// abort mid-run with an opaque Tantivy "read-only mode" error — exactly the
    /// failure reported in issue #133.
    ///
    /// Performs no side effects; on failure returns a clear, actionable error
    /// naming the concrete cause (held writer lock vs. read-only filesystem)
    /// and the index that was selected.
    pub fn require_writable_search(&self) -> Result<()> {
        if self.search.is_readonly() {
            Err(MsError::SearchIndexReadOnly(
                self.readonly_search_diagnostic(),
            ))
        } else {
            Ok(())
        }
    }

    fn readonly_search_diagnostic(&self) -> String {
        let index_dir = self.ms_root.join("index");
        let writer_lock = index_dir.join(".tantivy-writer.lock");

        let cause = if dir_is_writable(&index_dir) {
            if writer_lock.exists() {
                format!(
                    "another `ms` process is holding the search-index writer lock at {}. \
                     A running `ms mcp serve` keeps this lock for its entire lifetime; \
                     stop it (or any other `ms` writing to this index) and retry. \
                     If you are certain no such process is running, the lock is stale \
                     and can be deleted.",
                    writer_lock.display()
                )
            } else {
                format!(
                    "the search index could not be opened for writing: {}",
                    index_dir.display()
                )
            }
        } else {
            format!(
                "the index directory is on a read-only filesystem or you lack \
                 permission to write to it: {}",
                index_dir.display()
            )
        };

        format!(
            "Cannot write to the search index: {cause}\n\
             This index was selected from {} (via $MS_ROOT, the nearest .ms directory \
             above the current working directory, or the global data dir). If that is \
             not the index you meant to write, run the command from the target project \
             or set MS_ROOT explicitly.",
            self.ms_root.display()
        )
    }

    fn find_ms_root() -> Result<PathBuf> {
        if let Ok(root) = std::env::var("MS_ROOT") {
            return Ok(PathBuf::from(root));
        }
        let cwd = std::env::current_dir()?;
        if let Some(found) = find_upwards(&cwd, ".ms")? {
            return Ok(found);
        }

        let data_dir = dirs::data_dir()
            .ok_or_else(|| MsError::MissingConfig("data directory not found".to_string()))?;
        Ok(data_dir.join("ms"))
    }
}

/// Best-effort writability probe: try to create (and immediately remove) a
/// uniquely named file inside `dir`, or its nearest existing ancestor. Used to
/// distinguish a genuinely read-only filesystem / permission problem from mere
/// writer-lock contention when reporting why the search index is read-only.
fn dir_is_writable(dir: &Path) -> bool {
    // Walk up to the nearest directory that actually exists — the target may be
    // absent if creation itself failed on a read-only mount.
    let mut probe_dir = dir;
    while !probe_dir.exists() {
        match probe_dir.parent() {
            Some(parent) => probe_dir = parent,
            None => return false,
        }
    }
    let probe = probe_dir.join(format!(".ms-writable-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn apply_cli_output_overrides(config: &mut Config, cli: &crate::cli::Cli) {
    if let Some(theme) = &cli.theme {
        config.output.theme = theme.clone();
    }

    config.output.plain = cli.force_plain();
    config.output.force_rich = cli.force_rich();
    config.output.no_unicode = cli.plain;
}

fn default_config_path(ms_root: &Path) -> PathBuf {
    if ms_root.ends_with(".ms") {
        ms_root.join("config.toml")
    } else {
        dirs::config_dir()
            .unwrap_or_else(|| ms_root.to_path_buf())
            .join("ms/config.toml")
    }
}

fn find_upwards(start: &Path, name: &str) -> Result<Option<PathBuf>> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join(name);
        if candidate.is_dir() {
            return Ok(Some(candidate));
        }
        current = dir.parent();
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::apply_cli_output_overrides;
    use crate::cli::Cli;
    use crate::config::Config;

    #[test]
    fn cli_theme_overrides_output_config() {
        let cli = Cli::parse_from(["ms", "--theme", "minimal", "config"]);
        let mut config = Config::default();

        apply_cli_output_overrides(&mut config, &cli);

        assert_eq!(config.output.theme, "minimal");
    }

    #[test]
    fn cli_color_overrides_output_mode() {
        let cli = Cli::parse_from(["ms", "--color", "always", "config"]);
        let mut config = Config::default();

        apply_cli_output_overrides(&mut config, &cli);

        assert!(config.output.force_rich);
        assert!(!config.output.plain);
        assert!(!config.output.no_unicode);
    }

    #[test]
    fn cli_plain_overrides_output_mode() {
        let cli = Cli::parse_from(["ms", "--plain", "config"]);
        let mut config = Config::default();

        apply_cli_output_overrides(&mut config, &cli);

        assert!(config.output.plain);
        assert!(config.output.no_unicode);
        assert!(!config.output.force_rich);
    }
}
