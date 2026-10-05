use std::cell::Cell;
use std::rc::Rc;

use board_model::*;
use ed25519_dalek::SigningKey;
use serde::de::DeserializeOwned;
use serde::Serialize;

use ech_db_entity::{EntityId, EntityKey, EntityMeta, EntityReader, EntityRecord};
use ech_db_protocol::errors::{DbError, DbErrorCode};
use ech_db_protocol::keys::{Prefix, StateKey};
use ech_db_protocol::values::{IntentBody, Invocation, SignedIntent};
use ech_db_test_fixtures::harness::{Harness, TestEnv};
use ech_db_entity_tests::bundles::Bundles;

fn env() -> TestEnv {
    TestEnv::from_env()
}

fn signing() -> SigningKey {
    SigningKey::from_bytes(&rand::random())
}

fn root_of(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

fn intent_for(
    root: [u8; 32],
    key: &SigningKey,
    seq: u64,
    program_hash: [u8; 32],
    function: &str,
    args: Vec<u8>,
) -> SignedIntent {
    let body = IntentBody {
        version: IntentBody::FORMAT_VERSION,
        root,
        author: root_of(key),
        tweak: [0u8; 32],
        expected_seq: seq,
        call: Invocation {
            program_hash,
            function: function.to_string(),
            args,
        },
    };
    SignedIntent::sign(body, key).unwrap()
}

struct Run<'a> {
    harness: &'a Harness,
    key: SigningKey,
    root: [u8; 32],
    attestor: SigningKey,
    program: [u8; 32],
    seq: Rc<Cell<u64>>,
}

impl<'a> Run<'a> {
    fn new(harness: &'a Harness, key: SigningKey, program: [u8; 32]) -> Self {
        Self {
            harness,
            root: root_of(&key),
            attestor: key.clone(),
            key,
            program,
            seq: Rc::new(Cell::new(0)),
        }
    }

    fn sibling(&self, key: SigningKey, program: [u8; 32]) -> Self {
        Self {
            harness: self.harness,
            root: self.root,
            attestor: self.attestor.clone(),
            key,
            program,
            seq: Rc::clone(&self.seq),
        }
    }

    fn delegate(&self, key: SigningKey) -> Self {
        Self {
            harness: self.harness,
            root: self.root,
            attestor: self.attestor.clone(),
            key,
            program: self.program,
            seq: Rc::new(Cell::new(0)),
        }
    }

    async fn call(&mut self, function: &str, args: &impl Serialize) -> Result<Vec<u8>, DbError> {
        let args = bcs::to_bytes(args).unwrap();
        let next = self.seq.get() + 1;
        let intent = intent_for(self.root, &self.key, next, self.program, function, args);
        let outcome = self.harness.append(self.root, intent, &self.attestor).await;
        if outcome.is_ok() {
            self.seq.set(next);
        }
        outcome
    }

    async fn ok<T: DeserializeOwned>(&mut self, function: &str, args: &impl Serialize) -> T {
        let bytes = self.call(function, args).await.unwrap();
        bcs::from_bytes(&bytes).unwrap()
    }

    async fn fail(&mut self, function: &str, args: &impl Serialize) -> CommandError {
        let error = self.call(function, args).await.unwrap_err();
        application_error(&error)
    }

    async fn retry<T: DeserializeOwned>(&mut self, function: &str, args: &impl Serialize) -> T {
        loop {
            match self.call(function, args).await {
                Ok(bytes) => return bcs::from_bytes(&bytes).unwrap(),
                Err(error) if error.is_retryable() => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => panic!("terminal error: {error:?}"),
            }
        }
    }
}

fn application_error(error: &DbError) -> CommandError {
    match &error.code {
        DbErrorCode::Application { bytes } => bcs::from_bytes(bytes).unwrap(),
        other => panic!("expected application error, got {other:?}"),
    }
}

fn application_text(error: &DbError) -> String {
    match &error.code {
        DbErrorCode::Application { bytes } => String::from_utf8(bytes.clone()).unwrap(),
        other => panic!("expected application error, got {other:?}"),
    }
}

fn bootstrap_args(name: &str, probe: bool) -> BootstrapArgs {
    BootstrapArgs {
        name: name.to_string(),
        probe,
    }
}

fn board_args(slug: &str, title: &str, admin: [u8; 32]) -> CreateBoardArgs {
    CreateBoardArgs {
        slug: slug.to_string(),
        title: title.to_string(),
        admin,
        first_thread: None,
        corrupt_key: false,
    }
}

fn thread_id(board: EntityId, num: u64) -> EntityId {
    EntityId::child(board, "ThreadEntity", &(board, num)).unwrap()
}

async fn raw_record(harness: &Harness, root: [u8; 32], id: EntityId) -> Option<Vec<u8>> {
    let trx = harness.server.fdb.trx().unwrap();
    trx.get_raw(&StateKey::physical(&root, &EntityKey::record(&id)))
        .await
        .unwrap()
}

async fn snapshot(harness: &Harness, root: [u8; 32]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let trx = harness.server.fdb.trx().unwrap();
    let mut rows = Vec::new();
    for namespace in ["log", "state"] {
        let prefix = foundationdb_tuple::pack(&(
            foundationdb_tuple::Bytes::from(root.as_slice()),
            namespace,
        ));
        let end = Prefix::successor(&prefix).unwrap();
        rows.extend(trx.range_raw(&prefix, &end).await.unwrap());
    }
    rows.sort();
    rows
}

async fn assert_unchanged(harness: &Harness, root: [u8; 32], baseline: &[(Vec<u8>, Vec<u8>)]) {
    let after = snapshot(harness, root).await;
    if &after == baseline {
        return;
    }
    let before: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = baseline.iter().cloned().collect();
    let now: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = after.iter().cloned().collect();
    for (key, value) in &now {
        match before.get(key) {
            None => eprintln!(
                "ADDED   {} = {}",
                hex::encode(key),
                hex::encode(&value[..value.len().min(24)])
            ),
            Some(old) if old != value => eprintln!("CHANGED {}", hex::encode(key)),
            _ => {}
        }
    }
    for key in before.keys() {
        if !now.contains_key(key) {
            eprintln!("REMOVED {}", hex::encode(key));
        }
    }
    panic!("intent leaked state changes");
}

async fn upload_board_programs(harness: &Harness, root: [u8; 32]) -> ([u8; 32], [u8; 32]) {
    let legacy = harness
        .server
        .upload_program(root, Bundles::board_legacy())
        .await
        .unwrap();
    let latest = harness
        .server
        .upload_program(root, Bundles::board())
        .await
        .unwrap();
    assert_ne!(legacy, latest);
    (legacy, latest)
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn entity_lifecycle_and_journal() {
    let harness = Harness::start(env()).await;
    let mut harness = harness;
    harness.start_grpc().await;
    let key = signing();
    let root = root_of(&key);
    let (_, program) = upload_board_programs(&harness, root).await;
    let mut run = Run::new(&harness, key.clone(), program);

    run.ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = run
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "rust".to_string(),
                title: "Rust Board".to_string(),
                admin: [0x22u8; 32],
                first_thread: Some(FirstThread {
                    num: 7,
                    title: "Intro".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();

    let first: Inspection = run
        .ok(
            "inspect",
            &InspectArgs {
                board,
                thread: Some(thread),
            },
        )
        .await;
    assert_eq!(first.board.version, 3);
    assert_eq!(first.board.threads, 1);
    assert_eq!(first.board.moderators, 1);
    assert_eq!(first.board.events, 1);
    assert_eq!(first.thread.as_ref().unwrap().posts, 0);
    assert_eq!(first.thread.as_ref().unwrap().events, 1);

    run.ok::<()>(
        "rename_board",
        &RenameArgs {
            board,
            title: "Renamed".to_string(),
        },
    )
    .await;
    run.ok::<()>(
        "recursive",
        &RepeatArgs {
            board,
            remaining: 2,
            corrupt_at: None,
        },
    )
    .await;
    run.ok::<()>(
        "submit_post",
        &SubmitArgs {
            board,
            thread,
            post: 42,
            text: "hello".to_string(),
        },
    )
    .await;
    run.ok::<()>("move_probe", &BoardIdArgs { board }).await;

    let second: Inspection = run
        .ok(
            "inspect",
            &InspectArgs {
                board,
                thread: Some(thread),
            },
        )
        .await;
    assert_eq!(second.board.events, 7, "one event per successful apply");
    assert_eq!(second.thread.as_ref().unwrap().posts, 1);
    assert_eq!(second.thread.as_ref().unwrap().events, 2);

    let post = EntityId::child(thread, "PostEntity", &(board, 42u64)).unwrap();
    let record: EntityRecord<PostEntity> = EntityRecord::decode(
        &raw_record(&harness, root, post).await.unwrap(),
        "PostEntity",
    )
    .unwrap();
    match &record.state {
        PostEntity::V1 {
            board: b,
            num,
            text,
        } => {
            assert_eq!(*b, board);
            assert_eq!(*num, 42);
            assert_eq!(text, "hello");
        }
    }
    assert_eq!(record.parent, Some(thread));

    let client = harness.client(key.to_bytes());
    let events = EntityReader::read(&client, |reader| {
        let board = board;
        async move { read_events(&reader, board).await }
    })
    .await
    .unwrap();
    assert_eq!(
        events,
        vec![
            BoardEvent::Genesis {
                slug: "rust".to_string(),
                title: "Rust Board".to_string(),
                admin: [0x22u8; 32],
                first_thread: Some(FirstThread {
                    num: 7,
                    title: "Intro".to_string(),
                }),
                corrupt_key: false,
            },
            BoardEvent::Renamed {
                title: "Renamed".to_string(),
            },
            BoardEvent::Recursive {
                remaining: 0,
                corrupt_at: None,
            },
            BoardEvent::Recursive {
                remaining: 1,
                corrupt_at: None,
            },
            BoardEvent::Recursive {
                remaining: 2,
                corrupt_at: None,
            },
            BoardEvent::PostSubmitted {
                thread,
                post: 42,
                text: "hello".to_string(),
            },
            BoardEvent::Ping,
        ]
    );

    let thread_events = EntityReader::read(&client, |reader| {
        let thread = thread;
        async move { read_thread_events(&reader, thread).await }
    })
    .await
    .unwrap();
    assert_eq!(
        thread_events,
        vec![
            ThreadEvent::Genesis {
                board,
                num: 7,
                title: "Intro".to_string(),
            },
            ThreadEvent::PostAdded { post: 42 },
        ]
    );
}

async fn read_events(
    reader: &EntityReader,
    board: EntityId,
) -> Result<Vec<BoardEvent>, ech_db_entity::ReadError> {
    let events = board.events().unwrap();
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.feed_page::<BoardEvent>(&events, cursor, 4).await?;
        items.extend(page.items.into_iter().map(|entry| entry.value));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(items)
}

async fn read_thread_events(
    reader: &EntityReader,
    thread: EntityId,
) -> Result<Vec<ThreadEvent>, ech_db_entity::ReadError> {
    let events = thread.events().unwrap();
    let mut items = Vec::new();
    let mut seq = 1;
    while let Some(value) = reader.feed_entry::<ThreadEvent>(&events, seq).await? {
        items.push(value);
        seq += 1;
    }
    Ok(items)
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn ownership_creation_and_tampering_rules() {
    let harness = Harness::start(env()).await;
    let key = signing();
    let root = root_of(&key);
    let (_, program) = upload_board_programs(&harness, root).await;
    let mut run = Run::new(&harness, key.clone(), program);

    run.ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = run
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "rules".to_string(),
                title: "Rules".to_string(),
                admin: [0x22u8; 32],
                first_thread: Some(FirstThread {
                    num: 1,
                    title: "First".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();

    let baseline = snapshot(&harness, root).await;

    assert_eq!(
        run.fail("double_load", &BoardIdArgs { board }).await,
        CommandError::Entity {
            code: "already-loaded".to_string(),
            message: "entity EntityId(".to_string() + &hex::encode(board.0) + ") is already loaded",
        }
    );
    assert_unchanged(&harness, root, &baseline).await;

    let reloaded: EntityId = run.ok("drop_reload", &BoardIdArgs { board }).await;
    assert_eq!(reloaded, board);
    let baseline = snapshot(&harness, root).await;

    assert_eq!(
        run.fail("wrong_type_load", &BoardIdArgs { board }).await,
        CommandError::Entity {
            code: "type-mismatch".to_string(),
            message: format!("entity type mismatch: expected ThreadEntity, found BoardEntity"),
        }
    );
    assert_unchanged(&harness, root, &baseline).await;

    assert_eq!(
        run.fail("nested_reload", &NestedArgs { board, thread })
            .await,
        CommandError::Entity {
            code: "already-loaded".to_string(),
            message: "entity EntityId(".to_string() + &hex::encode(board.0) + ") is already loaded",
        }
    );
    assert_eq!(
        run.fail("detached_apply", &BoardIdArgs { board }).await,
        CommandError::Entity {
            code: "not-attached".to_string(),
            message: "this value is not attached to the running intent".to_string(),
        }
    );
    assert_eq!(
        run.fail(
            "spawn_duplicate",
            &board_args("rules", "Rules", [0x22u8; 32])
        )
        .await,
        CommandError::Entity {
            code: "already-exists".to_string(),
            message: "entity EntityId(".to_string()
                + &hex::encode(board.0)
                + ") already exists in storage",
        }
    );
    assert_eq!(
        run.fail("bootstrap_again", &bootstrap_args("root", false))
            .await,
        CommandError::Entity {
            code: "already-exists".to_string(),
            message: "entity EntityId(".to_string()
                + &hex::encode(EntityId::root(&root).0)
                + ") already exists in storage",
        }
    );
    assert_eq!(
        run.fail(
            "create_board",
            &CreateBoardArgs {
                slug: "tamper".to_string(),
                title: "Tamper".to_string(),
                admin: [0x22u8; 32],
                first_thread: None,
                corrupt_key: true,
            }
        )
        .await,
        CommandError::Entity {
            code: "key-changed".to_string(),
            message: "entity key changed after it was fixed".to_string(),
        }
    );
    assert_eq!(
        run.fail("key_change", &BoardIdArgs { board }).await,
        CommandError::Entity {
            code: "key-changed".to_string(),
            message: "entity key changed after it was fixed".to_string(),
        }
    );
    assert_eq!(
        run.fail("collection_swap", &BoardIdArgs { board }).await,
        CommandError::Entity {
            code: "collection-address".to_string(),
            message: "collection field address does not match its entity".to_string(),
        }
    );
    assert_eq!(
        run.fail("events_append_probe", &BoardIdArgs { board })
            .await,
        CommandError::Entity {
            code: "not-bound".to_string(),
            message: "collection has no write binding in this intent".to_string(),
        }
    );
    assert_unchanged(&harness, root, &baseline).await;

    run.ok::<()>("move_probe", &BoardIdArgs { board }).await;

    let probe_key = signing();
    let probe_root = root_of(&probe_key);
    let (_, probe_program) = upload_board_programs(&harness, probe_root).await;
    let mut probe = Run::new(&harness, probe_key, probe_program);
    assert_eq!(
        probe
            .fail("bootstrap", &bootstrap_args("probe", true))
            .await,
        CommandError::Entity {
            code: "storage-forbidden".to_string(),
            message: "storage access is forbidden while running genesis".to_string(),
        }
    );
    assert_eq!(
        raw_record(&harness, probe_root, EntityId::root(&probe_root)).await,
        None
    );
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn poison_cancels_the_intent() {
    let harness = Harness::start(env()).await;
    let key = signing();
    let root = root_of(&key);
    let (_, program) = upload_board_programs(&harness, root).await;
    let mut run = Run::new(&harness, key.clone(), program);

    run.ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = run
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "poison".to_string(),
                title: "Poison".to_string(),
                admin: [0x22u8; 32],
                first_thread: Some(FirstThread {
                    num: 1,
                    title: "First".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();

    let baseline = snapshot(&harness, root).await;

    let error = run
        .call("reject_caught", &BoardIdArgs { board })
        .await
        .unwrap_err();
    let text = application_text(&error);
    assert!(
        text.contains("poisoned"),
        "caught mutation errors must still cancel the intent: {text}"
    );
    assert_unchanged(&harness, root, &baseline).await;

    let error = run.call("detached_write_caught", &()).await.unwrap_err();
    assert!(application_text(&error).contains("poisoned"));
    assert_unchanged(&harness, root, &baseline).await;

    let error = run
        .call(
            "post_then_reject",
            &SubmitArgs {
                board,
                thread,
                post: 9,
                text: "doomed".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        application_error(&error),
        CommandError::Entity {
            code: "rejected".to_string(),
            message: "application rejected the operation".to_string(),
        }
    );
    assert_unchanged(&harness, root, &baseline).await;
    let post = EntityId::child(thread, "PostEntity", &(board, 9u64)).unwrap();
    assert_eq!(raw_record(&harness, root, post).await, None);
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn concurrent_inserts_keep_counts_correct() {
    let harness = Harness::start(env()).await;
    let author = signing();
    let root = root_of(&author);
    let (_, program) = upload_board_programs(&harness, root).await;
    let mut owner = Run::new(&harness, author, program);

    owner
        .ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = owner
        .ok(
            "create_board",
            &board_args("conc", "Concurrent", [0x22u8; 32]),
        )
        .await;
    let board = created.board;

    let key_b = signing();
    let key_c = signing();
    let mut b = owner.delegate(key_b);
    let mut c = owner.delegate(key_c);

    let args_b = CreateThreadArgs {
        board,
        num: 100,
        title: "B".to_string(),
    };
    let args_c = CreateThreadArgs {
        board,
        num: 101,
        title: "C".to_string(),
    };
    let (thread_b, thread_c) = tokio::join!(
        b.retry::<EntityId>("create_thread", &args_b),
        c.retry::<EntityId>("create_thread", &args_c),
    );
    assert_eq!(thread_b, thread_id(board, 100));
    assert_eq!(thread_c, thread_id(board, 101));

    let view: BoardView = owner.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(view.threads, 2);

    let same_b = CreateThreadArgs {
        board,
        num: 200,
        title: "Same".to_string(),
    };
    let same_c = CreateThreadArgs {
        board,
        num: 200,
        title: "Same".to_string(),
    };
    let (outcome_b, outcome_c) = tokio::join!(
        b.retry_outcome("create_thread", &same_b),
        c.retry_outcome("create_thread", &same_c),
    );
    let winners = [outcome_b.is_ok(), outcome_c.is_ok()];
    assert_eq!(
        winners.iter().filter(|won| **won).count(),
        1,
        "exactly one concurrent spawn may win"
    );
    let loser = match (outcome_b, outcome_c) {
        (Err(error), Ok(_)) | (Ok(_), Err(error)) => application_error(&error),
        _ => unreachable!(),
    };
    match loser {
        CommandError::Entity { code, .. } => assert_eq!(code, "already-exists"),
        other => panic!("unexpected loser error: {other:?}"),
    }

    let view: BoardView = owner.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(view.threads, 3);
}

impl<'a> Run<'a> {
    async fn retry_outcome(
        &mut self,
        function: &str,
        args: &impl Serialize,
    ) -> Result<Vec<u8>, DbError> {
        loop {
            match self.call(function, args).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) if error.is_retryable() => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn client_typed_reads_and_pagination() {
    let harness = Harness::start(env()).await;
    let mut harness = harness;
    let _address = harness.start_grpc().await;
    let key = signing();
    let root = root_of(&key);
    let (_, program) = upload_board_programs(&harness, root).await;
    let mut run = Run::new(&harness, key.clone(), program);

    run.ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = run
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "reads".to_string(),
                title: "Reads".to_string(),
                admin: [0x22u8; 32],
                first_thread: Some(FirstThread {
                    num: 1,
                    title: "First".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();

    for num in 2..14u64 {
        run.ok::<EntityId>(
            "create_thread",
            &CreateThreadArgs {
                board,
                num,
                title: format!("Thread {num}"),
            },
        )
        .await;
    }
    for post in 1..=3u64 {
        run.ok::<()>(
            "submit_post",
            &SubmitArgs {
                board,
                thread,
                post,
                text: format!("Post {post}"),
            },
        )
        .await;
    }

    let client = harness.client(key.to_bytes());
    let thread_id_value = thread;
    EntityReader::read(
        &client,
        move |reader| {
            let board = board;
            async move {
                let record = reader
                    .record::<BoardEntity>(board, "BoardEntity")
                    .await?
                    .unwrap();
                assert_eq!(record.type_name, "BoardEntity");
                let threads = board.collection("threads").unwrap();
                let moderators = board.collection("moderators").unwrap();
                assert_eq!(reader.map_count(&threads).await?, 13);
                let one: Option<EntityId> = reader.map_get(&threads, &1u64).await?;
                assert_eq!(one, Some(thread_id_value));
                let missing: Option<EntityId> = reader.map_get(&threads, &999u64).await?;
                assert_eq!(missing, None);
                assert!(reader.set_has(&moderators, &[0x22u8; 32]).await?);
                assert!(!reader.set_has(&moderators, &[0x33u8; 32]).await?);
                assert_eq!(reader.set_count(&moderators).await?, 1);

                let posts = thread_id_value.collection("posts").unwrap();
                assert_eq!(reader.feed_len(&posts).await?, 3);
                assert_eq!(reader.feed_entry::<u64>(&posts, 1).await?, Some(1));
                assert_eq!(reader.feed_entry::<u64>(&posts, 3).await?, Some(3));
                assert_eq!(reader.feed_entry::<u64>(&posts, 4).await?, None);

                let mut keys = Vec::new();
                let mut cursor = None;
                let mut pages = 0;
                loop {
                    let page = reader
                        .map_page::<u64, EntityId>(&threads, cursor, 5)
                        .await?;
                    assert!(page.items.len() <= 5);
                    pages += 1;
                    keys.extend(page.items.into_iter().map(|entry| entry.key));
                    match page.next {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
                assert_eq!(pages, 3);
                assert_eq!(keys, (1..14u64).collect::<Vec<_>>());

                let page = reader.map_page::<u64, EntityId>(&threads, None, 0).await;
                assert!(matches!(page, Err(ech_db_entity::ReadError::PageLimit)));
                let page = reader
                    .map_page::<u64, EntityId>(&threads, None, ech_db_entity::MAX_PAGE + 1)
                    .await;
                assert!(matches!(page, Err(ech_db_entity::ReadError::PageLimit)));

                let mut set_keys = Vec::new();
                let mut cursor = None;
                loop {
                    let page = reader.set_page::<[u8; 32]>(&moderators, cursor, 10).await?;
                    set_keys.extend(page.items);
                    match page.next {
                        Some(next) => cursor = Some(next),
                        None => break,
                    }
                }
                assert_eq!(set_keys, vec![[0x22u8; 32]]);

                let feed = reader.feed_page::<u64>(&posts, None, 10).await?;
                assert_eq!(
                    feed.items
                        .iter()
                        .map(|entry| (entry.seq, entry.value))
                        .collect::<Vec<_>>(),
                    vec![(1, 1), (2, 2), (3, 3)]
                );
                Ok::<(), ech_db_entity::ReadError>(())
            }
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn migrations_across_bundles() {
    let harness = Harness::start(env()).await;
    let key = signing();
    let root = root_of(&key);
    let (legacy_hash, latest_hash) = upload_board_programs(&harness, root).await;
    let mut legacy = Run::new(&harness, key.clone(), legacy_hash);
    let mut latest = legacy.sibling(key.clone(), latest_hash);

    legacy
        .ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = legacy
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "legacy".to_string(),
                title: "Legacy Board".to_string(),
                admin: [0x44u8; 32],
                first_thread: Some(FirstThread {
                    num: 5,
                    title: "First".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();
    legacy
        .ok::<()>(
            "submit_post",
            &SubmitArgs {
                board,
                thread,
                post: 1,
                text: "old post".to_string(),
            },
        )
        .await;

    let before = raw_record(&harness, root, board).await.unwrap();
    let decoded: EntityRecord<BoardEntity> = EntityRecord::decode(&before, "BoardEntity").unwrap();
    assert!(matches!(decoded.state, BoardEntity::V1 { .. }));
    assert_eq!(decoded.parent, Some(EntityId::root(&root)));

    let legacy_view: BoardView = legacy.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(legacy_view.version, 1);
    assert_eq!(legacy_view.locked, None);
    assert_eq!(legacy_view.threads, 1);

    let fresh_view: BoardView = latest.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(fresh_view.version, 1, "load must not migrate");
    assert_eq!(raw_record(&harness, root, board).await.unwrap(), before);

    let upgraded: BoardView = latest.ok("upgrade_board", &BoardIdArgs { board }).await;
    assert_eq!(upgraded.version, 3);
    assert_eq!(
        upgraded.locked,
        Some(false),
        "auto migration applies the default"
    );
    assert_eq!(upgraded.moderators, 0, "auto migration adds an empty set");
    assert_eq!(upgraded.threads, 1, "collections survive the migration");

    let inspection: Inspection = latest
        .ok(
            "inspect",
            &InspectArgs {
                board,
                thread: Some(thread),
            },
        )
        .await;
    assert_eq!(inspection.thread.as_ref().unwrap().posts, 1);
    assert_eq!(
        inspection.board.events, 2,
        "upgrade itself must not write an event"
    );

    let after = raw_record(&harness, root, board).await.unwrap();
    let decoded: EntityRecord<BoardEntity> = EntityRecord::decode(&after, "BoardEntity").unwrap();
    match &decoded.state {
        BoardEntity::V3 {
            slug,
            title,
            locked,
            ..
        } => {
            assert_eq!(slug, "legacy");
            assert_eq!(title, "Legacy Board");
            assert!(!locked);
        }
        other => panic!("expected V3, got version {}", other.version()),
    }

    let created: CreatedBoard = legacy
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "locked".to_string(),
                title: "Locked Board".to_string(),
                admin: [0x55u8; 32],
                first_thread: None,
                corrupt_key: false,
            },
        )
        .await;
    let locked = created.board;
    latest
        .ok::<()>("lock_board", &BoardIdArgs { board: locked })
        .await;
    let view: BoardView = latest
        .ok("read_board", &BoardIdArgs { board: locked })
        .await;
    assert_eq!(view.version, 3);
    assert_eq!(
        view.locked,
        Some(true),
        "apply migrates then runs the handler"
    );
    assert_eq!(view.moderators, 0, "upgrade adds an empty set");
}

#[tokio::test]
#[ignore = "requires foundationdb and s3 services"]
async fn replay_restores_entities_byte_identically() {
    use ech_db_recovery::recovery::Recovery;
    use ech_db_server::exec::Executor;
    use ech_db_server::exporter::ExportOutcome;
    use ech_db_server::wasm::Profile;

    let harness = Harness::start(env()).await;
    let key = signing();
    let root = root_of(&key);
    let (legacy_hash, latest_hash) = upload_board_programs(&harness, root).await;
    let mut legacy = Run::new(&harness, key.clone(), legacy_hash);
    let mut latest = legacy.sibling(key.clone(), latest_hash);

    legacy
        .ok::<EntityId>("bootstrap", &bootstrap_args("root", false))
        .await;
    let created: CreatedBoard = legacy
        .ok(
            "create_board",
            &CreateBoardArgs {
                slug: "replay".to_string(),
                title: "Replay".to_string(),
                admin: [0x66u8; 32],
                first_thread: Some(FirstThread {
                    num: 3,
                    title: "First".to_string(),
                }),
                corrupt_key: false,
            },
        )
        .await;
    let board = created.board;
    let thread = created.thread.unwrap();
    legacy
        .ok::<()>(
            "submit_post",
            &SubmitArgs {
                board,
                thread,
                post: 1,
                text: "legacy post".to_string(),
            },
        )
        .await;
    harness.force_close_window(root).await;

    latest
        .ok::<BoardView>("upgrade_board", &BoardIdArgs { board })
        .await;
    latest
        .ok::<()>(
            "rename_board",
            &RenameArgs {
                board,
                title: "Replayed".to_string(),
            },
        )
        .await;
    latest
        .ok::<()>(
            "recursive",
            &RepeatArgs {
                board,
                remaining: 2,
                corrupt_at: None,
            },
        )
        .await;
    latest
        .ok::<()>(
            "submit_post",
            &SubmitArgs {
                board,
                thread,
                post: 2,
                text: "new post".to_string(),
            },
        )
        .await;
    let view: BoardView = latest.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(view.version, 3);
    assert_eq!(view.threads, 1);
    harness.force_close_window(root).await;

    while let ExportOutcome::Exported = harness.server.export_once(root).await.unwrap() {}
    let cursor: ech_db_protocol::values::ExportCursor = harness
        .server
        .fdb
        .trx()
        .unwrap()
        .get_bcs(&ech_db_protocol::keys::SysKey::export_cursor(&root))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cursor.number, 1);

    let reference = snapshot(&harness, root).await;
    assert!(!reference.is_empty());

    let trx = harness.server.fdb.trx().unwrap();
    let prefix = foundationdb_tuple::pack(&(foundationdb_tuple::Bytes::from(root.as_slice()),));
    let end = Prefix::successor(&prefix).unwrap();
    trx.clear_range(&prefix, &end);
    trx.commit().await.unwrap();
    assert!(snapshot(&harness, root).await.is_empty());

    let recovery = Recovery::new(&harness.config, Executor::new(Profile::engine())).unwrap();
    recovery.run(root).await.unwrap();
    assert_eq!(snapshot(&harness, root).await, reference);

    let view: BoardView = latest.ok("read_board", &BoardIdArgs { board }).await;
    assert_eq!(view.version, 3);
    assert_eq!(view.events, 7);
    let inspection: Inspection = latest
        .ok(
            "inspect",
            &InspectArgs {
                board,
                thread: Some(thread),
            },
        )
        .await;
    assert_eq!(inspection.thread.as_ref().unwrap().posts, 2);
    let client = {
        let mut harness = harness;
        harness.start_grpc().await;
        harness.client(key.to_bytes())
    };
    let events = EntityReader::read(&client, |reader| {
        let board = board;
        async move { read_events(&reader, board).await }
    })
    .await
    .unwrap();
    assert_eq!(events.len(), 7);
    assert!(matches!(events[0], BoardEvent::Genesis { .. }));
    assert!(matches!(events[1], BoardEvent::PostSubmitted { .. }));
    assert!(matches!(events[2], BoardEvent::Renamed { .. }));
    assert!(matches!(
        events[3],
        BoardEvent::Recursive { remaining: 0, .. }
    ));
    assert!(matches!(
        events[4],
        BoardEvent::Recursive { remaining: 1, .. }
    ));
    assert!(matches!(
        events[5],
        BoardEvent::Recursive { remaining: 2, .. }
    ));
    assert!(matches!(events[6], BoardEvent::PostSubmitted { .. }));
}
