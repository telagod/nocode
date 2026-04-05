/// Model pricing lookup and cost estimation.

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub model_pattern: String,
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_write_per_million: f64,
    pub cache_read_per_million: f64,
}

#[derive(Debug, Clone, Default)]
pub struct CostEstimate {
    pub input_cost: f64,
    pub output_cost: f64,
    pub cache_write_cost: f64,
    pub cache_read_cost: f64,
    pub total: f64,
}

/// Built-in pricing table. Matches model names containing the pattern.
const PRICING_TABLE: &[(&str, f64, f64, f64, f64)] = &[
    ("haiku",  1.0,  5.0,  1.25, 0.1),
    ("sonnet", 3.0,  15.0, 3.75, 0.3),
    ("opus",   15.0, 75.0, 18.75, 1.5),
];

/// Look up pricing for a model by substring match against known patterns.
pub fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    let lower = model.to_lowercase();
    PRICING_TABLE.iter().find_map(|(pattern, inp, out, cw, cr)| {
        if lower.contains(pattern) {
            Some(ModelPricing {
                model_pattern: (*pattern).to_string(),
                input_per_million: *inp,
                output_per_million: *out,
                cache_write_per_million: *cw,
                cache_read_per_million: *cr,
            })
        } else {
            None
        }
    })
}

/// Estimate cost for a given token usage.
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_write: u64,
    cache_read: u64,
) -> CostEstimate {
    let pricing = match pricing_for_model(model) {
        Some(p) => p,
        None => return CostEstimate::default(),
    };

    let input_cost = (input_tokens as f64) * pricing.input_per_million / 1_000_000.0;
    let output_cost = (output_tokens as f64) * pricing.output_per_million / 1_000_000.0;
    let cache_write_cost = (cache_write as f64) * pricing.cache_write_per_million / 1_000_000.0;
    let cache_read_cost = (cache_read as f64) * pricing.cache_read_per_million / 1_000_000.0;
    let total = input_cost + output_cost + cache_write_cost + cache_read_cost;

    CostEstimate {
        input_cost,
        output_cost,
        cache_write_cost,
        cache_read_cost,
        total,
    }
}

/// Format a dollar amount for display, e.g. "$0.0042".
pub fn format_usd(amount: f64) -> String {
    if amount == 0.0 {
        return "$0.00".to_string();
    }
    // Show enough decimal places to capture small costs
    if amount < 0.01 {
        format!("${amount:.4}")
    } else {
        format!("${amount:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pricing_for_known_models() {
        let p = pricing_for_model("claude-haiku-4-5").unwrap();
        assert_eq!(p.model_pattern, "haiku");
        assert!((p.input_per_million - 1.0).abs() < f64::EPSILON);

        let p = pricing_for_model("claude-sonnet-4-6").unwrap();
        assert_eq!(p.model_pattern, "sonnet");

        let p = pricing_for_model("claude-opus-4-6").unwrap();
        assert_eq!(p.model_pattern, "opus");
        assert!((p.output_per_million - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pricing_for_unknown_model() {
        assert!(pricing_for_model("gpt-4o").is_none());
    }

    #[test]
    fn test_pricing_case_insensitive() {
        assert!(pricing_for_model("Claude-OPUS-4").is_some());
        assert!(pricing_for_model("HAIKU").is_some());
    }

    #[test]
    fn test_estimate_cost_opus() {
        let est = estimate_cost("opus", 1_000_000, 1_000_000, 0, 0);
        assert!((est.input_cost - 15.0).abs() < f64::EPSILON);
        assert!((est.output_cost - 75.0).abs() < f64::EPSILON);
        assert!((est.total - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_estimate_cost_with_cache() {
        let est = estimate_cost("sonnet", 500_000, 100_000, 200_000, 300_000);
        let expected_input = 0.5 * 3.0;   // 1.5
        let expected_output = 0.1 * 15.0;  // 1.5
        let expected_cw = 0.2 * 3.75;     // 0.75
        let expected_cr = 0.3 * 0.3;      // 0.09
        assert!((est.total - (expected_input + expected_output + expected_cw + expected_cr)).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_cost_unknown_model() {
        let est = estimate_cost("unknown", 1000, 1000, 0, 0);
        assert!((est.total - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_format_usd() {
        assert_eq!(format_usd(0.0), "$0.00");
        assert_eq!(format_usd(1.5), "$1.50");
        assert_eq!(format_usd(0.0042), "$0.0042");
        assert_eq!(format_usd(0.1), "$0.10");
    }
}
