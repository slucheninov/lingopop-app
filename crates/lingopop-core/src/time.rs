use std::cmp::Ordering;
use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Maps NaN and `-0.0` to `0.0` and saturates infinities, so a `Timestamp` is always
/// finite and `-0.0` cannot split equality / ordering from the integer `0` encoding.
fn sanitize(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
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
            Ok(Timestamp(sanitize(v)))
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

/// Source of "now" for the stores, injected so tests control time.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The wall clock (`std::time::SystemTime`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let unix = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs_f64(),
            Err(before) => -before.duration().as_secs_f64(),
        };
        Timestamp::from_unix_seconds(unix)
    }
}

/// A clock tests drive by hand; public so later phases reuse it.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Timestamp>,
}

impl ManualClock {
    pub fn new(start: Timestamp) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    pub fn set(&self, now: Timestamp) {
        *self.lock() = now;
    }

    pub fn advance(&self, seconds: f64) {
        let mut now = self.lock();
        *now = now.plus_seconds(seconds);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Timestamp> {
        // A poisoned lock still holds a valid `Timestamp`.
        self.now.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.lock()
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
    fn integer_boundary_encoding() {
        let enc = |s: f64| serde_json::to_string(&Timestamp::from_apple_seconds(s)).unwrap();
        // Largest exactly representable odd integer, 2^53 - 1: still an integer.
        assert_eq!(enc(9007199254740991.0), "9007199254740991");
        assert_eq!(enc(-9007199254740991.0), "-9007199254740991");
        assert_eq!(enc(-3.0), "-3");
        // The rule is `abs() < 2^53`, so -2^53 takes the float path (and round-trips).
        let json = enc(-9007199254740992.0);
        assert_eq!(json, "-9007199254740992.0");
        let back: Timestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.apple_seconds().to_bits(),
            (-9007199254740992.0_f64).to_bits()
        );
    }

    #[test]
    fn negative_zero_is_normalized_everywhere() {
        let zero = Timestamp::from_apple_seconds(0.0);
        let from_ctor = Timestamp::from_apple_seconds(-0.0);
        assert_eq!(from_ctor.apple_seconds().to_bits(), 0.0_f64.to_bits());
        assert_eq!(from_ctor, zero);
        assert_eq!(from_ctor.cmp(&zero), Ordering::Equal);
        assert_eq!(serde_json::to_string(&from_ctor).unwrap(), "0");
        // Decoding a JSON -0.0 / -0 gives the same value as 0.
        for json in ["-0.0", "-0", "-0e0"] {
            let t: Timestamp = serde_json::from_str(json).unwrap();
            assert_eq!(t.apple_seconds().to_bits(), 0.0_f64.to_bits(), "{json}");
            assert_eq!(t, zero, "{json}");
        }
        // Arithmetic that lands on -0.0 is normalized as well.
        let sum = Timestamp::from_apple_seconds(-0.0).plus_seconds(-0.0);
        assert_eq!(sum.apple_seconds().to_bits(), 0.0_f64.to_bits());
        let unix = Timestamp::from_unix_seconds(Timestamp::APPLE_TO_UNIX_OFFSET);
        assert_eq!(unix.apple_seconds().to_bits(), 0.0_f64.to_bits());
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

    #[test]
    fn manual_clock_set_and_advance() {
        let clock = ManualClock::new(Timestamp::from_apple_seconds(100.0));
        assert_eq!(clock.now(), Timestamp::from_apple_seconds(100.0));
        clock.advance(10.5);
        assert_eq!(clock.now(), Timestamp::from_apple_seconds(110.5));
        clock.set(Timestamp::from_apple_seconds(50.0));
        assert_eq!(clock.now(), Timestamp::from_apple_seconds(50.0));
    }

    #[test]
    fn system_clock_is_close_to_the_real_time() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let now = SystemClock.now().unix_seconds();
        assert!((now - before).abs() < 60.0, "{now} vs {before}");
    }
}
