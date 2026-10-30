use std::cmp::Ordering;
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Largest magnitude (2^53) below which every whole `f64` is exactly an integer.
const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;

/// A point in time stored exactly as the persisted files hold it: `f64` seconds
/// since the Apple reference date (2001-01-01T00:00:00Z). Keeping the raw value
/// makes decode/encode lossless. Always finite; display formatting is the
/// frontend's job (it receives Unix time via [`Timestamp::unix_seconds`]).
#[derive(Clone, Copy, Debug)]
pub struct Timestamp(f64);

/// Maps NaN to `0.0` and saturates infinities, so a `Timestamp` is always finite.
fn sanitize(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else if value == f64::INFINITY {
        f64::MAX
    } else if value == f64::NEG_INFINITY {
        f64::MIN
    } else {
        value
    }
}

impl Timestamp {
    /// Seconds between the Unix epoch and the Apple reference date.
    pub const APPLE_TO_UNIX_OFFSET: f64 = 978_307_200.0;

    pub fn from_apple_seconds(seconds: f64) -> Self {
        Self(sanitize(seconds))
    }

    pub fn apple_seconds(self) -> f64 {
        self.0
    }

    pub fn from_unix_seconds(seconds: f64) -> Self {
        Self::from_apple_seconds(seconds - Self::APPLE_TO_UNIX_OFFSET)
    }

    pub fn unix_seconds(self) -> f64 {
        sanitize(self.0 + Self::APPLE_TO_UNIX_OFFSET)
    }

    pub fn plus_seconds(self, seconds: f64) -> Self {
        Self::from_apple_seconds(self.0 + seconds)
    }
}

impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Timestamp {}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Swift's JSONEncoder writes whole-second dates without a fraction.
        if self.0.is_finite() && self.0.fract() == 0.0 && self.0.abs() < MAX_EXACT_INT {
            serializer.serialize_i64(self.0 as i64)
        } else {
            serializer.serialize_f64(self.0)
        }
    }
}

struct TimestampVisitor;

impl Visitor<'_> for TimestampVisitor {
    type Value = Timestamp;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a finite JSON number of seconds since 2001-01-01T00:00:00Z")
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Timestamp, E> {
        if v.is_finite() {
            Ok(Timestamp(v))
        } else {
            Err(E::custom("timestamp must be finite"))
        }
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Timestamp, E> {
        Ok(Timestamp(v as f64))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Timestamp, E> {
        Ok(Timestamp(v as f64))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TimestampVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_epoch_offset() {
        assert_eq!(
            Timestamp::from_apple_seconds(0.0).unix_seconds(),
            978_307_200.0
        );
        assert_eq!(Timestamp::APPLE_TO_UNIX_OFFSET, 978_307_200.0);
        let t = Timestamp::from_unix_seconds(1_761_782_400.5);
        assert_eq!(t.unix_seconds(), 1_761_782_400.5);
        assert_eq!(t.apple_seconds(), 1_761_782_400.5 - 978_307_200.0);
    }

    #[test]
    fn date_round_trips_bit_exact() {
        for s in [
            815059756.035193,
            0.0,
            815059756.0,
            -1.5,
            1_000_000_000.123456,
            1e12,
        ] {
            let json = serde_json::to_string(&Timestamp::from_apple_seconds(s)).unwrap();
            let back: Timestamp = serde_json::from_str(&json).unwrap();
            assert_eq!(
                back.apple_seconds().to_bits(),
                s.to_bits(),
                "{s} via {json}"
            );
        }
    }

    #[test]
    fn accepts_integer_json_number() {
        let t: Timestamp = serde_json::from_str("815059756").unwrap();
        assert_eq!(t.apple_seconds(), 815059756.0);
        let t: Timestamp = serde_json::from_str("-3").unwrap();
        assert_eq!(t.apple_seconds(), -3.0);
    }

    #[test]
    fn rejects_iso_string_and_null() {
        assert!(serde_json::from_str::<Timestamp>("\"2026-10-30T00:00:00Z\"").is_err());
        assert!(serde_json::from_str::<Timestamp>("null").is_err());
        assert!(serde_json::from_str::<Timestamp>("true").is_err());
    }

    #[test]
    fn whole_second_timestamp_serializes_without_fraction() {
        let enc = |s: f64| serde_json::to_string(&Timestamp::from_apple_seconds(s)).unwrap();
        assert_eq!(enc(815059756.0), "815059756");
        assert_eq!(enc(0.0), "0");
        assert_eq!(enc(-1.5), "-1.5");
        for s in [9007199254740992.0_f64, 1e300] {
            let json = enc(s);
            assert!(
                json.contains('.') || json.contains('e') || json.contains('E'),
                "{s} must stay a float, got {json}"
            );
            let back: Timestamp = serde_json::from_str(&json).unwrap();
            assert_eq!(back.apple_seconds().to_bits(), s.to_bits());
        }
    }

    #[test]
    fn absurd_values_do_not_panic() {
        let t: Timestamp = serde_json::from_str("1e300").unwrap();
        assert!(t.unix_seconds().is_finite());
        let _ = t.plus_seconds(1.0);
        let _ = t.cmp(&Timestamp::from_apple_seconds(0.0));
        assert!(serde_json::from_str::<Timestamp>("1e999").is_err());
        assert_eq!(Timestamp::from_apple_seconds(f64::NAN).apple_seconds(), 0.0);
        assert!(Timestamp::from_unix_seconds(f64::INFINITY)
            .apple_seconds()
            .is_finite());
        assert!(Timestamp::from_unix_seconds(f64::NEG_INFINITY)
            .unix_seconds()
            .is_finite());
    }

    #[test]
    fn plus_seconds_never_yields_non_finite() {
        assert_eq!(
            Timestamp::from_apple_seconds(1e308)
                .plus_seconds(1e308)
                .apple_seconds(),
            f64::MAX
        );
        assert_eq!(
            Timestamp::from_apple_seconds(-1e308)
                .plus_seconds(-1e308)
                .apple_seconds(),
            f64::MIN
        );
        assert!(Timestamp::from_apple_seconds(5.0)
            .plus_seconds(f64::NAN)
            .apple_seconds()
            .is_finite());
    }

    #[test]
    fn ordering_is_total_and_chronological() {
        let a = Timestamp::from_apple_seconds(1.0);
        let b = Timestamp::from_apple_seconds(2.0);
        assert!(a < b);
        assert_eq!(a, Timestamp::from_apple_seconds(1.0));
        assert_eq!(a.plus_seconds(1.0), b);
    }
}
