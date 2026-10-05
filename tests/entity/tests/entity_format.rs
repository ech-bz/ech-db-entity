use board_model::*;
use ech_db_entity::{
    bootstrap_root, host_store, runtime, EntityId, EntityKey, EntityMeta, EntityRecord,
    Error, Feed, Map, RecordError, Set,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fixture_root() -> [u8; 32] {
    [0x11u8; 32]
}

fn fixture_board() -> EntityId {
    let root = EntityId::root(&fixture_root());
    EntityId::child(root, "BoardEntity", &("rust",)).unwrap()
}

fn fixture_thread(board: EntityId) -> EntityId {
    EntityId::child(board, "ThreadEntity", &(board, 7u64)).unwrap()
}

fn begin(root: [u8; 32]) {
    host_store::reset();
    runtime::begin(root, [0x33u8; 32], 1);
}

fn bootstrap() -> ech_db_entity::Guard<RootEntity> {
    begin(fixture_root());
    bootstrap_root::<RootEntity>(&RootEvent::Genesis {
        name: "Root Board".to_string(),
        probe: false,
    })
    .unwrap()
}

fn create_board(
    root: &mut ech_db_entity::Guard<RootEntity>,
    slug: &str,
) -> ech_db_entity::Guard<BoardEntity> {
    root.spawn::<BoardEntity>(&BoardEvent::Genesis {
        slug: slug.to_string(),
        title: format!("Board {slug}"),
        admin: [0x22u8; 32],
        first_thread: None,
        corrupt_key: false,
    })
    .unwrap()
}

#[test]
fn id_and_key_vectors_are_stable() {
    let root = EntityId::root(&fixture_root());
    let board = fixture_board();
    let thread = fixture_thread(board);
    let post = EntityId::child(thread, "PostEntity", &(board, 42u64)).unwrap();
    assert_eq!(hex(&root.0), "6274ff2ffa5b489beba01746cd5321dc79a2b71d56868f822ffe6e0149278cce");
    assert_eq!(hex(&board.0), "835ab22d6d3991b6ae5cc7a982abe3ea5130e8ca18bcd8c8393da482b05ec44f");
    assert_eq!(hex(&thread.0), "d96d8a0d68f01a27d3568fafb5a633806bcabe7514b645fc82c73ed20add756a");
    assert_eq!(hex(&post.0), "98b2018ee900d84fe7515e71cb4d2b51369fb5de329976ba9be443119cdf5f78");

    let threads_id = board.collection("threads").unwrap();
    let moderators_id = board.collection("moderators").unwrap();
    let posts_id = thread.collection("posts").unwrap();
    assert_eq!(hex(threads_id.as_array()), "31397a73bf709853b82dca8d83352eafc114a0156c6152c5f5655ca3f47b6bfc");
    assert_eq!(hex(moderators_id.as_array()), "6f57ad8fc7bbd06caddd512fbd91c47c87880d25328a243997e7a90d9224e64e");
    assert_eq!(hex(posts_id.as_array()), "8eb9f81a6dd2fcd190b90aa62d83d5be095c2b325d4e7eb0eeb4f0ae8c83f56c");
    assert_eq!(hex(board.events().unwrap().as_array()), "02e1df8e443ea7571af7ed6c0817808ceab2fdcc9ea2ee78fb7ff6850c61ba69");

    assert_eq!(hex(&EntityKey::record(&board)), "02656e746974790001835ab22d6d3991b6ae5cc7a982abe3ea5130e8ca18bcd8c8393da482b05ec44f00");
    assert_eq!(
        hex(&ech_db_entity::CollectionKey::entry(&threads_id, &1u64).unwrap()),
        "02636f6c6c656374696f6e000131397a73bf709853b82dca8d83352eafc114a0156c6152c5f5655ca3f47b6bfc0002656e74727900010100ff00ff00ff00ff00ff00ff00ff00"
    );
    assert_eq!(
        hex(&ech_db_entity::CollectionKey::feed_entry(&posts_id, 1)),
        "02636f6c6c656374696f6e00018eb9f81a6dd2fcd190b90aa62d83d5be095c2b325d4e7eb0eeb4f0ae8c83f56c0002656e747279001501"
    );
    assert_eq!(
        hex(&ech_db_entity::CollectionKey::count(&threads_id)),
        "02636f6c6c656374696f6e000131397a73bf709853b82dca8d83352eafc114a0156c6152c5f5655ca3f47b6bfc0002636f756e7400"
    );
    assert_eq!(
        hex(&ech_db_entity::CollectionKey::length(&posts_id)),
        "02636f6c6c656374696f6e00018eb9f81a6dd2fcd190b90aa62d83d5be095c2b325d4e7eb0eeb4f0ae8c83f56c00026c656e67746800"
    );
}

#[test]
fn record_type_is_checked_before_state_decode() {
    let mut bytes = bcs::to_bytes("Zzz").unwrap();
    bytes.push(0);
    bytes.extend_from_slice(&[0xff, 0xfe, 0xfd]);
    match EntityRecord::<BoardEntity>::decode(&bytes, "BoardEntity") {
        Err(RecordError::TypeMismatch { expected, found }) => {
            assert_eq!(expected, "BoardEntity");
            assert_eq!(found, "Zzz");
        }
        _ => panic!("expected type mismatch"),
    }
    match EntityRecord::<BoardEntity>::decode(&bytes, "Zzz") {
        Err(RecordError::Decode) => {}
        _ => panic!("expected a decode error"),
    }
}

#[test]
fn map_distinguishes_missing_from_value_and_counts_once() {
    let mut root = bootstrap();
    let mut board = create_board(&mut root, "counts");
    let board_id = board.id().unwrap();
    let (threads, moderators) = match &mut *board {
        BoardEntity::V3 {
            threads,
            moderators,
            ..
        } => (threads, moderators),
        other => panic!("expected V3, got version {}", other.version()),
    };

    assert!(threads.get(&1).unwrap().is_none());
    assert_eq!(threads.count().unwrap(), 0);

    let first = EntityId::root(&[0x99u8; 32]);
    let reads = host_store::reads();
    threads.set(&1, &first).unwrap();
    assert_eq!(host_store::reads() - reads, 2, "entry read + count read");
    assert_eq!(threads.count().unwrap(), 1);

    threads.set(&1, &EntityId::root(&[0x88u8; 32])).unwrap();
    assert_eq!(threads.count().unwrap(), 1);
    assert_eq!(threads.get(&1).unwrap(), Some(EntityId::root(&[0x88u8; 32])));

    threads.set(&2, &first).unwrap();
    assert_eq!(threads.count().unwrap(), 2);

    let reads = host_store::reads();
    assert_eq!(threads.count().unwrap(), 2);
    assert_eq!(host_store::reads() - reads, 1, "count is a single read");

    assert!(moderators.has(&[0x22u8; 32]).unwrap());
    let moderators_id = board_id.collection("moderators").unwrap();
    let entry = ech_db_entity::CollectionKey::entry(&moderators_id, &[0x22u8; 32]).unwrap();
    assert_eq!(host_store::peek(&entry), Some(Vec::new()), "unit encodes empty");
}

#[test]
fn repeated_insert_in_one_transaction_counts_once() {
    let mut root = bootstrap();
    let mut board = create_board(&mut root, "repeat");
    let threads = match &mut *board {
        BoardEntity::V3 { threads, .. } => threads,
        other => panic!("expected V3, got version {}", other.version()),
    };
    let value = EntityId::root(&[0x77u8; 32]);
    threads.set(&5, &value).unwrap();
    threads.set(&5, &value).unwrap();
    threads.set(&5, &value).unwrap();
    assert_eq!(threads.count().unwrap(), 1);
}

#[test]
fn feed_starts_at_one_and_rejects_overflow() {
    let mut root = bootstrap();
    let mut board = create_board(&mut root, "feed");
    let board_id = board.id().unwrap();
    let mut thread = board
        .spawn::<ThreadEntity>(&ThreadEvent::Genesis {
            board: board_id,
            num: 1,
            title: "Thread".to_string(),
        })
        .unwrap();
    let thread_id = thread.id().unwrap();
    let posts_id = thread_id.collection("posts").unwrap();
    thread.apply(&ThreadEvent::PostAdded { post: 10 }).unwrap();
    let posts = match &mut *thread {
        ThreadEntity::V1 { posts, .. } => posts,
    };
    assert_eq!(posts.len().unwrap(), 1);
    assert_eq!(posts.get(1).unwrap(), Some(10));
    assert_eq!(posts.get(0).unwrap(), None);
    assert_eq!(posts.get(2).unwrap(), None);

    host_store::put(
        &ech_db_entity::CollectionKey::length(&posts_id),
        &bcs::to_bytes(&u64::MAX).unwrap(),
    );
    assert_eq!(posts.append(&10).unwrap_err(), Error::Overflow);
    let _ = &posts;
    assert!(
        runtime::finish(Ok(vec![])).is_err(),
        "a failed append poisons the intent"
    );
}

#[test]
fn events_journal_is_read_only() {
    let mut root = bootstrap();
    let board = create_board(&mut root, "journal");
    let board_id = board.id().unwrap();
    let log = board.events().unwrap();
    assert_eq!(log.len().unwrap(), 1);
    assert_eq!(
        log.get(1).unwrap(),
        Some(BoardEvent::Genesis {
            slug: "journal".to_string(),
            title: "Board journal".to_string(),
            admin: [0x22u8; 32],
            first_thread: None,
            corrupt_key: false,
        })
    );

    let events_id = board_id.events().unwrap();
    let mut feed: Feed<BoardEvent> = bcs::from_bytes(&bcs::to_bytes(&events_id).unwrap()).unwrap();
    assert_eq!(feed.append(&BoardEvent::Ping).unwrap_err(), Error::NotBound);
    assert!(runtime::finish(Ok(vec![])).is_err());
}

#[test]
fn unbound_collections_cannot_be_used() {
    begin(fixture_root());
    let mut map: Map<u64, EntityId> = Map::default();
    assert_eq!(map.get(&1).unwrap_err(), Error::Unbound);
    assert_eq!(map.count().unwrap_err(), Error::Unbound);
    let set: Set<u64> = Set::default();
    assert_eq!(set.has(&1).unwrap_err(), Error::Unbound);
    let mut feed: Feed<u64> = Feed::default();
    assert_eq!(feed.len().unwrap_err(), Error::Unbound);
    assert_eq!(map.set(&1, &EntityId::root(&[1u8; 32])).unwrap_err(), Error::Unbound);
    assert_eq!(feed.append(&1).unwrap_err(), Error::Poisoned);
    assert!(runtime::finish(Ok(vec![])).is_err());
}

#[test]
fn board_wasm_passes_server_validation() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/wasm32-unknown-unknown/release/board_program.wasm"
    );
    let bytes = std::fs::read(path).unwrap();
    let engine = ech_db_server::wasm::Profile::engine();
    let module = wasmi::Module::new(&engine, &bytes).unwrap();
    ech_db_server::abi::Abi::check(&module).unwrap();
    ech_db_server::abi::Abi::check_profile_shape(&bytes).unwrap();
}

#[test]
fn poison_blocks_following_writes() {
    let mut root = bootstrap();
    let mut board = create_board(&mut root, "poison");
    let result = board.apply(&BoardEvent::Reject);
    assert_eq!(result.unwrap_err(), Error::Rejected);
    let again = board.apply(&BoardEvent::Ping);
    assert_eq!(again.unwrap_err(), Error::Poisoned);
    assert!(runtime::finish(Ok(vec![])).is_err());
}

#[test]
fn genesis_storage_access_is_rejected() {
    begin(fixture_root());
    let outcome = bootstrap_root::<RootEntity>(&RootEvent::Genesis {
        name: "probe".to_string(),
        probe: true,
    });
    match outcome {
        Err(Error::StorageForbidden) => {}
        _ => panic!("expected storage forbidden"),
    }
    assert!(runtime::finish(Ok(vec![])).is_err());
}
