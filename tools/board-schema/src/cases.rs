use board_model::{
    BoardEntity, BoardEvent, BoardView, CommandError, FirstThread, PostEntity, PostEvent, RootEntity,
    RootEvent, ThreadEntity, ThreadEvent, ThreadView,
};
use ech_db_entity::{CollectionId, EntityId, EntityRecord};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

pub struct Case {
    pub name: &'static str,
    pub schema: &'static str,
    pub hex: String,
    pub json: Value,
}

pub struct Cases;

impl Cases {
    pub fn all() -> Vec<Case> {
        let application_root = [0x11u8; 32];
        let root = EntityId::root(&application_root);
        let board = EntityId::child(root, "BoardEntity", &("rust",)).unwrap();
        let thread = EntityId::child(board, "ThreadEntity", &(board, 7u64)).unwrap();
        let admin = [0x22u8; 32];

        let threads_id = board.collection("threads").unwrap();
        let moderators_id = board.collection("moderators").unwrap();
        let posts_id = thread.collection("posts").unwrap();

        let mut cases = Vec::new();

        cases.push(case(
            "record_root_v1",
            "RootEntityRecord",
            &EntityRecord {
                type_name: "RootEntity".to_string(),
                parent: None,
                state: RootEntity::V1 {
                    name: "Root Board".to_string(),
                },
            },
            json!({
                "type_name": "RootEntity",
                "parent": null,
                "state": { "$kind": "V1", "V1": { "name": "Root Board" } },
            }),
        ));

        cases.push(case(
            "record_board_v1",
            "BoardEntityRecord",
            &EntityRecord {
                type_name: "BoardEntity".to_string(),
                parent: Some(root),
                state: BoardEntity::V1 {
                    slug: "rust".to_string(),
                    title: "Rust Board".to_string(),
                    admin,
                    threads: bound::<ech_db_entity::Map<u64, EntityId>>(&threads_id),
                },
            },
            json!({
                "type_name": "BoardEntity",
                "parent": root,
                "state": {
                    "$kind": "V1",
                    "V1": { "slug": "rust", "title": "Rust Board", "admin": admin, "threads": threads_id },
                },
            }),
        ));

        cases.push(case(
            "record_board_v2",
            "BoardEntityRecord",
            &EntityRecord {
                type_name: "BoardEntity".to_string(),
                parent: Some(root),
                state: BoardEntity::V2 {
                    slug: "rust".to_string(),
                    title: "Rust Board".to_string(),
                    admin,
                    threads: bound::<ech_db_entity::Map<u64, EntityId>>(&threads_id),
                    locked: true,
                },
            },
            json!({
                "type_name": "BoardEntity",
                "parent": root,
                "state": {
                    "$kind": "V2",
                    "V2": { "slug": "rust", "title": "Rust Board", "admin": admin, "threads": threads_id, "locked": true },
                },
            }),
        ));

        cases.push(case(
            "record_board_v3",
            "BoardEntityRecord",
            &EntityRecord {
                type_name: "BoardEntity".to_string(),
                parent: Some(root),
                state: BoardEntity::V3 {
                    slug: "rust".to_string(),
                    title: "Rust Board".to_string(),
                    threads: bound::<ech_db_entity::Map<u64, EntityId>>(&threads_id),
                    locked: false,
                    moderators: bound::<ech_db_entity::Set<[u8; 32]>>(&moderators_id),
                },
            },
            json!({
                "type_name": "BoardEntity",
                "parent": root,
                "state": {
                    "$kind": "V3",
                    "V3": { "slug": "rust", "title": "Rust Board", "threads": threads_id, "locked": false, "moderators": moderators_id },
                },
            }),
        ));

        cases.push(case(
            "record_thread_v1",
            "ThreadEntityRecord",
            &EntityRecord {
                type_name: "ThreadEntity".to_string(),
                parent: Some(board),
                state: ThreadEntity::V1 {
                    board,
                    num: 7,
                    title: "Intro".to_string(),
                    posts: bound::<ech_db_entity::Feed<u64>>(&posts_id),
                },
            },
            json!({
                "type_name": "ThreadEntity",
                "parent": board,
                "state": {
                    "$kind": "V1",
                    "V1": { "board": board, "num": "7", "title": "Intro", "posts": posts_id },
                },
            }),
        ));

        cases.push(case(
            "record_post_v1",
            "PostEntityRecord",
            &EntityRecord {
                type_name: "PostEntity".to_string(),
                parent: Some(thread),
                state: PostEntity::V1 {
                    board,
                    num: 42,
                    text: "hello".to_string(),
                },
            },
            json!({
                "type_name": "PostEntity",
                "parent": thread,
                "state": {
                    "$kind": "V1",
                    "V1": { "board": board, "num": "42", "text": "hello" },
                },
            }),
        ));

        cases.push(case(
            "event_root_genesis",
            "RootEvent",
            &RootEvent::Genesis {
                name: "Root Board".to_string(),
                probe: false,
            },
            json!({ "$kind": "Genesis", "Genesis": { "name": "Root Board", "probe": false } }),
        ));

        cases.push(case(
            "event_board_genesis",
            "BoardEvent",
            &BoardEvent::Genesis {
                slug: "rust".to_string(),
                title: "Rust Board".to_string(),
                admin,
                first_thread: Some(FirstThread {
                    num: 1,
                    title: "Intro".to_string(),
                }),
                corrupt_key: false,
            },
            json!({
                "$kind": "Genesis",
                "Genesis": {
                    "slug": "rust",
                    "title": "Rust Board",
                    "admin": admin,
                    "first_thread": { "num": "1", "title": "Intro" },
                    "corrupt_key": false,
                },
            }),
        ));

        cases.push(case(
            "event_board_locked",
            "BoardEvent",
            &BoardEvent::Locked,
            json!({ "$kind": "Locked", "Locked": true }),
        ));

        cases.push(case(
            "event_thread_genesis",
            "ThreadEvent",
            &ThreadEvent::Genesis {
                board,
                num: 7,
                title: "Intro".to_string(),
            },
            json!({ "$kind": "Genesis", "Genesis": { "board": board, "num": "7", "title": "Intro" } }),
        ));

        cases.push(case(
            "event_post_genesis",
            "PostEvent",
            &PostEvent::Genesis {
                board,
                num: 42,
                text: "hello".to_string(),
            },
            json!({ "$kind": "Genesis", "Genesis": { "board": board, "num": "42", "text": "hello" } }),
        ));

        cases.push(case(
            "error_command",
            "CommandError",
            &CommandError::Entity {
                code: "already-loaded".to_string(),
                message: "entity is already loaded".to_string(),
            },
            json!({ "$kind": "Entity", "Entity": { "code": "already-loaded", "message": "entity is already loaded" } }),
        ));

        cases.push(case(
            "view_board",
            "BoardView",
            &BoardView {
                version: 3,
                title: "Rust Board".to_string(),
                slug: "rust".to_string(),
                locked: Some(false),
                threads: 2,
                moderators: 1,
                events: 4,
            },
            json!({
                "version": 3,
                "title": "Rust Board",
                "slug": "rust",
                "locked": false,
                "threads": "2",
                "moderators": "1",
                "events": "4",
            }),
        ));

        cases.push(case(
            "view_thread",
            "ThreadView",
            &ThreadView {
                num: 7,
                title: "Intro".to_string(),
                posts: 1,
                events: 2,
            },
            json!({ "num": "7", "title": "Intro", "posts": "1", "events": "2" }),
        ));

        cases.push(case(
            "entity_id",
            "EntityId",
            &EntityId([0x11u8; 32]),
            json!(EntityId([0x11u8; 32])),
        ));
        cases.push(case(
            "collection_id",
            "CollectionId",
            &threads_id,
            json!(threads_id),
        ));

        cases
    }

    pub fn fixture() -> String {
        let entries: Vec<serde_json::Value> = Self::all()
            .into_iter()
            .map(|case| {
                serde_json::json!({
                    "name": case.name,
                    "schema": case.schema,
                    "hex": case.hex,
                    "json": case.json,
                })
            })
            .collect();
        serde_json::to_string_pretty(&entries).unwrap() + "\n"
    }
}

fn bound<T: DeserializeOwned>(id: &CollectionId) -> T {
    bcs::from_bytes(&bcs::to_bytes(id).unwrap()).unwrap()
}

fn encode<T: Serialize>(value: &T) -> String {
    hex::encode(bcs::to_bytes(value).unwrap())
}

fn case<T: Serialize>(
    name: &'static str,
    schema: &'static str,
    value: &T,
    json: Value,
) -> Case {
    Case {
        name,
        schema,
        hex: encode(value),
        json,
    }
}
