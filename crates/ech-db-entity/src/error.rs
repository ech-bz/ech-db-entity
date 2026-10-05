use crate::ids::EntityId;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
pub enum Error {
    #[error("intent was already poisoned by an earlier error")]
    Poisoned,
    #[error("this value is not attached to the running intent")]
    NotAttached,
    #[error("entity {0:?} is already loaded")]
    AlreadyLoaded(EntityId),
    #[error("entity {0:?} already exists in storage")]
    AlreadyExists(EntityId),
    #[error("entity {0:?} is missing")]
    Missing(EntityId),
    #[error("entity type mismatch: expected {expected}, found {found}")]
    TypeMismatch { expected: String, found: String },
    #[error("entity state does not decode")]
    Decode,
    #[error("stored value does not decode")]
    ValueDecode,
    #[error("entity key changed after it was fixed")]
    KeyChanged,
    #[error("collection field address does not match its entity")]
    CollectionAddress,
    #[error("spawn requires a Genesis event")]
    ExpectedGenesis,
    #[error("Genesis event is not allowed here")]
    GenesisRejected,
    #[error("storage access is forbidden while running genesis")]
    StorageForbidden,
    #[error("collection has no address")]
    Unbound,
    #[error("collection has no write binding in this intent")]
    NotBound,
    #[error("unsupported entity version {0}")]
    UnsupportedVersion(u32),
    #[error("migration did not produce the next version")]
    MigrationInvalid,
    #[error("counter or sequence overflow")]
    Overflow,
    #[error("value does not encode")]
    Encode,
    #[error("nested operation is not allowed here")]
    Nested,
    #[error("call depth limit reached")]
    DepthExceeded,
    #[error("application rejected the operation")]
    Rejected,
    #[error("schema invariant violated: {0}")]
    Schema(String),
}

pub type Result<T> = core::result::Result<T, Error>;
