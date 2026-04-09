//! Auto-dream trigger — automatically consolidates memories during idle periods.
//!
//! Checks if consolidation is needed at session end or after idle timeout,
//! and runs `DreamConsolidator` when thresholds are met.

use crate::storage::memory::{ConsolidationReport, DreamConsolidator};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Configuration for auto-dream triggers.
#[derive(Debug, Clone)]
pub struct DreamTriggerConfig {
    /// Enable auto-dream consolidation.
    pub enabled: bool,
    /// Minimum idle seconds before triggering consolidation.
    pub idle_threshold_secs: u64,
    /// Maximum age in days for project memories.
    pub project_max_age_days: u64,
    /// Minimum interval between consolidation runs (seconds).
    pub min_interval_secs: u64,
}

impl Default for DreamTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_threshold_secs: 300, // 5 minutes
            project_max_age_days: 30,
            min_interval_secs: 3600, // 1 hour
        }
    }
}

/// Tracks idle state and triggers consolidation when appropriate.
pub struct DreamTrigger {
    config: DreamTriggerConfig,
    memory_dir: String,
    last_activity: Instant,
    last_consolidation_epoch: AtomicU64,
    running: AtomicBool,
}

impl DreamTrigger {
    pub fn new(memory_dir: &str, config: DreamTriggerConfig) -> Self {
        Self {
            config,
            memory_dir: memory_dir.to_string(),
            last_activity: Instant::now(),
            last_consolidation_epoch: AtomicU64::new(0),
            running: AtomicBool::new(false),
        }
    }

    /// Record user activity (resets idle timer).
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Seconds since last activity.
    pub fn idle_secs(&self) -> u64 {
        self.last_activity.elapsed().as_secs()
    }

    /// Check if consolidation should run now.
    pub fn should_trigger(&self) -> bool {
        if !self.config.enabled {
            return false;
        }
        if self.running.load(Ordering::Relaxed) {
            return false;
        }
        // Check idle threshold
        if self.idle_secs() < self.config.idle_threshold_secs {
            return false;
        }
        // Check min interval
        let now = chrono::Utc::now().timestamp() as u64;
        let last = self.last_consolidation_epoch.load(Ordering::Relaxed);
        if last > 0 && now - last < self.config.min_interval_secs {
            return false;
        }
        // Check if consolidator thinks it's needed
        let consolidator =
            DreamConsolidator::new(&self.memory_dir, self.config.project_max_age_days);
        consolidator.should_consolidate()
    }

    /// Run consolidation if triggered. Returns report if run.
    pub fn maybe_consolidate(&self) -> Option<ConsolidationReport> {
        if !self.should_trigger() {
            return None;
        }
        self.running.store(true, Ordering::Relaxed);
        let consolidator =
            DreamConsolidator::new(&self.memory_dir, self.config.project_max_age_days);
        let result = consolidator.consolidate();
        self.running.store(false, Ordering::Relaxed);
        let now = chrono::Utc::now().timestamp() as u64;
        self.last_consolidation_epoch.store(now, Ordering::Relaxed);
        result.ok()
    }

    /// Force consolidation regardless of triggers.
    pub fn force_consolidate(&self) -> Result<ConsolidationReport, String> {
        self.running.store(true, Ordering::Relaxed);
        let consolidator =
            DreamConsolidator::new(&self.memory_dir, self.config.project_max_age_days);
        let result = consolidator.consolidate();
        self.running.store(false, Ordering::Relaxed);
        let now = chrono::Utc::now().timestamp() as u64;
        self.last_consolidation_epoch.store(now, Ordering::Relaxed);
        result
    }

    /// Called at session end — consolidate if needed.
    pub fn on_session_end(&self) -> Option<ConsolidationReport> {
        if !self.config.enabled {
            return None;
        }
        let consolidator =
            DreamConsolidator::new(&self.memory_dir, self.config.project_max_age_days);
        if consolidator.should_consolidate() {
            self.force_consolidate().ok()
        } else {
            None
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> DreamTriggerConfig {
        DreamTriggerConfig {
            enabled: true,
            idle_threshold_secs: 0, // immediate for testing
            project_max_age_days: 30,
            min_interval_secs: 0, // no cooldown for testing
        }
    }

    #[test]
    fn disabled_never_triggers() {
        let config = DreamTriggerConfig {
            enabled: false,
            ..Default::default()
        };
        let trigger = DreamTrigger::new("/tmp/nocode_dream_never", config);
        assert!(!trigger.should_trigger());
        assert!(trigger.maybe_consolidate().is_none());
    }

    #[test]
    fn default_config_values() {
        let config = DreamTriggerConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.idle_threshold_secs, 300);
        assert_eq!(config.project_max_age_days, 30);
        assert_eq!(config.min_interval_secs, 3600);
    }

    #[test]
    fn touch_resets_idle() {
        let mut trigger = DreamTrigger::new("/tmp/nocode_dream_touch", test_config());
        trigger.touch();
        assert!(trigger.idle_secs() < 2);
    }

    #[test]
    fn force_consolidate_empty_dir() {
        let tmp = format!("/tmp/nocode_dream_force_{}", std::process::id());
        let _ = std::fs::create_dir_all(&tmp);
        let trigger = DreamTrigger::new(&tmp, test_config());
        let report = trigger.force_consolidate().unwrap();
        assert_eq!(report.scanned, 0);
        assert_eq!(report.remaining, 0);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn on_session_end_disabled() {
        let config = DreamTriggerConfig {
            enabled: false,
            ..Default::default()
        };
        let trigger = DreamTrigger::new("/tmp/nocode_dream_end", config);
        assert!(trigger.on_session_end().is_none());
    }

    #[test]
    fn is_running_flag() {
        let trigger = DreamTrigger::new("/tmp/nocode_dream_run", test_config());
        assert!(!trigger.is_running());
    }

    #[test]
    fn is_enabled_flag() {
        let trigger = DreamTrigger::new("/tmp/nocode_dream_en", test_config());
        assert!(trigger.is_enabled());
    }
}
