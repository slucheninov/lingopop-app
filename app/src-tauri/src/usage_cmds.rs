//! Usage tab IPC. Aggregation stays in `lingopop_core::usage_stats`; this module only
//! builds the labels and reads the store. `usage_view` takes the clock and the UTC offset
//! so a test never depends on the host time zone.
use crate::commands::ShellState;
use crate::state::ShellStores;
use lingopop_core::time::{Clock, SystemClock};
use lingopop_core::{
    chart_buckets, format_cost, format_tokens, i18n, totals, totals_by_provider,
    usage_display_name, Bucket, Language, Period, ProviderType, Totals, UsageRecord,
    TRACKED_PROVIDERS,
};
use serde::Serialize;
use tauri::State;

const COLOURS: [&str; 4] = ["red", "green", "blue", "purple"];
const ABSENT: &str = "—";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub provider: ProviderType,
    /// `usage_display_name`. Provider names are never localized.
    pub name: &'static str,
    pub colour: &'static str,
    pub input_label: String,
    pub output_label: String,
    pub calls: u64,
    pub cost_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageView {
    pub period: Period,
    pub totals: Totals,
    pub cost_label: String,
    pub tokens_label: String,
    pub in_out_label: String,
    pub calls_label: String,
    pub period_label: String,
    pub chart_title: String,
    pub buckets: Vec<Bucket>,
    pub rows: Vec<UsageRow>,
    /// True when the usage store failed to open. The command does not return an error.
    pub unavailable: bool,
}

/// Windows-only catalog key.
const USAGE_RESET_FAILED: &str = "Usage statistics couldn't be reset.";

fn text(language: Language, key: &str) -> String {
    i18n::t(language, key, &[])
}

fn cost_label(language: Language, cost: Option<f64>) -> String {
    match cost {
        Some(amount) => format_cost(amount),
        None => text(language, "~$?"),
    }
}

fn period_key(period: Period) -> &'static str {
    match period {
        Period::Day => "today",
        Period::Days7 => "last 7 days",
        Period::Days30 => "last 30 days",
    }
}

fn chart_key(period: Period) -> &'static str {
    match period {
        Period::Day => "Hourly tokens · today",
        Period::Days7 => "Daily tokens · 7 days",
        Period::Days30 => "Daily tokens · 30 days",
    }
}

/// Pure view. `unavailable` is false; [`usage_from_stores`] sets it when the store is missing.
pub fn usage_view(
    records: &[UsageRecord],
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
    language: Language,
) -> UsageView {
    let totals = totals(records, period, now_unix, utc_offset_seconds);
    let by_provider = totals_by_provider(records, period, now_unix, utc_offset_seconds);
    let input = format_tokens(totals.input_tokens);
    let output = format_tokens(totals.output_tokens);
    let calls_label = if totals.calls == 0 {
        text(language, "no activity")
    } else {
        let average =
            format_tokens((totals.input_tokens + totals.output_tokens) / totals.calls as i64);
        i18n::t(language, "avg %@/call", &[&average])
    };
    let rows = TRACKED_PROVIDERS
        .into_iter()
        .zip(COLOURS)
        .map(|(provider, colour)| {
            let name = usage_display_name(provider);
            match by_provider.get(provider.raw_value()) {
                Some(row) => UsageRow {
                    provider,
                    name,
                    colour,
                    input_label: format_tokens(row.input_tokens),
                    output_label: format_tokens(row.output_tokens),
                    calls: row.calls,
                    cost_label: cost_label(language, row.cost),
                },
                None => UsageRow {
                    provider,
                    name,
                    colour,
                    input_label: text(language, ABSENT),
                    output_label: text(language, ABSENT),
                    calls: 0,
                    cost_label: text(language, ABSENT),
                },
            }
        })
        .collect();
    UsageView {
        period,
        cost_label: cost_label(language, totals.cost),
        tokens_label: format_tokens(totals.input_tokens + totals.output_tokens),
        in_out_label: i18n::t(language, "in %@ · out %@", &[&input, &output]),
        calls_label,
        period_label: text(language, period_key(period)),
        chart_title: text(language, chart_key(period)),
        buckets: chart_buckets(records, period, now_unix, utc_offset_seconds, language),
        rows,
        totals,
        unavailable: false,
    }
}

/// Reads the store when it opened. A missing store yields an empty view with `unavailable`.
pub fn usage_from_stores(
    stores: &ShellStores,
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
    language: Language,
) -> UsageView {
    let available = stores.usage_available();
    let records = if available {
        stores.usage_records()
    } else {
        Vec::new()
    };
    let mut view = usage_view(&records, period, now_unix, utc_offset_seconds, language);
    view.unavailable = !available;
    view
}

fn local_offset_seconds() -> i32 {
    chrono::Local::now().offset().local_minus_utc()
}

#[tauri::command]
pub async fn usage_get(state: State<'_, ShellState>, period: Period) -> Result<UsageView, String> {
    Ok(usage_from_stores(
        &state.stores,
        period,
        SystemClock.now().unix_seconds(),
        local_offset_seconds(),
        state.ui.language(),
    ))
}

#[tauri::command]
pub async fn usage_reset(state: State<'_, ShellState>) -> Result<(), String> {
    state.stores.reset_usage().map_err(|_| {
        // Fixed text: the store error can carry a file path.
        log::warn!("usage reset failed");
        text(state.ui.language(), USAGE_RESET_FAILED)
    })
}

#[cfg(test)]
mod tests {
    use super::{usage_from_stores, usage_view};
    use lingopop_core::{
        format_tokens, i18n, AIOperation, Language, Period, ProviderType, Timestamp, UsageRecord,
    };

    /// 2026-11-01T10:30:00Z. Offset 0 keeps the window independent of the host zone.
    const NOW: f64 = 1_793_529_000.0;

    fn claude(input: i64, output: i64) -> UsageRecord {
        UsageRecord::new(
            ProviderType::Claude,
            "claude-haiku-5-5".into(),
            AIOperation::Translate,
            input,
            output,
            Timestamp::from_unix_seconds(NOW - 10.0),
        )
    }

    fn view(records: &[UsageRecord], period: Period, language: Language) -> super::UsageView {
        usage_view(records, period, NOW, 0, language)
    }

    #[test]
    fn usage_view_labels_match_mac() {
        let records = [claude(1000, 2000), claude(1000, 2000)];
        let view = view(&records, Period::Days30, Language::En);
        assert_eq!(view.cost_label, "$0.0022");
        assert_eq!(view.tokens_label, "6.0K");
        assert_eq!(view.in_out_label, "in 2.0K · out 4.0K");
        assert_eq!(view.calls_label, "avg 3.0K/call");
        assert_eq!(view.period_label, "last 30 days");
        assert_eq!(view.chart_title, "Daily tokens · 30 days");
        assert!(!view.unavailable);
        assert_eq!(view.rows[0].input_label, "2.0K");
        assert_eq!(view.rows[0].output_label, "4.0K");
        assert_eq!(view.rows[0].calls, 2);
        assert_eq!(view.rows[0].cost_label, "$0.0022");
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["costLabel"], "$0.0022");
        assert_eq!(json["tokensLabel"], "6.0K");
        assert_eq!(json["inOutLabel"], "in 2.0K · out 4.0K");
        assert_eq!(json["callsLabel"], "avg 3.0K/call");
        assert_eq!(json["period"], "days30");
    }

    #[test]
    fn usage_view_no_activity() {
        let view = view(&[], Period::Day, Language::En);
        assert_eq!(view.cost_label, "~$?");
        assert_eq!(view.calls_label, "no activity");
        assert_eq!(view.period_label, "today");
        assert_eq!(view.chart_title, "Hourly tokens · today");
        assert_eq!(view.totals.calls, 0);
        assert!(view.totals.cost.is_none());
        assert_eq!(view.rows.len(), 4);
        for row in &view.rows {
            assert_eq!(
                (
                    row.input_label.as_str(),
                    row.output_label.as_str(),
                    row.calls,
                    row.cost_label.as_str()
                ),
                ("—", "—", 0, "—")
            );
        }
    }

    #[test]
    fn usage_view_rows_always_four_in_order() {
        let records = [UsageRecord::new(
            ProviderType::Grok,
            "not-a-priced-model".into(),
            AIOperation::Translate,
            5,
            7,
            Timestamp::from_unix_seconds(NOW - 10.0),
        )];
        let view = view(&records, Period::Days7, Language::En);
        assert_eq!(
            view.rows.iter().map(|row| row.name).collect::<Vec<_>>(),
            ["Claude", "OpenAI", "Gemini", "Grok"]
        );
        assert_eq!(
            view.rows.iter().map(|row| row.colour).collect::<Vec<_>>(),
            ["red", "green", "blue", "purple"]
        );
        assert_eq!(
            view.rows.iter().map(|row| row.provider).collect::<Vec<_>>(),
            [
                ProviderType::Claude,
                ProviderType::OpenAI,
                ProviderType::Gemini,
                ProviderType::Grok
            ]
        );
        for row in &view.rows[..3] {
            assert_eq!(
                (
                    row.input_label.as_str(),
                    row.output_label.as_str(),
                    row.calls,
                    row.cost_label.as_str()
                ),
                ("—", "—", 0, "—")
            );
        }
        assert_eq!(view.rows[3].input_label, "5");
        assert_eq!(view.rows[3].output_label, "7");
        assert_eq!(view.rows[3].calls, 1);
        assert_eq!(view.rows[3].cost_label, "~$?");
        assert_eq!(view.period_label, "last 7 days");
        assert_eq!(view.chart_title, "Daily tokens · 7 days");
    }

    #[test]
    fn usage_view_labels_are_localized() {
        let records = [claude(1000, 2000), claude(1000, 2000)];
        let en = view(&records, Period::Days30, Language::En);
        let pl = view(&records, Period::Days30, Language::Pl);
        let input = format_tokens(2000);
        let output = format_tokens(4000);
        let average = format_tokens(3000);
        assert_ne!(pl.in_out_label, en.in_out_label);
        assert_ne!(pl.calls_label, en.calls_label);
        assert_ne!(pl.period_label, en.period_label);
        assert_ne!(pl.chart_title, en.chart_title);
        assert_eq!(
            pl.in_out_label,
            i18n::t(Language::Pl, "in %@ · out %@", &[&input, &output])
        );
        assert_eq!(
            pl.calls_label,
            i18n::t(Language::Pl, "avg %@/call", &[&average])
        );
        assert_eq!(pl.period_label, i18n::t(Language::Pl, "last 30 days", &[]));
        assert_eq!(
            pl.chart_title,
            i18n::t(Language::Pl, "Daily tokens · 30 days", &[])
        );
        assert_eq!(pl.rows[0].name, "Claude");
        assert_eq!(en.rows[0].name, pl.rows[0].name);
        assert_eq!(pl.cost_label, en.cost_label);
    }

    #[test]
    fn usage_unavailable_when_store_missing_and_reset_does_not_create_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("usage.json")).unwrap();
        let (_, stores, ui) = crate::task4_tests::rig(dir.path());
        assert_eq!(ui.language(), Language::Uk);
        let view = usage_from_stores(&stores, Period::Day, NOW, 0, ui.language());
        assert!(view.unavailable);
        assert_eq!(view.cost_label, i18n::t(Language::Uk, "~$?", &[]));
        assert_eq!(view.calls_label, i18n::t(Language::Uk, "no activity", &[]));
        assert_ne!(view.calls_label, "no activity");
        stores.reset_usage().unwrap();
        assert!(
            dir.path().join("usage.json").is_dir(),
            "reset must not replace a missing store with a file"
        );
    }
}
