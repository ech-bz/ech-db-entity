use std::cell::RefCell;
use std::rc::Rc;

use serde::de::{DeserializeSeed, Error as DeError, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::EntityId;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EntityRecord<T> {
    pub type_name: String,
    pub parent: Option<EntityId>,
    pub state: T,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("entity type mismatch: expected {expected}, found {found}")]
    TypeMismatch { expected: String, found: String },
    #[error("entity record does not decode")]
    Decode,
}

impl<T: serde::de::DeserializeOwned> EntityRecord<T> {
    pub fn decode(bytes: &[u8], expected_type: &str) -> Result<Self, RecordError> {
        let mismatch = Rc::new(RefCell::new(None));
        let seed = RecordSeed::<T> {
            expected: expected_type.to_string(),
            mismatch: mismatch.clone(),
            marker: std::marker::PhantomData,
        };
        match bcs::from_bytes_seed(seed, bytes) {
            Ok(record) => Ok(record),
            Err(_) => match mismatch.borrow_mut().take() {
                Some(found) => Err(RecordError::TypeMismatch {
                    expected: expected_type.to_string(),
                    found,
                }),
                None => Err(RecordError::Decode),
            },
        }
    }
}

pub struct EntityRecordRef<'a, T: ?Sized> {
    pub type_name: &'a str,
    pub parent: Option<EntityId>,
    pub state: &'a T,
}

impl<T: Serialize + ?Sized> EntityRecordRef<'_, T> {
    pub fn encode(&self) -> Result<Vec<u8>, bcs::Error> {
        bcs::to_bytes(self)
    }
}

impl<T: Serialize + ?Sized> Serialize for EntityRecordRef<'_, T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut record = serializer.serialize_struct("EntityRecord", 3)?;
        record.serialize_field("type_name", self.type_name)?;
        record.serialize_field("parent", &self.parent)?;
        record.serialize_field("state", self.state)?;
        record.end()
    }
}

struct RecordSeed<T> {
    expected: String,
    mismatch: Rc<RefCell<Option<String>>>,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for RecordSeed<T> {
    type Value = EntityRecord<T>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(
            "EntityRecord",
            &["type_name", "parent", "state"],
            RecordVisitor {
                expected: self.expected,
                mismatch: self.mismatch,
                marker: std::marker::PhantomData,
            },
        )
    }
}

struct RecordVisitor<T> {
    expected: String,
    mismatch: Rc<RefCell<Option<String>>>,
    marker: std::marker::PhantomData<T>,
}

impl<'de, T: Deserialize<'de>> Visitor<'de> for RecordVisitor<T> {
    type Value = EntityRecord<T>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a struct EntityRecord")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let type_name: String = seq
            .next_element()?
            .ok_or_else(|| A::Error::custom("missing entity type name"))?;
        if type_name != self.expected {
            *self.mismatch.borrow_mut() = Some(type_name.clone());
            return Err(A::Error::custom("entity type mismatch"));
        }
        let parent: Option<EntityId> = seq
            .next_element()?
            .ok_or_else(|| A::Error::custom("missing entity parent"))?;
        let state: T = seq
            .next_element()?
            .ok_or_else(|| A::Error::custom("missing entity state"))?;
        Ok(EntityRecord {
            type_name,
            parent,
            state,
        })
    }
}
