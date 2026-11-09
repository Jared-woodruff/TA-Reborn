//! Allocation limits at the serialized command boundary. These only affect decoding;
//! serde's existing sequence representation stays unchanged.

use std::marker::PhantomData;
use serde::{Deserialize, Deserializer};
use serde::de::{Error, SeqAccess, Visitor};

pub const MAX_COMMAND_ITEMS: usize = 1_000_000;

/// Reject oversized declared lengths before allocating, and never trust a size hint
/// enough to reserve more than 1 MiB initially. Also supports sequences without hints.
pub fn vector<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where D: Deserializer<'de>, T: Deserialize<'de> {
    struct Bounded<T, const MAX: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for Bounded<T, MAX> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "at most {MAX} sequence items") }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let hint = sequence.size_hint();
            let count = hint.unwrap_or(0);
            if count > MAX { return Err(A::Error::custom("sequence item limit exceeded")); }
            let reserve = count.min((1 << 20) / std::mem::size_of::<T>().max(1));
            let mut values = Vec::new();
            values.try_reserve_exact(reserve).map_err(A::Error::custom)?;
            while let Some(value) = sequence.next_element()? {
                if values.len() >= MAX { return Err(A::Error::custom("sequence item limit exceeded")); }
                if values.len() == values.capacity() {
                    let capacity = values.capacity().max(1).saturating_mul(2).min(hint.unwrap_or(MAX));
                    values.try_reserve_exact(capacity.saturating_sub(values.len()).max(1)).map_err(A::Error::custom)?;
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded::<T, MAX>(PhantomData))
}

pub fn command_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where D: Deserializer<'de>, T: Deserialize<'de> {
    vector::<D, T, MAX_COMMAND_ITEMS>(deserializer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn count_limit_rejects_known_and_unknown_length_sequences() {
        let sequence = serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new([1u32, 2, 3].into_iter());
        assert!(vector::<_, u32, 2>(sequence).unwrap_err().to_string().contains("item limit"));
        let sequence = serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new([1u32, 2].into_iter());
        assert_eq!(vector::<_, u32, 2>(sequence).unwrap(), [1, 2]);
        let sequence = serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new((0u32..3).filter(|_| true));
        assert!(vector::<_, u32, 2>(sequence).unwrap_err().to_string().contains("item limit"));
    }
}
