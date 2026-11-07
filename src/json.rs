/// Decode bounded caller input without accepting duplicate object keys.
/// Serde's recursion limit remains enabled. Callers own their byte/item budgets.
pub fn decode_unique_json<T: serde::de::DeserializeOwned>(
    contents: &[u8],
) -> Result<T, serde_json::Error> {
    use serde::{Deserialize, Deserializer, de};
    use std::collections::HashSet;
    use std::fmt;

    struct UniqueFields;
    struct Visitor;

    impl<'de> Deserialize<'de> for UniqueFields {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_any(Visitor)
        }
    }

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = UniqueFields;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("JSON without duplicate object keys")
        }

        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(UniqueFields)
        }

        fn visit_seq<A: de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            while sequence.next_element::<UniqueFields>()?.is_some() {}
            Ok(UniqueFields)
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !keys.insert(key) {
                    return Err(de::Error::custom("duplicate object key"));
                }
                map.next_value::<UniqueFields>()?;
            }
            Ok(UniqueFields)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(contents);
    UniqueFields::deserialize(&mut deserializer)?;
    deserializer.end()?;
    serde_json::from_slice(contents)
}
