pub mod collections;
pub mod entry;
pub mod error;
pub mod ids;
#[cfg(not(target_arch = "wasm32"))]
pub mod reader;
pub mod record;
pub mod runtime;
pub mod schema;
pub mod state_keys;
pub mod storage;

pub use bcs;
pub use ech_db_entity_macros::{entity, entity_impl, BcsSchema};
pub use ech_db_protocol::values::{
    Attestation, AttestationBody, AttestedIntent, CallOutput, IntentRef, SignedIntent,
};

pub use collections::{CollectionSlot, Feed, Map, Set};
pub use error::{Error, Error as EntityError, Result};
pub use ids::{CollectionId, EntityId};
#[cfg(not(target_arch = "wasm32"))]
pub use reader::{Counter, Cursor, EntityReader, FeedEntry, MapEntry, Page, ReadError, MAX_PAGE};
pub use record::{EntityRecord, RecordError};
pub use state_keys::{CollectionKey, EntityKey};
pub use runtime::{
    bootstrap_root, feed_source, Entity, EntityContext, EntityHandlers, EntityMeta, EventLog, Guard,
};
pub use schema::{
    BcsSchema, EntitySchema, FieldKind, FieldSchema, MigrationKind, Schema,
    VariantSchema, VersionSchema,
};
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub use storage::host_store;
