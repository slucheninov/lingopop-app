//! Per-model token prices, copied from the macOS `Core/UsagePricing.swift` table.
//! Prices are USD per 1M tokens. Rows stay in source order.

/// `(model, per-1M input USD, per-1M output USD)` in macOS source order.
pub const PRICING: &[(&str, f64, f64)] = &[
    ("claude-haiku-4-5", 0.80, 4.00),
    ("claude-sonnet-4-6", 3.00, 15.00),
    ("claude-opus-4-7", 15.00, 75.00),
    ("gpt-5.4-nano", 0.15, 0.60),
    ("gpt-5.4-mini", 0.60, 2.40),
    ("gpt-5.4", 2.50, 10.00),
    ("gpt-5.5", 5.00, 20.00),
    ("gemini-2.5-flash-lite", 0.075, 0.30),
    ("gemini-2.5-flash", 0.15, 0.60),
    ("gemini-2.5-pro", 1.25, 5.00),
    ("claude-sonnet-5", 2.00, 10.00),
    ("claude-opus-5", 5.00, 25.00),
    ("claude-opus-4-8", 5.00, 25.00),
    ("claude-fable-5", 10.00, 50.00),
    ("gpt-5.6-luna", 1.00, 6.00),
    ("gpt-5.6-terra", 2.50, 15.00),
    ("gpt-5.6-sol", 5.00, 30.00),
    ("gpt-5.5-pro", 30.00, 180.00),
    ("gpt-4.1", 2.00, 8.00),
    ("gpt-4.1-mini", 0.40, 1.60),
    ("gemini-3.6-flash", 1.50, 7.50),
    ("gemini-3.5-flash", 1.50, 9.00),
    ("gemini-3.5-flash-lite", 0.30, 2.50),
    ("gemini-3.1-pro-preview", 2.00, 12.00),
    ("gemini-3.1-flash-lite", 0.25, 1.50),
    ("claude-fable-5-1", 10.00, 50.00),
    ("claude-opus-5-5", 4.00, 20.00),
    ("claude-sonnet-5-5", 2.00, 10.00),
    ("claude-haiku-5-5", 0.10, 0.50),
    ("gpt-6-astra", 10.00, 50.00),
    ("gpt-6.1-sol", 2.00, 10.00),
    ("gpt-6-luna", 0.10, 0.50),
    ("gemini-4-argon", 2.00, 10.00),
    ("gemini-3.8-flash", 0.75, 3.75),
    ("grok-4.7", 2.00, 6.00),
    ("grok-4.6", 2.00, 6.00),
    ("grok-4.5", 2.00, 6.00),
    ("grok-4.3", 1.25, 2.50),
    ("grok-4.20-0309-reasoning", 1.25, 2.50),
    ("grok-4.20-0309-non-reasoning", 1.25, 2.50),
    ("grok-build-0.1", 1.00, 2.00),
];

/// Input and output price per 1M tokens when `model` matches a table row exactly.
pub fn price(model: &str) -> Option<(f64, f64)> {
    PRICING
        .iter()
        .find(|(name, _, _)| *name == model)
        .map(|(_, input, output)| (*input, *output))
}

/// `(input_tokens * input_price + output_tokens * output_price) / 1e6`.
/// `None` when `model` is not in [`PRICING`].
pub fn estimated_cost(model: &str, input_tokens: i64, output_tokens: i64) -> Option<f64> {
    let (input_price, output_price) = price(model)?;
    Some((input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::{estimated_cost, price, PRICING};

    /// The macOS `UsagePricing.table` rows, in source order.
    const MAC_ROWS: &[(&str, f64, f64)] = &[
        ("claude-haiku-4-5", 0.80, 4.00),
        ("claude-sonnet-4-6", 3.00, 15.00),
        ("claude-opus-4-7", 15.00, 75.00),
        ("gpt-5.4-nano", 0.15, 0.60),
        ("gpt-5.4-mini", 0.60, 2.40),
        ("gpt-5.4", 2.50, 10.00),
        ("gpt-5.5", 5.00, 20.00),
        ("gemini-2.5-flash-lite", 0.075, 0.30),
        ("gemini-2.5-flash", 0.15, 0.60),
        ("gemini-2.5-pro", 1.25, 5.00),
        ("claude-sonnet-5", 2.00, 10.00),
        ("claude-opus-5", 5.00, 25.00),
        ("claude-opus-4-8", 5.00, 25.00),
        ("claude-fable-5", 10.00, 50.00),
        ("gpt-5.6-luna", 1.00, 6.00),
        ("gpt-5.6-terra", 2.50, 15.00),
        ("gpt-5.6-sol", 5.00, 30.00),
        ("gpt-5.5-pro", 30.00, 180.00),
        ("gpt-4.1", 2.00, 8.00),
        ("gpt-4.1-mini", 0.40, 1.60),
        ("gemini-3.6-flash", 1.50, 7.50),
        ("gemini-3.5-flash", 1.50, 9.00),
        ("gemini-3.5-flash-lite", 0.30, 2.50),
        ("gemini-3.1-pro-preview", 2.00, 12.00),
        ("gemini-3.1-flash-lite", 0.25, 1.50),
        ("claude-fable-5-1", 10.00, 50.00),
        ("claude-opus-5-5", 4.00, 20.00),
        ("claude-sonnet-5-5", 2.00, 10.00),
        ("claude-haiku-5-5", 0.10, 0.50),
        ("gpt-6-astra", 10.00, 50.00),
        ("gpt-6.1-sol", 2.00, 10.00),
        ("gpt-6-luna", 0.10, 0.50),
        ("gemini-4-argon", 2.00, 10.00),
        ("gemini-3.8-flash", 0.75, 3.75),
        ("grok-4.7", 2.00, 6.00),
        ("grok-4.6", 2.00, 6.00),
        ("grok-4.5", 2.00, 6.00),
        ("grok-4.3", 1.25, 2.50),
        ("grok-4.20-0309-reasoning", 1.25, 2.50),
        ("grok-4.20-0309-non-reasoning", 1.25, 2.50),
        ("grok-build-0.1", 1.00, 2.00),
    ];

    #[test]
    fn pricing_table_has_every_mac_row() {
        assert_eq!(MAC_ROWS.len(), 41);
        assert_eq!(PRICING, MAC_ROWS);
        assert_eq!(price("claude-haiku-5-5"), Some((0.10, 0.50)));
        assert_eq!(price("grok-build-0.1"), Some((1.00, 2.00)));
        assert_eq!(price("claude-haiku-5-5-extra"), None);
    }

    #[test]
    fn estimated_cost_formula() {
        let cost = estimated_cost("claude-haiku-5-5", 1000, 2000).unwrap();
        assert!(
            (cost - 0.0011).abs() < 1e-12,
            "cost {cost} is not 0.0011 within 1e-12"
        );
        assert_eq!(estimated_cost("unknown-model", 1000, 2000), None);
    }
}
