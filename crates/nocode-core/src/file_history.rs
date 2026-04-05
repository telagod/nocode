#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryConfig {
    pub enabled: bool,
}

impl FileHistoryConfig {
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }

    pub const fn disabled() -> Self {
        Self { enabled: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryState {
    pub config: FileHistoryConfig,
    pub requested_snapshots: u32,
    pub committed_snapshots: u32,
}

impl FileHistoryState {
    pub fn new(config: FileHistoryConfig) -> Self {
        Self {
            config,
            requested_snapshots: 0,
            committed_snapshots: 0,
        }
    }

    pub fn request_snapshot(&mut self) -> FileHistoryPlan {
        if !self.config.enabled {
            return FileHistoryPlan {
                snapshot_requested: false,
                total_requests: self.requested_snapshots,
                total_committed: self.committed_snapshots,
            };
        }

        self.requested_snapshots += 1;
        FileHistoryPlan {
            snapshot_requested: true,
            total_requests: self.requested_snapshots,
            total_committed: self.committed_snapshots,
        }
    }

    pub fn commit_snapshot(&mut self) -> FileHistoryPlan {
        if !self.config.enabled {
            return FileHistoryPlan {
                snapshot_requested: false,
                total_requests: self.requested_snapshots,
                total_committed: self.committed_snapshots,
            };
        }
        self.committed_snapshots += 1;
        FileHistoryPlan {
            snapshot_requested: true,
            total_requests: self.requested_snapshots,
            total_committed: self.committed_snapshots,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryPlan {
    pub snapshot_requested: bool,
    pub total_requests: u32,
    pub total_committed: u32,
}

#[cfg(test)]
mod tests {
    use super::{FileHistoryConfig, FileHistoryState};

    #[test]
    fn disabled_config_never_requests() {
        let mut state = FileHistoryState::new(FileHistoryConfig::disabled());
        let plan = state.request_snapshot();
        assert!(!plan.snapshot_requested);
        assert_eq!(plan.total_requests, 0);
        assert_eq!(plan.total_committed, 0);
        let commit = state.commit_snapshot();
        assert!(!commit.snapshot_requested);
        assert_eq!(state.committed_snapshots, 0);
    }

    #[test]
    fn enabled_config_tracks_requests_and_commits() {
        let mut state = FileHistoryState::new(FileHistoryConfig::enabled());
        let plan1 = state.request_snapshot();
        assert!(plan1.snapshot_requested);
        assert_eq!(plan1.total_requests, 1);
        assert_eq!(plan1.total_committed, 0);
        let commit = state.commit_snapshot();
        assert_eq!(state.committed_snapshots, 1);
        assert_eq!(commit.total_committed, 1);
        let plan2 = state.request_snapshot();
        assert_eq!(plan2.total_requests, 2);
        assert_eq!(plan2.total_committed, 1);
    }
}
