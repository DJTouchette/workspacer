use anyhow::{Result, bail};
use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use std::fmt;
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Strict;
        impl<'de> Visitor<'de> for Strict {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("non-null JSON without duplicate fields")
            }
            fn visit_bool<E: Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: Error>(self, v: f64) -> std::result::Result<Unique, E> {
                Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: Error>(self) -> std::result::Result<Unique, E> {
                Err(E::custom("null is not a preference; use reset"))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut map = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if map.contains_key(&k) {
                        return Err(A::Error::custom(format!("duplicate field {k}")));
                    }
                    let v = a.next_value::<Unique>()?;
                    map.insert(k, v.0);
                }
                Ok(Unique(Value::Object(map)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = vec![];
                while let Some(v) = a.next_element::<Unique>()? {
                    values.push(v.0);
                }
                Ok(Unique(Value::Array(values)))
            }
        }
        d.deserialize_any(Strict)
    }
}
/// Must run on raw RPC params before any serde_json::Value decoding. Unknown
/// fields and exact case are checked by the typed preference method afterwards.
pub fn validate_preferences_raw(raw: &[u8]) -> Result<Value> {
    if raw.len() > 256 * 1024 {
        bail!("routing preferences payload too large");
    }
    let raw = if raw.is_empty() {
        b"{}".as_slice()
    } else {
        raw
    };
    let mut deserializer = serde_json::Deserializer::from_slice(raw);
    let value = Unique::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    if !value.is_object() {
        bail!("routing preferences requires one JSON object");
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_preferences_preserve_the_duplicate_and_null_boundary() {
        for raw in [
            r#"{"patch":{"modes":{"global":"normal","global":"conserve"}}}"#,
            r#"{"patch":null}"#,
            r#"{"patch":{"profiles":{"mixed":{"reviewer":{"alternatives":[null]}}}}}"#,
            r#"{} {}"#,
            r#"[]"#,
        ] {
            assert!(validate_preferences_raw(raw.as_bytes()).is_err(), "{raw}");
        }
        assert_eq!(
            validate_preferences_raw(b"").unwrap(),
            serde_json::json!({})
        );
        assert!(
            validate_preferences_raw(
                br#"{"baseRevision":"r","patch":{"modes":{"global":"normal"}}}"#
            )
            .is_ok()
        );
    }
}
