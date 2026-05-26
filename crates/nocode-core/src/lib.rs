pub mod agent;
pub mod auth;
pub mod bridge;
pub mod config;
pub mod ide_server;
pub mod mcp;
pub mod message;
pub mod prompt;
pub mod provider;
pub mod query;
pub mod recovery;
pub mod session;
pub mod skill;
pub mod storage;
pub mod telemetry;
pub mod tool;
pub mod update_checker;
pub mod ws_bridge;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, OnceLock};

    /// Process-wide guard for tests that mutate `HOME`, `cwd`, or any other
    /// process-global state. All such tests across the crate must `lock()`
    /// this before mutating, so they serialize and never clobber each other.
    pub(crate) fn env_mutex() -> &'static Mutex<()> {
        static M: OnceLock<Mutex<()>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(()))
    }
}
