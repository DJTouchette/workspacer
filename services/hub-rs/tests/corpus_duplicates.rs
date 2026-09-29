//! Validate JSON tokens before normal map decoding can discard duplicate keys.
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{collections::BTreeSet, fmt};

struct Scan<'a> {
    path: String,
    duplicates: &'a mut Vec<String>,
}
impl<'de> DeserializeSeed<'de> for Scan<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Scan<'_> {
    type Value = ();
    fn expecting(&self, out: &mut fmt::Formatter) -> fmt::Result {
        out.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut seen = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            let path = format!("{}.{}", self.path, key);
            if !seen.insert(key) {
                self.duplicates.push(path.clone());
            }
            map.next_value_seed(Scan {
                path,
                duplicates: &mut *self.duplicates,
            })?;
        }
        Ok(())
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let mut index = 0;
        while seq
            .next_element_seed(Scan {
                path: format!("{}[{index}]", self.path),
                duplicates: &mut *self.duplicates,
            })?
            .is_some()
        {
            index += 1;
        }
        Ok(())
    }
}
fn duplicate_keys(bytes: &[u8]) -> Result<Vec<String>, serde_json::Error> {
    let mut duplicates = Vec::new();
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    Scan {
        path: "$".into(),
        duplicates: &mut duplicates,
    }
    .deserialize(&mut decoder)?;
    decoder.end()?;
    duplicates.sort();
    Ok(duplicates)
}

#[test]
fn every_contract_json_rejects_duplicate_keys_before_map_decoding() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let mut count = 0;
    let mut directories = vec![directory];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                directories.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let duplicates = duplicate_keys(&std::fs::read(&path).unwrap())
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                duplicates.is_empty(),
                "{} has duplicate keys: {duplicates:?}",
                path.display()
            );
            count += 1;
        }
    }
    assert!(
        count >= 16,
        "contract discovery silently shrank to {count} files"
    );
}

#[test]
fn token_guard_detects_nested_array_and_escaped_key_mutations() {
    for (body, expected) in [
        (r#"{"a":1,"a":2}"#, vec!["$.a"]),
        (r#"{"outer":{"note":"x","note":"y"}}"#, vec!["$.outer.note"]),
        (
            r#"{"cases":[{"why":"a"},{"why":"b","why":"c"}]}"#,
            vec!["$.cases[1].why"],
        ),
        (r#"{"a":1,"\u0061":2}"#, vec!["$.a"]),
        (r#"{"a":{"k":1},"b":{"k":2}}"#, vec![]),
        (r#"{"a":"x","b":"x"}"#, vec![]),
        (
            r#"{"text":"{\"a\":1,\"a\":2}","empty":null,"bool":true,"number":1.25}"#,
            vec![],
        ),
    ] {
        assert_eq!(duplicate_keys(body.as_bytes()).unwrap(), expected, "{body}");
    }
    for invalid in [b"{} {}".as_slice(), b"{broken}", b"[1,"] {
        assert!(duplicate_keys(invalid).is_err());
    }
}
