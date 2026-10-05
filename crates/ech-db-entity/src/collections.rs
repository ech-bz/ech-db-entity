use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use ech_db_protocol::values::IntentRef;

use crate::error::{Error, Result};
use crate::ids::{CollectionId, EntityId};
use crate::runtime;
use crate::schema::{BcsSchema, FieldKind, Schema};
use crate::state_keys::CollectionKey;
use crate::storage;

#[derive(Clone, Copy)]
pub struct Binding {
    pub(crate) owner: EntityId,
    pub(crate) epoch: u64,
    pub(crate) generation: u64,
}

impl Binding {
    pub(crate) fn new(owner: EntityId, epoch: u64, generation: u64) -> Self {
        Self {
            owner,
            epoch,
            generation,
        }
    }
}

pub trait CollectionSlot {
    fn kind(&self) -> FieldKind;
    fn address(&self) -> Option<&CollectionId>;
    fn attach(&mut self, id: CollectionId, binding: Binding);
    fn bind(&mut self, binding: Binding);
}

pub struct Map<K, V> {
    id: Option<CollectionId>,
    binding: Option<Binding>,
    marker: PhantomData<fn() -> (K, V)>,
}

impl<K, V> Map<K, V> {
    fn require_address(&self) -> Result<&CollectionId> {
        self.id.as_ref().ok_or(Error::Unbound)
    }

    fn require_write_address(&self) -> Result<&CollectionId> {
        let id = self.require_address()?;
        let binding = self.binding.as_ref().ok_or(Error::NotBound)?;
        if !runtime::binding_valid(binding) {
            return Err(Error::NotBound);
        }
        Ok(id)
    }

    pub fn count(&self) -> Result<u64> {
        let id = self.require_address()?;
        read_counter(&CollectionKey::count(id))
    }
}

impl<K: Serialize, V: DeserializeOwned> Map<K, V> {
    pub fn get(&self, key: &K) -> Result<Option<V>> {
        let id = self.require_address()?;
        let entry = CollectionKey::entry(id, key)?;
        match storage::get(&entry)? {
            None => Ok(None),
            Some(bytes) => bcs::from_bytes(&bytes).map(Some).map_err(|_| Error::ValueDecode),
        }
    }
}

impl<K: Serialize, V: Serialize> Map<K, V> {
    pub fn set(&mut self, key: &K, value: &V) -> Result<()> {
        runtime::ensure_write_allowed()?;
        let result = (|| {
            let id = self.require_write_address()?;
            let entry = CollectionKey::entry(id, key)?;
            let encoded = bcs::to_bytes(value).map_err(|_| Error::Encode)?;
            if storage::get(&entry)?.is_none() {
                let count_key = CollectionKey::count(id);
                let count = read_counter(&count_key)?;
                let next = count.checked_add(1).ok_or(Error::Overflow)?;
                storage::set(&count_key, &encode_u64(next)?)?;
            }
            storage::set(&entry, &encoded)?;
            Ok(())
        })();
        runtime::poison_on_error(result)
    }
}

impl<K: Serialize, V> Map<K, V> {
    pub fn remove(&mut self, key: &K) -> Result<()> {
        runtime::ensure_write_allowed()?;
        let result = (|| {
            let id = self.require_write_address()?;
            let entry = CollectionKey::entry(id, key)?;
            if storage::get(&entry)?.is_none() {
                return Ok(());
            }
            storage::del(&entry)?;
            let count_key = CollectionKey::count(id);
            let count = read_counter(&count_key)?;
            let next = count.checked_sub(1).ok_or(Error::Overflow)?;
            storage::set(&count_key, &encode_u64(next)?)?;
            Ok(())
        })();
        runtime::poison_on_error(result)
    }
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            id: None,
            binding: None,
            marker: PhantomData,
        }
    }
}

impl<K, V> CollectionSlot for Map<K, V> {
    fn kind(&self) -> FieldKind {
        FieldKind::Map
    }

    fn address(&self) -> Option<&CollectionId> {
        self.id.as_ref()
    }

    fn attach(&mut self, id: CollectionId, binding: Binding) {
        self.id = Some(id);
        self.binding = Some(binding);
    }

    fn bind(&mut self, binding: Binding) {
        self.binding = Some(binding);
    }
}

impl<K, V> Serialize for Map<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match &self.id {
            Some(id) => id.serialize(serializer),
            None => Err(serde::ser::Error::custom("unbound collection")),
        }
    }
}

impl<'de, K, V> Deserialize<'de> for Map<K, V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let id = CollectionId::deserialize(deserializer)?;
        Ok(Self {
            id: Some(id),
            binding: None,
            marker: PhantomData,
        })
    }
}

impl<K, V> BcsSchema for Map<K, V> {
    fn schema() -> Schema {
        CollectionId::schema()
    }
}

pub struct Set<K> {
    inner: Map<K, ()>,
}

impl<K> Set<K> {
    pub fn count(&self) -> Result<u64> {
        self.inner.count()
    }
}

impl<K: Serialize> Set<K> {
    pub fn has(&self, key: &K) -> Result<bool> {
        Ok(self.inner.get(key)?.is_some())
    }

    pub fn set(&mut self, key: &K) -> Result<()> {
        self.inner.set(key, &())
    }

    pub fn remove(&mut self, key: &K) -> Result<()> {
        self.inner.remove(key)
    }
}

impl<K> Default for Set<K> {
    fn default() -> Self {
        Self {
            inner: Map::default(),
        }
    }
}

impl<K> CollectionSlot for Set<K> {
    fn kind(&self) -> FieldKind {
        FieldKind::Set
    }

    fn address(&self) -> Option<&CollectionId> {
        self.inner.address()
    }

    fn attach(&mut self, id: CollectionId, binding: Binding) {
        self.inner.attach(id, binding);
    }

    fn bind(&mut self, binding: Binding) {
        self.inner.bind(binding);
    }
}

impl<K> Serialize for Set<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.inner.serialize(serializer)
    }
}

impl<'de, K> Deserialize<'de> for Set<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(Self {
            inner: Map::deserialize(deserializer)?,
        })
    }
}

impl<K> BcsSchema for Set<K> {
    fn schema() -> Schema {
        CollectionId::schema()
    }
}

pub struct Feed<V> {
    id: Option<CollectionId>,
    binding: Option<Binding>,
    marker: PhantomData<fn() -> V>,
}

impl<V> Feed<V> {
    fn require_address(&self) -> Result<&CollectionId> {
        self.id.as_ref().ok_or(Error::Unbound)
    }

    fn require_write_address(&self) -> Result<&CollectionId> {
        let id = self.require_address()?;
        let binding = self.binding.as_ref().ok_or(Error::NotBound)?;
        if !runtime::binding_valid(binding) {
            return Err(Error::NotBound);
        }
        Ok(id)
    }

    pub fn len(&self) -> Result<u64> {
        let id = self.require_address()?;
        read_counter(&CollectionKey::length(id))
    }

    pub fn source(&self, seq: u64) -> Result<Option<IntentRef>> {
        let id = self.require_address()?;
        runtime::feed_source(id, seq)
    }
}

impl<V: DeserializeOwned> Feed<V> {
    pub fn get(&self, seq: u64) -> Result<Option<V>> {
        let id = self.require_address()?;
        let entry = CollectionKey::feed_entry(id, seq);
        match storage::get(&entry)? {
            None => Ok(None),
            Some(bytes) => bcs::from_bytes(&bytes).map(Some).map_err(|_| Error::ValueDecode),
        }
    }
}

impl<V: Serialize> Feed<V> {
    pub fn append(&mut self, value: &V) -> Result<u64> {
        runtime::ensure_write_allowed()?;
        let result = (|| {
            let id = self.require_write_address()?;
            let encoded = bcs::to_bytes(value).map_err(|_| Error::Encode)?;
            append_entry(id, &encoded)
        })();
        runtime::poison_on_error(result)
    }
}

impl<V> Default for Feed<V> {
    fn default() -> Self {
        Self {
            id: None,
            binding: None,
            marker: PhantomData,
        }
    }
}

impl<V> CollectionSlot for Feed<V> {
    fn kind(&self) -> FieldKind {
        FieldKind::Feed
    }

    fn address(&self) -> Option<&CollectionId> {
        self.id.as_ref()
    }

    fn attach(&mut self, id: CollectionId, binding: Binding) {
        self.id = Some(id);
        self.binding = Some(binding);
    }

    fn bind(&mut self, binding: Binding) {
        self.binding = Some(binding);
    }
}

impl<V> Serialize for Feed<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match &self.id {
            Some(id) => id.serialize(serializer),
            None => Err(serde::ser::Error::custom("unbound collection")),
        }
    }
}

impl<'de, V> Deserialize<'de> for Feed<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let id = CollectionId::deserialize(deserializer)?;
        Ok(Self {
            id: Some(id),
            binding: None,
            marker: PhantomData,
        })
    }
}

impl<V> BcsSchema for Feed<V> {
    fn schema() -> Schema {
        CollectionId::schema()
    }
}

pub(crate) fn read_counter(key: &[u8]) -> Result<u64> {
    match storage::get(key)? {
        None => Ok(0),
        Some(bytes) => bcs::from_bytes(&bytes).map_err(|_| Error::ValueDecode),
    }
}

pub(crate) fn encode_u64(value: u64) -> Result<Vec<u8>> {
    bcs::to_bytes(&value).map_err(|_| Error::Encode)
}

pub(crate) fn append_entry(id: &CollectionId, encoded: &[u8]) -> Result<u64> {
    let length_key = CollectionKey::length(id);
    let length = read_counter(&length_key)?;
    let seq = length.checked_add(1).ok_or(Error::Overflow)?;
    storage::set(&CollectionKey::feed_entry(id, seq), encoded)?;
    storage::set(&length_key, &encode_u64(seq)?)?;
    if let Some(source) = runtime::intent_source() {
        let bytes = bcs::to_bytes(&source).map_err(|_| Error::Encode)?;
        storage::set(&CollectionKey::source(id, seq), &bytes)?;
    }
    Ok(seq)
}
