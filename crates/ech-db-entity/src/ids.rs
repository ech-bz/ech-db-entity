use core::fmt;

use serde::{Deserialize, Serialize};

use ech_db_protocol::hash::Blake2b256;

use crate::error::{Error, Result};

pub const EVENTS_FIELD: &str = "$events";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub [u8; 32]);

impl EntityId {
    const ROOT_PREFIX: &'static [u8] = b"root-entity";
    const CHILD_PREFIX: &'static [u8] = b"entity";

    pub fn root(application_root: &[u8; 32]) -> Self {
        Self(Blake2b256::hash(&[Self::ROOT_PREFIX, application_root]))
    }

    pub fn child<K: Serialize + ?Sized>(parent: Self, type_name: &str, key: &K) -> Result<Self> {
        let key = bcs::to_bytes(key).map_err(|_| Error::Encode)?;
        Self::child_key_bytes(parent, type_name, &key)
    }

    pub fn child_key_bytes(parent: Self, type_name: &str, key: &[u8]) -> Result<Self> {
        let prefix = bcs::to_bytes(type_name).map_err(|_| Error::Encode)?;
        Ok(Self(Blake2b256::hash(&[
            Self::CHILD_PREFIX,
            &parent.0,
            &prefix,
            key,
        ])))
    }

    pub fn collection(&self, stable_field: &str) -> Result<CollectionId> {
        let field = bcs::to_bytes(stable_field).map_err(|_| Error::Encode)?;
        Ok(CollectionId(Blake2b256::hash(&[
            b"field",
            &self.0,
            &field,
        ])))
    }

    pub fn events(&self) -> Result<CollectionId> {
        self.collection(EVENTS_FIELD)
    }
}

impl fmt::Debug for EntityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntityId(")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str(")")
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CollectionId([u8; 32]);

impl CollectionId {
    pub fn as_array(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for CollectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CollectionId(")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str(")")
    }
}
