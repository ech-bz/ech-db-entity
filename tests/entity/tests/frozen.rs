use std::path::PathBuf;

use board_model::*;
use board_schema::corpus::Corpus;
use ech_db_entity::{CollectionId, EntityId, EntityMeta, EntityRecord, RecordError};
use serde::{Deserialize, Serialize};

fn frozen_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frozen")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn frozen_corpus_matches_current_schemas() {
    let problems = Corpus::verify(&frozen_dir());
    assert!(
        problems.is_empty(),
        "frozen corpus mismatch:\n{}\n\nfix: cargo run -p board-schema -- --append-frozen tests/entity/frozen",
        problems.join("\n")
    );
}

#[test]
fn frozen_bytes_decode_and_reencode() {
    for case in Corpus::frozen_cases(&frozen_dir()) {
        let reencoded = match case.schema.as_str() {
            "RootEntityRecord" => record_roundtrip::<RootEntity>(&case.bytes, "RootEntity"),
            "BoardEntityRecord" => record_roundtrip::<BoardEntity>(&case.bytes, "BoardEntity"),
            "ThreadEntityRecord" => record_roundtrip::<ThreadEntity>(&case.bytes, "ThreadEntity"),
            "PostEntityRecord" => record_roundtrip::<PostEntity>(&case.bytes, "PostEntity"),
            "RootEvent" => value_roundtrip::<RootEvent>(&case.bytes),
            "BoardEvent" => value_roundtrip::<BoardEvent>(&case.bytes),
            "ThreadEvent" => value_roundtrip::<ThreadEvent>(&case.bytes),
            "PostEvent" => value_roundtrip::<PostEvent>(&case.bytes),
            "CommandError" => value_roundtrip::<CommandError>(&case.bytes),
            "BoardView" => value_roundtrip::<BoardView>(&case.bytes),
            "ThreadView" => value_roundtrip::<ThreadView>(&case.bytes),
            "EntityId" => value_roundtrip::<EntityId>(&case.bytes),
            "CollectionId" => value_roundtrip::<CollectionId>(&case.bytes),
            other => panic!("unknown case schema {other}"),
        };
        assert_eq!(hex(&reencoded), hex(&case.bytes), "case {}", case.name);
    }
}

#[test]
fn old_code_fails_on_unknown_variant() {
    let v1 = Corpus::frozen_case(&frozen_dir(), "record_board_v1").bytes;
    let v3 = Corpus::frozen_case(&frozen_dir(), "record_board_v3").bytes;
    assert!(EntityRecord::<OldBoardEntity>::decode(&v1, "BoardEntity").is_ok());
    assert_eq!(
        EntityRecord::<OldBoardEntity>::decode(&v3, "BoardEntity").unwrap_err(),
        RecordError::Decode
    );
}

#[test]
fn new_code_reads_old_versions() {
    for name in ["record_board_v1", "record_board_v2", "record_board_v3"] {
        let bytes = Corpus::frozen_case(&frozen_dir(), name).bytes;
        let record = EntityRecord::<BoardEntity>::decode(&bytes, "BoardEntity").unwrap();
        assert!(record.state.version() >= 1);
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
enum OldBoardEntity {
    V1 {
        slug: String,
        title: String,
        admin: [u8; 32],
        threads: CollectionId,
    },
}

fn record_roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(
    bytes: &[u8],
    expected_type: &str,
) -> Vec<u8> {
    let record = EntityRecord::<T>::decode(bytes, expected_type).unwrap();
    assert_eq!(record.type_name, expected_type);
    ech_db_entity::record::EntityRecordRef {
        type_name: &record.type_name,
        parent: record.parent,
        state: &record.state,
    }
    .encode()
    .unwrap()
}

fn value_roundtrip<T: Serialize + serde::de::DeserializeOwned>(bytes: &[u8]) -> Vec<u8> {
    let value: T = bcs::from_bytes(bytes).unwrap();
    bcs::to_bytes(&value).unwrap()
}
