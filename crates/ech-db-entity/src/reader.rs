use serde::de::DeserializeOwned;
use serde::Serialize;

use ech_db_client::{Client, ReadSession};
use ech_db_protocol::ids::Stream;
use ech_db_protocol::keys::{Prefix, Range, ReadKey};
use ech_db_protocol::values::AttestedIntent;

use crate::error::Error as EntityError;
use crate::ids::{CollectionId, EntityId};
use crate::record::{EntityRecord, RecordError};
use crate::state_keys::{CollectionKey, EntityKey};

pub const MAX_PAGE: usize = 256;

const MAX_CURSOR: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Client(#[from] ech_db_client::Error),
    #[error(transparent)]
    Entity(#[from] EntityError),
    #[error("protocol violation")]
    Protocol,
    #[error("page limit out of range")]
    PageLimit,
}

#[derive(Clone, Copy)]
pub enum Counter {
    Count,
    Length,
}

#[derive(Clone, Debug)]
pub struct Cursor(Vec<u8>);

pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<Cursor>,
}

pub struct MapEntry<K, V> {
    pub key: K,
    pub value: V,
}

pub struct FeedEntry<V> {
    pub seq: u64,
    pub value: V,
}

#[derive(Clone)]
pub struct EntityReader {
    session: ReadSession,
}

impl EntityReader {
    pub fn new(session: ReadSession) -> Self {
        Self { session }
    }

    pub async fn read<T, F, Fut>(client: &Client, operation: F) -> Result<T, ReadError>
    where
        F: Fn(EntityReader) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T, ReadError>> + Send,
    {
        let result = client
            .read(move |session| {
                let future = operation(EntityReader::new(session));
                async move { Ok(future.await) }
            })
            .await?;
        result
    }

    pub async fn raw_get(&self, keys: Vec<ReadKey>) -> Result<Vec<Option<Vec<u8>>>, ReadError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.session.get_raw(&keys).await?)
    }

    pub async fn raw_range(
        &self,
        range: Range,
        limit: Option<u64>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ReadError> {
        Ok(self.session.get_range_raw_limited(&range, limit).await?)
    }

    pub async fn state_get(&self, keys: Vec<Vec<u8>>) -> Result<Vec<Option<Vec<u8>>>, ReadError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let read_keys: Vec<ReadKey> = keys.into_iter().map(ReadKey::state).collect();
        self.raw_get(read_keys).await
    }

    pub async fn record<T: DeserializeOwned>(
        &self,
        id: EntityId,
        expected_type: &str,
    ) -> Result<Option<EntityRecord<T>>, ReadError> {
        let key = EntityKey::record(&id);
        let Some(bytes) = self.point(&key).await? else {
            return Ok(None);
        };
        EntityRecord::decode(&bytes, expected_type)
            .map(Some)
            .map_err(record_error)
    }

    pub async fn counter(&self, id: &CollectionId, kind: Counter) -> Result<u64, ReadError> {
        let key = match kind {
            Counter::Count => CollectionKey::count(id),
            Counter::Length => CollectionKey::length(id),
        };
        self.counter_value(&key).await
    }

    pub async fn map_count(&self, id: &CollectionId) -> Result<u64, ReadError> {
        self.counter_value(&CollectionKey::count(id)).await
    }

    pub async fn set_count(&self, id: &CollectionId) -> Result<u64, ReadError> {
        self.counter_value(&CollectionKey::count(id)).await
    }

    pub async fn feed_len(&self, id: &CollectionId) -> Result<u64, ReadError> {
        self.counter_value(&CollectionKey::length(id)).await
    }

    pub async fn set_has<K: Serialize + ?Sized>(
        &self,
        id: &CollectionId,
        key: &K,
    ) -> Result<bool, ReadError> {
        Ok(self.point(&CollectionKey::entry(id, key)?).await?.is_some())
    }

    pub async fn map_get<K: Serialize + ?Sized, V: DeserializeOwned>(
        &self,
        id: &CollectionId,
        key: &K,
    ) -> Result<Option<V>, ReadError> {
        match self.point(&CollectionKey::entry(id, key)?).await? {
            None => Ok(None),
            Some(bytes) => decode(&bytes).map(Some),
        }
    }

    pub async fn feed_entry<V: DeserializeOwned>(
        &self,
        id: &CollectionId,
        seq: u64,
    ) -> Result<Option<V>, ReadError> {
        match self.point(&CollectionKey::feed_entry(id, seq)).await? {
            None => Ok(None),
            Some(bytes) => decode(&bytes).map(Some),
        }
    }

    pub async fn feed_range(
        &self,
        id: &CollectionId,
        start: u64,
        end_exclusive: u64,
    ) -> Result<Vec<Option<Vec<u8>>>, ReadError> {
        if end_exclusive <= start {
            return Ok(Vec::new());
        }
        let keys: Vec<ReadKey> = (start..end_exclusive)
            .map(|seq| ReadKey::state(CollectionKey::feed_entry(id, seq)))
            .collect();
        self.raw_get(keys).await
    }

    pub async fn set_members<K: DeserializeOwned>(&self, id: &CollectionId) -> Result<Vec<K>, ReadError> {
        let mut cursor: Option<Vec<u8>> = None;
        let mut values = Vec::new();
        loop {
            let (rows, next) = self.entry_page(id, cursor, MAX_PAGE).await?;
            for (suffix, _) in rows {
                values.push(decode(&CollectionKey::decode_entry_key(&suffix)?)?);
            }
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(values)
    }

    pub async fn map_entries<K: DeserializeOwned, V: DeserializeOwned>(
        &self,
        id: &CollectionId,
    ) -> Result<Vec<(K, V)>, ReadError> {
        let mut cursor: Option<Vec<u8>> = None;
        let mut values = Vec::new();
        loop {
            let (rows, next) = self.entry_page(id, cursor, MAX_PAGE).await?;
            for (suffix, value) in rows {
                values.push((
                    decode(&CollectionKey::decode_entry_key(&suffix)?)?,
                    decode(&value)?,
                ));
            }
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(values)
    }

    pub async fn intents_for(
        &self,
        refs: &[([u8; 32], u64)],
    ) -> Result<Vec<Option<AttestedIntent>>, ReadError> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<ReadKey> = refs
            .iter()
            .map(|(author, seq)| ReadKey::log_data(&Stream::intent(*author).id(), *seq))
            .collect();
        let values = self.raw_get(keys).await?;
        Ok(values
            .into_iter()
            .map(|value| value.and_then(|bytes| bcs::from_bytes::<AttestedIntent>(&bytes).ok()))
            .collect())
    }

    pub async fn map_page<K: DeserializeOwned, V: DeserializeOwned>(
        &self,
        id: &CollectionId,
        cursor: Option<Cursor>,
        limit: usize,
    ) -> Result<Page<MapEntry<K, V>>, ReadError> {
        let (rows, next) = self.entry_page(id, cursor.map(|cursor| cursor.0), limit).await?;
        let mut items = Vec::with_capacity(rows.len());
        for (suffix, value) in rows {
            items.push(MapEntry {
                key: decode(&CollectionKey::decode_entry_key(&suffix)?)?,
                value: decode(&value)?,
            });
        }
        Ok(Page {
            items,
            next: next.map(Cursor),
        })
    }

    pub async fn set_page<K: DeserializeOwned>(
        &self,
        id: &CollectionId,
        cursor: Option<Cursor>,
        limit: usize,
    ) -> Result<Page<K>, ReadError> {
        let (rows, next) = self.entry_page(id, cursor.map(|cursor| cursor.0), limit).await?;
        let mut items = Vec::with_capacity(rows.len());
        for (suffix, _) in rows {
            items.push(decode(&CollectionKey::decode_entry_key(&suffix)?)?);
        }
        Ok(Page {
            items,
            next: next.map(Cursor),
        })
    }

    pub async fn feed_page<V: DeserializeOwned>(
        &self,
        id: &CollectionId,
        cursor: Option<Cursor>,
        limit: usize,
    ) -> Result<Page<FeedEntry<V>>, ReadError> {
        let (rows, next) = self.entry_page(id, cursor.map(|cursor| cursor.0), limit).await?;
        let mut items = Vec::with_capacity(rows.len());
        for (suffix, value) in rows {
            let seq = CollectionKey::decode_feed_seq(&suffix)?;
            items.push(FeedEntry {
                seq,
                value: decode(&value)?,
            });
        }
        Ok(Page {
            items,
            next: next.map(Cursor),
        })
    }

    async fn entry_page(
        &self,
        id: &CollectionId,
        cursor: Option<Vec<u8>>,
        limit: usize,
    ) -> Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<Vec<u8>>), ReadError> {
        if limit == 0 || limit > MAX_PAGE {
            return Err(ReadError::PageLimit);
        }
        let prefix = CollectionKey::entry_prefix(id);
        let mut start = prefix.clone();
        if let Some(cursor) = cursor {
            if cursor.is_empty() || cursor.len() > MAX_CURSOR {
                return Err(ReadError::Protocol);
            }
            start.extend_from_slice(&cursor);
            start.push(0x00);
        }
        let end = Prefix::successor(&prefix).ok_or(ReadError::Protocol)?;
        let rows = self
            .raw_range(Range::state(start, Some(end)), Some(limit as u64))
            .await?;
        let prefix_len = prefix.len();
        let rows: Vec<(Vec<u8>, Vec<u8>)> = rows
            .into_iter()
            .map(|(key, value)| {
                let suffix = if key.len() >= prefix_len {
                    key[prefix_len..].to_vec()
                } else {
                    Vec::new()
                };
                (suffix, value)
            })
            .collect();
        let next = if rows.len() == limit {
            rows.last().map(|(key, _)| key.clone())
        } else {
            None
        };
        Ok((rows, next))
    }

    async fn point(&self, user_key: &[u8]) -> Result<Option<Vec<u8>>, ReadError> {
        let values = self
            .session
            .get_raw(&[ReadKey::state(user_key.to_vec())])
            .await?;
        Ok(values.into_iter().next().flatten())
    }

    async fn counter_value(&self, user_key: &[u8]) -> Result<u64, ReadError> {
        match self.point(user_key).await? {
            None => Ok(0),
            Some(bytes) => decode(&bytes),
        }
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ReadError> {
    bcs::from_bytes(bytes).map_err(|_| ReadError::Entity(EntityError::ValueDecode))
}

fn record_error(error: RecordError) -> ReadError {
    match error {
        RecordError::TypeMismatch { expected, found } => {
            ReadError::Entity(EntityError::TypeMismatch { expected, found })
        }
        RecordError::Decode => ReadError::Entity(EntityError::Decode),
    }
}
