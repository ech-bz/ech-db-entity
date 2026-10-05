use serde::{Deserialize, Serialize};

use crate::ids::{CollectionId, EntityId};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum Schema {
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    Bool,
    String,
    Vector(Box<Schema>),
    FixedArray { len: u64, item: Box<Schema> },
    Option(Box<Schema>),
    Tuple(Vec<Schema>),
    Struct {
        name: String,
        fields: Vec<(String, Schema)>,
    },
    Enum {
        name: String,
        variants: Vec<(String, VariantSchema)>,
    },
    Newtype {
        name: String,
        inner: Box<Schema>,
    },
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum VariantSchema {
    Unit,
    Fields(Vec<(String, Schema)>),
}

pub trait BcsSchema {
    fn schema() -> Schema;
}

macro_rules! scalar_schema {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(
            impl BcsSchema for $ty {
                fn schema() -> Schema {
                    Schema::$variant
                }
            }
        )*
    };
}

scalar_schema!(
    u8 => U8,
    u16 => U16,
    u32 => U32,
    u64 => U64,
    u128 => U128,
    i8 => I8,
    i16 => I16,
    i32 => I32,
    i64 => I64,
    i128 => I128,
    bool => Bool,
    String => String,
);

impl BcsSchema for () {
    fn schema() -> Schema {
        Schema::Tuple(Vec::new())
    }
}

impl<T: BcsSchema> BcsSchema for Vec<T> {
    fn schema() -> Schema {
        Schema::Vector(Box::new(T::schema()))
    }
}

impl<T: BcsSchema> BcsSchema for Option<T> {
    fn schema() -> Schema {
        Schema::Option(Box::new(T::schema()))
    }
}

impl<T: BcsSchema, const N: usize> BcsSchema for [T; N] {
    fn schema() -> Schema {
        Schema::FixedArray {
            len: N as u64,
            item: Box::new(T::schema()),
        }
    }
}

macro_rules! tuple_schema {
    ($($name:ident),+) => {
        impl<$($name: BcsSchema),+> BcsSchema for ($($name,)+) {
            fn schema() -> Schema {
                Schema::Tuple(vec![$($name::schema()),+])
            }
        }
    };
}

tuple_schema!(A);
tuple_schema!(A, B);
tuple_schema!(A, B, C);
tuple_schema!(A, B, C, D);
tuple_schema!(A, B, C, D, E);
tuple_schema!(A, B, C, D, E, F);

impl BcsSchema for EntityId {
    fn schema() -> Schema {
        Schema::Newtype {
            name: "EntityId".to_string(),
            inner: Box::new(Schema::FixedArray {
                len: 32,
                item: Box::new(Schema::U8),
            }),
        }
    }
}

impl BcsSchema for CollectionId {
    fn schema() -> Schema {
        Schema::Newtype {
            name: "CollectionId".to_string(),
            inner: Box::new(Schema::FixedArray {
                len: 32,
                item: Box::new(Schema::U8),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    Plain,
    Map,
    Set,
    Feed,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MigrationKind {
    Initial,
    Auto,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct FieldSchema {
    pub name: String,
    pub kind: FieldKind,
    pub ty: String,
    pub bcs: Schema,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct VersionSchema {
    pub version: u32,
    pub fields: Vec<FieldSchema>,
    pub migration: MigrationKind,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct EntitySchema {
    pub name: String,
    pub key: Vec<String>,
    pub versions: Vec<VersionSchema>,
}
