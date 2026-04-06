use crate::policy_engine::GreenLevel;

pub const TARGETED_TESTS: GreenLevel = 1;
pub const PACKAGE: GreenLevel = 2;
pub const WORKSPACE: GreenLevel = 3;
pub const MERGE_READY: GreenLevel = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractOutcome {
    Met,
    NotMet {
        required: GreenLevel,
        observed: GreenLevel,
    },
}

#[derive(Debug, Clone)]
pub struct GreenContract {
    pub name: String,
    pub required_level: GreenLevel,
}

impl GreenContract {
    pub fn evaluate(&self, observed: GreenLevel) -> ContractOutcome {
        if observed >= self.required_level {
            ContractOutcome::Met
        } else {
            ContractOutcome::NotMet {
                required: self.required_level,
                observed,
            }
        }
    }
}

pub fn level_name(level: GreenLevel) -> &'static str {
    match level {
        1 => "targeted-tests",
        2 => "package",
        3 => "workspace",
        4 => "merge-ready",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_met_when_observed_equals_required() {
        let c = GreenContract {
            name: "ci".into(),
            required_level: PACKAGE,
        };
        assert_eq!(c.evaluate(PACKAGE), ContractOutcome::Met);
    }

    #[test]
    fn contract_met_when_observed_exceeds_required() {
        let c = GreenContract {
            name: "ci".into(),
            required_level: TARGETED_TESTS,
        };
        assert_eq!(c.evaluate(WORKSPACE), ContractOutcome::Met);
    }

    #[test]
    fn contract_not_met_when_observed_below_required() {
        let c = GreenContract {
            name: "merge".into(),
            required_level: MERGE_READY,
        };
        assert_eq!(
            c.evaluate(PACKAGE),
            ContractOutcome::NotMet {
                required: MERGE_READY,
                observed: PACKAGE
            },
        );
    }

    #[test]
    fn level_name_maps_correctly() {
        assert_eq!(level_name(1), "targeted-tests");
        assert_eq!(level_name(2), "package");
        assert_eq!(level_name(3), "workspace");
        assert_eq!(level_name(4), "merge-ready");
        assert_eq!(level_name(0), "unknown");
        assert_eq!(level_name(255), "unknown");
    }
}
