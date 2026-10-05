use foundationdb_tuple::{pack, Bytes};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::ids::{CollectionId, EntityId};

pub struct EntityKey;

impl EntityKey {
    pub fn record(id: &EntityId) -> Vec<u8> {
        pack(&("entity", Bytes::from(id.0.as_slice())))
    }
}

pub struct CollectionKey;

impl CollectionKey {
    pub fn entry<K: Serialize + ?Sized>(id: &CollectionId, key: &K) -> Result<Vec<u8>> {
        let key = bcs::to_bytes(key).map_err(|_| Error::Encode)?;
        Ok(Self::entry_bytes(id, &key))
    }

    pub fn entry_bytes(id: &CollectionId, key: &[u8]) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "entry",
            Bytes::from(key),
        ))
    }

    pub fn feed_entry(id: &CollectionId, seq: u64) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "entry",
            seq,
        ))
    }

    pub fn source(id: &CollectionId, seq: u64) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "source",
            seq,
        ))
    }

    pub fn count(id: &CollectionId) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "count",
        ))
    }

    pub fn length(id: &CollectionId) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "length",
        ))
    }

    pub fn entry_prefix(id: &CollectionId) -> Vec<u8> {
        pack(&(
            "collection",
            Bytes::from(id.as_array().as_slice()),
            "entry",
        ))
    }

    pub fn decode_entry_key(suffix: &[u8]) -> Result<Vec<u8>> {
        foundationdb_tuple::unpack::<Bytes>(suffix)
            .map(|key| key.into_owned())
            .map_err(|_| Error::ValueDecode)
    }

    pub fn decode_feed_seq(suffix: &[u8]) -> Result<u64> {
        foundationdb_tuple::unpack::<u64>(suffix).map_err(|_| Error::ValueDecode)
    }
}
