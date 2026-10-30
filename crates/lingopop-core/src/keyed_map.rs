//! `ProviderKeyedMap`: a `[ProviderType: V]` map persisted as a JSON object keyed
//! by the provider's raw value. Port of `Models/ProviderKeyedDictionary.swift`.
//!
//! Decoding is lenient: the object form and the legacy flat array
//! `[k1, v1, k2, v2]` are both read; entries whose key is not a known provider
//! (a newer app version) or whose value does not decode are skipped; a trailing
//! key without a value is ignored. A top-level scalar is an error.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::provider::ProviderType;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderKeyedMap<V>(pub BTreeMap<ProviderType, V>);

// Not derived: the derive would require `V: Default`.
impl<V> Default for ProviderKeyedMap<V> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<V: Serialize> Serialize for ProviderKeyedMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(k, v)| (k.raw_value(), v)))
    }
}

fn entry<V: DeserializeOwned>(key: &Value, value: &Value) -> Option<(ProviderType, V)> {
    let provider = ProviderType::from_raw_value(key.as_str()?)?;
    let value = serde_json::from_value(value.clone()).ok()?;
    Some((provider, value))
}

impl<'de, V: DeserializeOwned> Deserialize<'de> for ProviderKeyedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = match Value::deserialize(deserializer)? {
            Value::Object(object) => object
                .iter()
                .filter_map(|(k, v)| entry(&Value::String(k.clone()), v))
                .collect(),
            // Legacy synthesized form; `zip` drops a trailing key without a value.
            Value::Array(items) => items
                .iter()
                .step_by(2)
                .zip(items.iter().skip(1).step_by(2))
                .filter_map(|(key, value)| entry(key, value))
                .collect(),
            other => {
                return Err(serde::de::Error::custom(format!(
                    "expected an object or a legacy array, found {}",
                    kind(&other)
                )))
            }
        };
        Ok(Self(map))
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: &str) -> Result<ProviderKeyedMap<i32>, serde_json::Error> {
        serde_json::from_str(json)
    }

    fn map(pairs: &[(ProviderType, i32)]) -> ProviderKeyedMap<i32> {
        ProviderKeyedMap(pairs.iter().copied().collect())
    }

    #[test]
    fn encodes_object_keyed_by_raw_value() {
        let json = serde_json::to_string(&map(&[(ProviderType::Claude, 1)])).unwrap();
        assert_eq!(json, r#"{"Claude":1}"#);
        let json = serde_json::to_string(&map(&[(ProviderType::GoogleFree, 7)])).unwrap();
        assert_eq!(json, r#"{"Google (Free)":7}"#);
    }

    #[test]
    fn decodes_legacy_flat_array() {
        let decoded = decode(r#"["Claude",1,"DeepL",2]"#).unwrap();
        assert_eq!(
            decoded,
            map(&[(ProviderType::Claude, 1), (ProviderType::DeepL, 2)])
        );
    }

    #[test]
    fn skips_unknown_provider_key() {
        let decoded = decode(r#"{"Claude":1,"Bogus":2}"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 1)]));
        let decoded = decode(r#"["Bogus",2,"Claude",1]"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 1)]));
    }

    #[test]
    fn skips_undecodable_value() {
        let decoded = decode(r#"{"Claude":1,"DeepL":"x"}"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 1)]));
        let decoded = decode(r#"["Claude",1,"DeepL","x"]"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 1)]));
    }

    #[test]
    fn top_level_scalar_is_error() {
        assert!(decode("42").is_err());
        assert!(decode(r#""Claude""#).is_err());
        assert!(decode("null").is_err());
        assert!(decode("true").is_err());
    }

    #[test]
    fn odd_length_legacy_array_does_not_panic() {
        let decoded = decode(r#"["Claude",1,"DeepL"]"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 1)]));
        assert_eq!(decode("[]").unwrap(), ProviderKeyedMap::default());
        assert_eq!(
            decode(r#"["Claude"]"#).unwrap(),
            ProviderKeyedMap::default()
        );
    }

    #[test]
    fn non_string_legacy_key_is_skipped_but_pairing_is_kept() {
        let decoded = decode(r#"[5,1,"Claude",2]"#).unwrap();
        assert_eq!(decoded, map(&[(ProviderType::Claude, 2)]));
    }

    #[test]
    fn round_trip_through_object_form() {
        let original = map(&[
            (ProviderType::Claude, 1),
            (ProviderType::MicrosoftTranslator, 2),
        ]);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(decode(&json).unwrap(), original);
    }
}
