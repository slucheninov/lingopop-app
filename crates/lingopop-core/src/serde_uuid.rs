//! `#[serde(with = "crate::serde_uuid")]` codec: uppercase hyphenated on write
//! (as Swift's `UUID.uuidString`), any case on read.

use serde::{de, Deserialize, Deserializer, Serializer};
use uuid::Uuid;

pub fn serialize<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(id.hyphenated().encode_upper(&mut Uuid::encode_buffer()))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let text = String::deserialize(deserializer)?;
    Uuid::parse_str(&text).map_err(de::Error::custom)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "crate::serde_uuid")]
        id: Uuid,
    }

    const UPPER: &str = "16990D16-7E15-47C3-A299-EF2AEC4400D1";

    #[test]
    fn uuid_written_uppercase() {
        let h = Holder {
            id: Uuid::parse_str(UPPER).unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&h).unwrap(),
            format!("{{\"id\":\"{UPPER}\"}}")
        );
    }

    #[test]
    fn uuid_read_case_insensitive() {
        let lower: Holder =
            serde_json::from_str(&format!("{{\"id\":\"{}\"}}", UPPER.to_lowercase())).unwrap();
        let mixed: Holder =
            serde_json::from_str("{\"id\":\"16990d16-7E15-47c3-A299-ef2aeC4400d1\"}").unwrap();
        let upper: Holder = serde_json::from_str(&format!("{{\"id\":\"{UPPER}\"}}")).unwrap();
        assert_eq!(lower, upper);
        assert_eq!(mixed, upper);
        assert!(serde_json::from_str::<Holder>("{\"id\":\"not-a-uuid\"}").is_err());
        assert!(serde_json::from_str::<Holder>("{\"id\":5}").is_err());
    }
}
