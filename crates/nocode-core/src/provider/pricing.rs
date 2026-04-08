//! Model pricing table — maps model names to per-token USD costs.
//!
//! Prices are per 1M tokens. Updated 2026-04-08.

/// Per-token pricing for a model (USD per 1M tokens).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    /// USD per 1M input tokens.
    pub input_per_m: f64,
    /// USD per 1M output tokens.
    pub output_per_m: f64,
    /// USD per 1M cached input tokens (if prompt caching is used).
    pub cache_read_per_m: f64,
    /// USD per 1M cache creation tokens.
    pub cache_write_per_m: f64,
}

impl ModelPricing {
    const fn new(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Self {
        Self {
            input_per_m: input,
            output_per_m: output,
            cache_read_per_m: cache_read,
            cache_write_per_m: cache_write,
        }
    }

    /// Calculate cost in USD from token counts.
    pub fn calculate(&self, input: u64, output: u64, cache_read: u64, cache_write: u64) -> f64 {
        let m = 1_000_000.0;
        (input as f64 * self.input_per_m / m)
            + (output as f64 * self.output_per_m / m)
            + (cache_read as f64 * self.cache_read_per_m / m)
            + (cache_write as f64 * self.cache_write_per_m / m)
    }
}

/// Known model pricing entries.
const PRICING_TABLE: &[(&str, ModelPricing)] = &[
    // Claude models
    ("claude-opus-4", ModelPricing::new(15.0, 75.0, 1.5, 18.75)),
    ("claude-sonnet-4", ModelPricing::new(3.0, 15.0, 0.3, 3.75)),
    ("claude-haiku-3.5", ModelPricing::new(0.8, 4.0, 0.08, 1.0)),
    // OpenAI models
    ("gpt-4o", ModelPricing::new(2.5, 10.0, 1.25, 2.5)),
    ("gpt-4o-mini", ModelPricing::new(0.15, 0.6, 0.075, 0.15)),
    ("gpt-4.1", ModelPricing::new(2.0, 8.0, 0.5, 2.0)),
    ("gpt-4.1-mini", ModelPricing::new(0.4, 1.6, 0.1, 0.4)),
    ("gpt-4.1-nano", ModelPricing::new(0.1, 0.4, 0.025, 0.1)),
    // Gemini models
    ("gemini-2.5-pro", ModelPricing::new(1.25, 10.0, 0.315, 1.25)),
    (
        "gemini-2.5-flash",
        ModelPricing::new(0.15, 0.6, 0.0375, 0.15),
    ),
];

/// Default fallback pricing (Sonnet-class).
const DEFAULT_PRICING: ModelPricing = ModelPricing::new(3.0, 15.0, 0.3, 3.75);

/// Look up pricing for a model by name. Uses prefix matching.
/// Falls back to Sonnet-class defaults if no match found.
pub fn lookup_pricing(model: &str) -> ModelPricing {
    let lower = model.to_ascii_lowercase();
    for &(prefix, pricing) in PRICING_TABLE {
        if lower.contains(prefix) {
            return pricing;
        }
    }
    DEFAULT_PRICING
}

/// Calculate cost in USD for a given model and usage.
pub fn calculate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    lookup_pricing(model).calculate(
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_pricing() {
        let p = lookup_pricing("claude-opus-4-20250514");
        assert!((p.input_per_m - 15.0).abs() < f64::EPSILON);
        assert!((p.output_per_m - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sonnet_pricing() {
        let p = lookup_pricing("claude-sonnet-4-20250514");
        assert!((p.input_per_m - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gpt4o_pricing() {
        let p = lookup_pricing("gpt-4o-2025-01-01");
        assert!((p.input_per_m - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn gemini_pricing() {
        let p = lookup_pricing("gemini-2.5-pro-latest");
        assert!((p.input_per_m - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_model_gets_default() {
        let p = lookup_pricing("some-unknown-model");
        assert!((p.input_per_m - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn calculate_cost_basic() {
        // 1M input + 1M output on Sonnet = $3 + $15 = $18
        let cost = calculate_cost("claude-sonnet-4", 1_000_000, 1_000_000, 0, 0);
        assert!((cost - 18.0).abs() < 0.01);
    }

    #[test]
    fn calculate_cost_with_cache() {
        // 1M cache read on Sonnet = $0.30
        let cost = calculate_cost("claude-sonnet-4", 0, 0, 1_000_000, 0);
        assert!((cost - 0.3).abs() < 0.01);
    }

    #[test]
    fn calculate_cost_zero_tokens() {
        let cost = calculate_cost("claude-opus-4", 0, 0, 0, 0);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }
}
