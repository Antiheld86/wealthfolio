//! Exact-precision `Decimal` (de)serialization for snapshot JSON fields.
//!
//! The workspace's `rust_decimal` default (`serde-float`) round-trips every
//! `Decimal` through `f64` on serialize, which only holds ~15-17 significant
//! decimal digits and can silently drift on repeated snapshot read/write
//! cycles. These helpers serialize as exact decimal strings instead.
//!
//! `rust_decimal::serde::str` looked like a ready-made fix, but its
//! `deserialize` calls `deserialize_str` directly and errors on a JSON number
//! rather than falling back to it (confirmed by a failing regression test) —
//! so it can't read snapshots written before this change. These helpers
//! accept either a JSON string or a JSON number on read, and always write a
//! string, so old rows keep loading and new rows stop losing precision.

use rust_decimal::Decimal;
use serde::{de::Error as DeError, Deserialize, Deserializer, Serializer};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Deserialize)]
#[serde(untagged)]
enum NumOrStr {
    Str(String),
    Num(f64),
}

fn decimal_from_num_or_str<E: DeError>(value: NumOrStr) -> Result<Decimal, E> {
    match value {
        NumOrStr::Str(s) => Decimal::from_str(&s).map_err(DeError::custom),
        NumOrStr::Num(n) => Decimal::try_from(n).map_err(DeError::custom),
    }
}

/// For a plain `Decimal` field: `#[serde(with = "decimal_serde")]`.
pub fn serialize<S>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    decimal_from_num_or_str(NumOrStr::deserialize(deserializer)?)
}

/// For an `Option<Decimal>` field: `#[serde(with = "decimal_serde::option")]`.
pub mod option {
    use super::*;

    pub fn serialize<S>(value: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(d) => serializer.serialize_some(&d.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<NumOrStr>::deserialize(deserializer)? {
            Some(value) => decimal_from_num_or_str(value).map(Some),
            None => Ok(None),
        }
    }
}

/// For a `HashMap<String, Decimal>` field: `#[serde(with = "decimal_serde::map")]`.
pub mod map {
    use super::*;

    pub fn serialize<S>(value: &HashMap<String, Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let as_strings: HashMap<&String, String> =
            value.iter().map(|(k, v)| (k, v.to_string())).collect();
        serde::Serialize::serialize(&as_strings, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: HashMap<String, NumOrStr> = HashMap::deserialize(deserializer)?;
        raw.into_iter()
            .map(|(k, v)| decimal_from_num_or_str(v).map(|d| (k, d)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrapper {
        #[serde(with = "super")]
        value: Decimal,
        #[serde(with = "super::option")]
        opt: Option<Decimal>,
    }

    #[test]
    fn round_trips_high_precision_value_as_string() {
        let w = Wrapper {
            value: Decimal::from_str("12345678901234.123456789").unwrap(),
            opt: Some(dec!(1.5)),
        };
        let json = serde_json::to_string(&w).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["value"].is_string());

        let round_tripped: Wrapper = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.value, w.value);
        assert_eq!(round_tripped.opt, w.opt);
    }

    #[test]
    fn deserializes_legacy_numeric_json() {
        let json = r#"{"value": 10.5, "opt": 3}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.value, dec!(10.5));
        assert_eq!(w.opt, Some(dec!(3)));
    }

    #[test]
    fn deserializes_null_option() {
        let json = r#"{"value": "1", "opt": null}"#;
        let w: Wrapper = serde_json::from_str(json).unwrap();
        assert_eq!(w.opt, None);
    }
}
