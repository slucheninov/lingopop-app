//! Period totals and chart buckets for the settings Usage tab.
//!
//! Day boundaries use the caller-supplied fixed UTC offset. A DST transition
//! inside the window is not modelled: every local day is exactly 86 400 seconds.

use std::collections::BTreeMap;

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeZone,
    Timelike, Weekday,
};
use serde::{Deserialize, Serialize};

use crate::provider::ProviderType;
use crate::records::UsageRecord;
use crate::ui_prefs::Language;
use crate::usage_pricing::estimated_cost;

const DAY_SECONDS: f64 = 86_400.0;

/// Providers drawn on the usage chart, in legend order.
pub const TRACKED_PROVIDERS: [ProviderType; 4] = [
    ProviderType::Claude,
    ProviderType::OpenAI,
    ProviderType::Gemini,
    ProviderType::Grok,
];

/// Short name used by the usage chart. Not the persisted raw value.
pub fn usage_display_name(provider: ProviderType) -> &'static str {
    match provider {
        ProviderType::Claude => "Claude",
        ProviderType::OpenAI => "OpenAI",
        ProviderType::Gemini => "Gemini",
        ProviderType::Grok => "Grok",
        ProviderType::GoogleFree => "Google Free",
        ProviderType::GoogleCloud => "Google Cloud",
        ProviderType::DeepL => "DeepL",
        ProviderType::MicrosoftTranslator => "Microsoft",
    }
}

/// Monday-first abbreviations. Rows are [`Language`] `En`, `Pl`, `Uk`, `De`.
const WEEKDAY_ABBREV: [[&str; 7]; 4] = [
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    ["pon", "wt", "śr", "czw", "pt", "sob", "ndz"],
    ["пн", "вт", "ср", "чт", "пт", "сб", "нд"],
    ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Period {
    Day,
    Days7,
    Days30,
}

impl Period {
    /// Local start of today, or that midnight minus 6 / 29 days, as Unix seconds.
    ///
    /// `utc_offset_seconds` is a fixed offset. The window does not follow DST.
    pub fn cutoff_unix(self, now_unix: f64, utc_offset_seconds: i32) -> f64 {
        let today = start_of_local_day(now_unix, utc_offset_seconds);
        let days_back = match self {
            Self::Day => 0.0,
            Self::Days7 => 6.0,
            Self::Days30 => 29.0,
        };
        today - days_back * DAY_SECONDS
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub calls: u64,
    /// Sum over records whose model is in the pricing table. `None` when none are.
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub start_unix: f64,
    pub label: String,
    /// Raw provider string to `(input tokens, output tokens)`.
    pub by_provider: BTreeMap<String, (i64, i64)>,
}

pub fn totals(
    records: &[UsageRecord],
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
) -> Totals {
    summarize(included(records, period, now_unix, utc_offset_seconds))
}

/// One entry per raw provider that has at least one record in `period`.
pub fn totals_by_provider(
    records: &[UsageRecord],
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
) -> BTreeMap<String, Totals> {
    let mut grouped: BTreeMap<&str, Vec<&UsageRecord>> = BTreeMap::new();
    for record in included(records, period, now_unix, utc_offset_seconds) {
        grouped
            .entry(record.provider.as_str())
            .or_default()
            .push(record);
    }
    grouped
        .into_iter()
        .map(|(provider, rows)| (provider.to_owned(), summarize(rows.into_iter())))
        .collect()
}

pub fn chart_buckets(
    records: &[UsageRecord],
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
    language: Language,
) -> Vec<Bucket> {
    let mut buckets = empty_buckets(period, now_unix, utc_offset_seconds, language);
    for record in included(records, period, now_unix, utc_offset_seconds) {
        let start = slot_start_unix(period, record.timestamp.unix_seconds(), utc_offset_seconds);
        let Some(index) = buckets.iter().position(|bucket| bucket.start_unix == start) else {
            continue;
        };
        let entry = buckets[index]
            .by_provider
            .entry(record.provider.clone())
            .or_insert((0, 0));
        entry.0 += record.input_tokens;
        entry.1 += record.output_tokens;
    }
    buckets
}

/// `>= 1000` is one decimal and a `K` suffix. Smaller values stay integers.
pub fn format_tokens(n: i64) -> String {
    if n >= 1000 {
        format!("{:.1}K", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// macOS `UsageFormatting.formatCost`.
pub fn format_cost(cost: f64) -> String {
    if cost >= 0.01 {
        format!("${cost:.2}")
    } else if cost >= 0.0001 {
        format!("${cost:.4}")
    } else if cost > 0.0 {
        "<$0.0001".to_owned()
    } else {
        "$0.00".to_owned()
    }
}

fn included(
    records: &[UsageRecord],
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
) -> impl Iterator<Item = &UsageRecord> {
    let cutoff = period.cutoff_unix(now_unix, utc_offset_seconds);
    records
        .iter()
        .filter(move |record| record.timestamp.unix_seconds() >= cutoff)
}

fn summarize<'a>(records: impl Iterator<Item = &'a UsageRecord>) -> Totals {
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut calls = 0;
    let mut cost = None;
    for record in records {
        input_tokens += record.input_tokens;
        output_tokens += record.output_tokens;
        calls += 1;
        if let Some(next) = estimated_cost(&record.model, record.input_tokens, record.output_tokens)
        {
            cost = Some(cost.unwrap_or(0.0) + next);
        }
    }
    Totals {
        input_tokens,
        output_tokens,
        calls,
        cost,
    }
}

fn empty_buckets(
    period: Period,
    now_unix: f64,
    utc_offset_seconds: i32,
    language: Language,
) -> Vec<Bucket> {
    let today = unix_to_local(now_unix, utc_offset_seconds).date();
    let specs: Vec<(NaiveDateTime, String)> = match period {
        Period::Day => (0..24)
            .map(|hour| {
                let local = at_hour(today, hour);
                let label = if hour % 6 == 0 {
                    hour.to_string()
                } else {
                    String::new()
                };
                (local, label)
            })
            .collect(),
        Period::Days7 => (0..7)
            .map(|index| {
                let date = today - Duration::days(6 - i64::from(index));
                (
                    midnight(date),
                    weekday_abbrev(date.weekday(), language).to_owned(),
                )
            })
            .collect(),
        Period::Days30 => (0..30)
            .map(|index| {
                let date = today - Duration::days(29 - i64::from(index));
                let label = if index % 5 == 0 {
                    date.day().to_string()
                } else {
                    String::new()
                };
                (midnight(date), label)
            })
            .collect(),
    };
    specs
        .into_iter()
        .map(|(local, label)| Bucket {
            start_unix: local_to_unix(local, utc_offset_seconds),
            label,
            by_provider: BTreeMap::new(),
        })
        .collect()
}

fn slot_start_unix(period: Period, unix: f64, utc_offset_seconds: i32) -> f64 {
    let local = unix_to_local(unix, utc_offset_seconds);
    let snapped = match period {
        Period::Day => at_hour(local.date(), local.hour()),
        Period::Days7 | Period::Days30 => midnight(local.date()),
    };
    local_to_unix(snapped, utc_offset_seconds)
}

fn start_of_local_day(unix: f64, utc_offset_seconds: i32) -> f64 {
    let local = unix_to_local(unix, utc_offset_seconds);
    local_to_unix(midnight(local.date()), utc_offset_seconds)
}

fn weekday_abbrev(day: Weekday, language: Language) -> &'static str {
    let row = match language {
        Language::En => 0,
        Language::Pl => 1,
        Language::Uk => 2,
        Language::De => 3,
    };
    WEEKDAY_ABBREV[row][day.num_days_from_monday() as usize]
}

fn fixed_offset(seconds: i32) -> FixedOffset {
    FixedOffset::east_opt(seconds)
        .or_else(|| FixedOffset::east_opt(0))
        .expect("a zero UTC offset is inside FixedOffset's range")
}

fn unix_to_local(unix: f64, utc_offset_seconds: i32) -> NaiveDateTime {
    let (secs, nanos) = split_unix(unix);
    let utc = DateTime::from_timestamp(secs, nanos).unwrap_or(DateTime::UNIX_EPOCH);
    fixed_offset(utc_offset_seconds)
        .from_utc_datetime(&utc.naive_utc())
        .naive_local()
}

fn local_to_unix(local: NaiveDateTime, utc_offset_seconds: i32) -> f64 {
    match fixed_offset(utc_offset_seconds).from_local_datetime(&local) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt.timestamp() as f64,
        LocalResult::None => 0.0,
    }
}

fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is a valid civil time")
}

fn at_hour(date: NaiveDate, hour: u32) -> NaiveDateTime {
    date.and_hms_opt(hour, 0, 0)
        .unwrap_or_else(|| midnight(date))
}

/// Whole seconds plus a nanosecond remainder in `0..1_000_000_000`.
fn split_unix(unix: f64) -> (i64, u32) {
    if !unix.is_finite() {
        return (0, 0);
    }
    let mut secs = unix.trunc() as i64;
    let mut frac = unix - secs as f64;
    if frac < 0.0 {
        secs -= 1;
        frac += 1.0;
    }
    let nanos = (frac * 1_000_000_000.0).round();
    if nanos >= 1_000_000_000.0 {
        (secs + 1, 0)
    } else {
        (secs, nanos as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::{chart_buckets, format_cost, format_tokens, totals, totals_by_provider, Period};
    use crate::usage_pricing::estimated_cost;
    use crate::{AIOperation, Language, ProviderType, Timestamp, UsageRecord};

    /// 2026-11-01T10:30:00Z. Local +01:00 is 11:30, so the local day starts at 2026-10-31T23:00:00Z.
    const NOW_NOV1: f64 = 1_793_529_000.0;
    const OFFSET_PLUS_1H: i32 = 3600;
    const NOV1_LOCAL_START: f64 = 1_793_487_600.0;

    fn record(
        provider: ProviderType,
        model: &str,
        input: i64,
        output: i64,
        stamp: Timestamp,
    ) -> UsageRecord {
        UsageRecord::new(
            provider,
            model.to_owned(),
            AIOperation::Translate,
            input,
            output,
            stamp,
        )
    }

    fn claude(model: &str, input: i64, output: i64, stamp: Timestamp) -> UsageRecord {
        record(ProviderType::Claude, model, input, output, stamp)
    }

    #[test]
    fn format_tokens_matches_mac() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_234_567), "1234.6K");
    }

    #[test]
    fn format_cost_matches_mac() {
        assert_eq!(format_cost(0.012), "$0.01");
        assert_eq!(format_cost(0.01), "$0.01");
        assert_eq!(format_cost(0.0011), "$0.0011");
        assert_eq!(format_cost(0.0001), "$0.0001");
        assert_eq!(format_cost(0.00005), "<$0.0001");
        assert_eq!(format_cost(0.0), "$0.00");
    }

    #[test]
    fn totals_sum_only_priced_costs() {
        let records = [
            claude(
                "claude-haiku-5-5",
                1000,
                2000,
                Timestamp::from_unix_seconds(NOW_NOV1 - 10.0),
            ),
            claude(
                "unknown-model",
                5000,
                5000,
                Timestamp::from_unix_seconds(NOW_NOV1 - 20.0),
            ),
        ];
        let got = totals(&records, Period::Day, NOW_NOV1, 0);
        assert_eq!(got.input_tokens, 6000);
        assert_eq!(got.output_tokens, 7000);
        assert_eq!(got.calls, 2);
        let cost = got.cost.expect("the priced record contributes a cost");
        assert!(
            (cost - 0.0011).abs() < 1e-12,
            "cost {cost} is not 0.0011 within 1e-12"
        );
    }

    #[test]
    fn totals_accumulate_records_per_provider() {
        let at = |seconds: f64| Timestamp::from_unix_seconds(NOW_NOV1 - seconds);
        let records = [
            claude("claude-haiku-5-5", 1000, 2000, at(10.0)),
            claude("claude-haiku-5-5", 3000, 4000, at(20.0)),
            record(ProviderType::OpenAI, "gpt-5.4-mini", 500, 700, at(30.0)),
        ];
        let claude_cost = estimated_cost("claude-haiku-5-5", 1000, 2000).unwrap()
            + estimated_cost("claude-haiku-5-5", 3000, 4000).unwrap();
        let openai_cost = estimated_cost("gpt-5.4-mini", 500, 700).unwrap();

        let by_provider = totals_by_provider(&records, Period::Day, NOW_NOV1, 0);
        let claude_totals = &by_provider["Claude"];
        assert_eq!(claude_totals.input_tokens, 4000);
        assert_eq!(claude_totals.output_tokens, 6000);
        assert_eq!(claude_totals.calls, 2);
        assert!((claude_totals.cost.unwrap() - claude_cost).abs() < 1e-12);

        let openai_totals = &by_provider["OpenAI"];
        assert_eq!(openai_totals.input_tokens, 500);
        assert_eq!(openai_totals.output_tokens, 700);
        assert_eq!(openai_totals.calls, 1);
        assert!((openai_totals.cost.unwrap() - openai_cost).abs() < 1e-12);

        let all = totals(&records, Period::Day, NOW_NOV1, 0);
        assert_eq!(all.input_tokens, 4500);
        assert_eq!(all.output_tokens, 6700);
        assert_eq!(all.calls, 3);
        assert!((all.cost.unwrap() - (claude_cost + openai_cost)).abs() < 1e-12);
    }

    #[test]
    fn totals_cost_none_when_nothing_priced() {
        let records = [claude(
            "unknown-model",
            4,
            5,
            Timestamp::from_unix_seconds(NOW_NOV1 - 10.0),
        )];
        let got = totals(&records, Period::Day, NOW_NOV1, 0);
        assert_eq!(got.calls, 1);
        assert_eq!(got.input_tokens, 4);
        assert_eq!(got.cost, None);
    }

    #[test]
    fn totals_by_provider_omits_absent_providers() {
        let cutoff = Period::Day.cutoff_unix(NOW_NOV1, 0);
        let records = [
            claude(
                "claude-haiku-5-5",
                1,
                1,
                Timestamp::from_unix_seconds(NOW_NOV1 - 30.0),
            ),
            record(
                ProviderType::OpenAI,
                "gpt-5.4",
                9,
                9,
                Timestamp::from_unix_seconds(cutoff - 1.0),
            ),
        ];
        let map = totals_by_provider(&records, Period::Day, NOW_NOV1, 0);
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            vec!["Claude".to_owned()]
        );
    }

    #[test]
    fn usage_display_name_matches_mac() {
        assert_eq!(
            super::TRACKED_PROVIDERS,
            [
                ProviderType::Claude,
                ProviderType::OpenAI,
                ProviderType::Gemini,
                ProviderType::Grok,
            ]
        );
        assert_eq!(super::usage_display_name(ProviderType::Claude), "Claude");
        assert_eq!(super::usage_display_name(ProviderType::OpenAI), "OpenAI");
        assert_eq!(super::usage_display_name(ProviderType::Gemini), "Gemini");
        assert_eq!(super::usage_display_name(ProviderType::Grok), "Grok");
        assert_eq!(
            super::usage_display_name(ProviderType::GoogleFree),
            "Google Free"
        );
        assert_eq!(
            super::usage_display_name(ProviderType::GoogleCloud),
            "Google Cloud"
        );
        assert_eq!(super::usage_display_name(ProviderType::DeepL), "DeepL");
        assert_eq!(
            super::usage_display_name(ProviderType::MicrosoftTranslator),
            "Microsoft"
        );
    }

    #[test]
    fn period_cutoff_matches_mac() {
        assert_eq!(
            Period::Day.cutoff_unix(NOW_NOV1, OFFSET_PLUS_1H),
            NOV1_LOCAL_START
        );
        assert_eq!(
            Period::Days7.cutoff_unix(NOW_NOV1, OFFSET_PLUS_1H),
            NOV1_LOCAL_START - 6.0 * 86_400.0
        );
        assert_eq!(
            Period::Days30.cutoff_unix(NOW_NOV1, OFFSET_PLUS_1H),
            NOV1_LOCAL_START - 29.0 * 86_400.0
        );
        assert_eq!(serde_json::to_string(&Period::Day).unwrap(), "\"day\"");
        assert_eq!(serde_json::to_string(&Period::Days7).unwrap(), "\"days7\"");
        assert_eq!(
            serde_json::to_string(&Period::Days30).unwrap(),
            "\"days30\""
        );
        let decoded: Period = serde_json::from_str("\"days30\"").unwrap();
        assert_eq!(decoded, Period::Days30);
    }

    #[test]
    fn records_filtered_on_unix_seconds() {
        let cutoff = Period::Day.cutoff_unix(NOW_NOV1, OFFSET_PLUS_1H);
        let records = [
            claude(
                "claude-haiku-5-5",
                10,
                0,
                Timestamp::from_unix_seconds(NOW_NOV1 - 60.0),
            ),
            claude(
                "claude-haiku-5-5",
                100,
                0,
                Timestamp::from_unix_seconds(cutoff - 1.0),
            ),
            // Raw Apple seconds equal `now`. Read as Unix time that is ~31 years ahead,
            // so the record is inside the period. Comparing the raw value with `now`
            // drops the genuinely recent record above.
            claude(
                "claude-haiku-5-5",
                1,
                0,
                Timestamp::from_apple_seconds(NOW_NOV1),
            ),
        ];
        let got = totals(&records, Period::Day, NOW_NOV1, OFFSET_PLUS_1H);
        assert_eq!(got.calls, 2);
        assert_eq!(got.input_tokens, 11);
    }

    #[test]
    fn buckets_day_has_24_hourly_slots_and_labels() {
        // 2026-11-01T12:30:00Z is 13:30 at +01:00.
        let at_1330 = claude("m", 4, 6, Timestamp::from_unix_seconds(1_793_536_200.0));
        let buckets = chart_buckets(
            &[at_1330],
            Period::Day,
            NOW_NOV1,
            OFFSET_PLUS_1H,
            Language::En,
        );
        assert_eq!(buckets.len(), 24);
        for (index, bucket) in buckets.iter().enumerate() {
            let label = if index % 6 == 0 {
                index.to_string()
            } else {
                String::new()
            };
            assert_eq!(bucket.label, label, "slot {index}");
            assert_eq!(
                bucket.start_unix,
                NOV1_LOCAL_START + index as f64 * 3_600.0,
                "slot {index}"
            );
        }
        assert_eq!(buckets[13].by_provider.get("Claude").copied(), Some((4, 6)));
        assert!(buckets
            .iter()
            .enumerate()
            .all(|(index, bucket)| index == 13 || bucket.by_provider.is_empty()));
        let json = serde_json::to_value(&buckets[0]).unwrap();
        assert!(json.get("startUnix").is_some());
        assert!(json.get("byProvider").is_some());
    }

    #[test]
    fn buckets_days7_end_today_with_weekday_labels() {
        // 2026-11-04T10:00:00Z is Wednesday afternoon at +02:00.
        const NOW: f64 = 1_793_786_400.0;
        const OFFSET: i32 = 7200;
        const TODAY: f64 = 1_793_743_200.0;
        let cases = [
            (
                Language::En,
                ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"],
            ),
            (Language::Pl, ["czw", "pt", "sob", "ndz", "pon", "wt", "śr"]),
            (Language::Uk, ["чт", "пт", "сб", "нд", "пн", "вт", "ср"]),
            (Language::De, ["Do", "Fr", "Sa", "So", "Mo", "Di", "Mi"]),
        ];
        for (language, labels) in cases {
            let buckets = chart_buckets(&[], Period::Days7, NOW, OFFSET, language);
            assert_eq!(buckets.len(), 7);
            assert_eq!(buckets[6].label, labels[6]);
            assert_eq!(buckets[6].start_unix, TODAY);
            let got: Vec<_> = buckets.iter().map(|bucket| bucket.label.as_str()).collect();
            assert_eq!(got, labels);
        }
    }

    #[test]
    fn buckets_days30_label_every_fifth() {
        let buckets = chart_buckets(&[], Period::Days30, NOW_NOV1, OFFSET_PLUS_1H, Language::En);
        assert_eq!(buckets.len(), 30);
        let labeled = ["3", "8", "13", "18", "23", "28"];
        for (index, bucket) in buckets.iter().enumerate() {
            if index % 5 == 0 {
                assert_eq!(bucket.label, labeled[index / 5], "slot {index}");
            } else {
                assert_eq!(bucket.label, "", "slot {index}");
            }
        }
        assert_eq!(buckets[29].label, "");
        assert_eq!(buckets[29].start_unix, NOV1_LOCAL_START);
    }

    #[test]
    fn bucket_boundary_uses_offset() {
        // 2026-11-01T23:30:00Z is 00:30 on the next local day at +01:00.
        const NOW: f64 = 1_793_620_800.0;
        const RECORD: f64 = 1_793_575_800.0;
        const TODAY: f64 = 1_793_574_000.0;
        const PREVIOUS: f64 = 1_793_487_600.0;
        let buckets = chart_buckets(
            &[claude("m", 4, 8, Timestamp::from_unix_seconds(RECORD))],
            Period::Days7,
            NOW,
            OFFSET_PLUS_1H,
            Language::En,
        );
        assert_eq!(buckets.last().unwrap().start_unix, TODAY);
        assert_eq!(
            buckets.last().unwrap().by_provider.get("Claude").copied(),
            Some((4, 8))
        );
        let previous = buckets
            .iter()
            .find(|bucket| bucket.start_unix == PREVIOUS)
            .unwrap();
        assert!(previous.by_provider.is_empty());
    }
}
