//! margin-core: everything that is not UI.
//!
//! * `git`      – a thin, fast wrapper over the user's own `git` binary
//!                (hooks, signing, credential helpers, LFS, SSO all keep working).
//! * `diff`     – unified diff parser + per-hunk patch builder.
//! * `graph`    – lane layout for the commit graph.
//! * `notes`    – Markdown notes attached to repo / branch / commit / task.
//! * `detached` – notes whose commit/branch is gone; follows branch renames.
//! * `journal`  – undo log for automatic re-attachments.
//! * `sync`     – two-way notes mirror via a cloud-synced folder (1 repo free).
//! * `markdown` – the note content policy (no images, no HTML, safe links only).
//! * `tasks`    – task-tracker integration (Jira Cloud + Data Center).
//! * `github`   – pull-request status (merge readiness, CI, comments).
//! * `gitflow`  – native git-flow (feature/bugfix/release/hotfix/support).
//! * `config`   – config + secret lookup (env var / macOS Keychain).

pub mod config;
pub mod detached;
pub mod diff;
pub mod git;
pub mod github;
pub mod gitflow;
pub mod history;
pub mod journal;
mod http;
pub mod graph;
pub mod markdown;
pub mod notes;
pub mod platform;
pub mod sync;
pub mod tasks;
pub mod update;
mod time;

pub use time::{format_local, format_utc, local_offset, local_zone};
pub use time::now_unix as time_now;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[macro_export]
macro_rules! bail {
    ($($t:tt)*) => { return Err($crate::Error(format!($($t)*))) };
}
