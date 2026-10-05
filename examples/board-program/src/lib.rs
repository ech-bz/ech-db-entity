#![cfg(target_arch = "wasm32")]

use board_model::*;
use ech_db_entity::{bootstrap_root, export_entity_program, AttestedIntent, EntityId, Feed, Map, SignedIntent};
use serde::de::DeserializeOwned;
use serde::Serialize;

fn decode_args<T: DeserializeOwned>(intent: &SignedIntent) -> std::result::Result<T, CommandError> {
    bcs::from_bytes(&intent.body.call.args).map_err(|error| CommandError::Args {
        message: error.to_string(),
    })
}

fn out<T: Serialize>(value: &T) -> std::result::Result<Vec<u8>, CommandError> {
    bcs::to_bytes(value).map_err(|error| CommandError::Encode {
        message: error.to_string(),
    })
}

fn handle(attested: AttestedIntent) -> std::result::Result<Vec<u8>, Vec<u8>> {
    match run(attested.intent.body.call.function.as_str(), &attested) {
        Ok(bytes) => Ok(bytes),
        Err(error) => match bcs::to_bytes(&error) {
            Ok(bytes) => Err(bytes),
            Err(_) => Err(b"encode".to_vec()),
        },
    }
}

fn run(function: &str, attested: &AttestedIntent) -> std::result::Result<Vec<u8>, CommandError> {
    let intent = &attested.intent;
    match function {
        "bootstrap" => {
            let args: BootstrapArgs = decode_args(intent)?;
            let root = bootstrap_root::<RootEntity>(&RootEvent::Genesis {
                name: args.name,
                probe: args.probe,
            })
            .map_err(|error| CommandError::entity(&error))?;
            out(&root.id().map_err(|error| CommandError::entity(&error))?)
        }
        "create_board" => {
            let args: CreateBoardArgs = decode_args(intent)?;
            let mut root = RootEntity::load_root().map_err(|error| CommandError::entity(&error))?;
            let board = root
                .spawn::<BoardEntity>(&BoardEvent::Genesis {
                    slug: args.slug,
                    title: args.title,
                    admin: args.admin,
                    first_thread: args.first_thread.clone(),
                    corrupt_key: args.corrupt_key,
                })
                .map_err(|error| CommandError::entity(&error))?;
            let board_id = board.id().map_err(|error| CommandError::entity(&error))?;
            let thread = match &args.first_thread {
                Some(seed) => board
                    .thread_of(seed.num)
                    .map_err(|error| CommandError::entity(&error))?,
                None => None,
            };
            out(&CreatedBoard {
                board: board_id,
                thread,
            })
        }
        "read_board" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            out(&BoardView::of(&board).map_err(|error| CommandError::entity(&error))?)
        }
        "upgrade_board" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board.upgrade().map_err(|error| CommandError::entity(&error))?;
            out(&BoardView::of(&board).map_err(|error| CommandError::entity(&error))?)
        }
        "rename_board" => {
            let args: RenameArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board
                .apply(&BoardEvent::Renamed { title: args.title })
                .map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "rename_reload" => {
            let args: RenameArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board
                .apply(&BoardEvent::Renamed { title: args.title })
                .map_err(|error| CommandError::entity(&error))?;
            drop(board);
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            out(&BoardView::of(&board).map_err(|error| CommandError::entity(&error))?)
        }
        "lock_board" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board.apply(&BoardEvent::Locked).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "create_thread" => {
            let args: CreateThreadArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let thread = board
                .spawn::<ThreadEntity>(&ThreadEvent::Genesis {
                    board: args.board,
                    num: args.num,
                    title: args.title,
                })
                .map_err(|error| CommandError::entity(&error))?;
            let id = thread.id().map_err(|error| CommandError::entity(&error))?;
            drop(thread);
            board
                .apply(&BoardEvent::ThreadRegistered {
                    num: args.num,
                    thread: id,
                })
                .map_err(|error| CommandError::entity(&error))?;
            out(&id)
        }
        "ping" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board.apply(&BoardEvent::Ping).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "submit_post" => {
            let args: SubmitArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board
                .apply(&BoardEvent::PostSubmitted {
                    thread: args.thread,
                    post: args.post,
                    text: args.text,
                })
                .map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "post_then_reject" => {
            let args: SubmitArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board
                .apply(&BoardEvent::PostSubmitted {
                    thread: args.thread,
                    post: args.post,
                    text: args.text,
                })
                .map_err(|error| CommandError::entity(&error))?;
            board.apply(&BoardEvent::Reject).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "nested_reload" => {
            let args: NestedArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let thread = ThreadEntity::load(args.thread).map_err(|error| CommandError::entity(&error))?;
            let again = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            drop(again);
            drop(thread);
            drop(board);
            out(&())
        }
        "recursive" => {
            let args: RepeatArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board
                .apply(&BoardEvent::Recursive {
                    remaining: args.remaining,
                    corrupt_at: args.corrupt_at,
                })
                .map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "reject_caught" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let _ = board.apply(&BoardEvent::Reject);
            out(&())
        }
        "detached_write_caught" => {
            let mut map: Map<u64, EntityId> = Map::default();
            let _ = map.set(&1, &EntityId([0u8; 32]));
            out(&())
        }
        "double_load" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let first = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let second = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            drop(first);
            drop(second);
            out(&())
        }
        "drop_reload" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let first = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            drop(first);
            let second = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let id = second.id().map_err(|error| CommandError::entity(&error))?;
            out(&id)
        }
        "wrong_type_load" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let wrong = ThreadEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            drop(wrong);
            out(&())
        }
        "detached_apply" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let bytes = bcs::to_bytes(&*board).map_err(|error| CommandError::Encode {
                message: error.to_string(),
            })?;
            drop(board);
            let mut copy: BoardEntity = bcs::from_bytes(&bytes).map_err(|error| CommandError::Args {
                message: error.to_string(),
            })?;
            copy.apply(&BoardEvent::Ping).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "key_change" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board.corrupt_slug();
            board.apply(&BoardEvent::Ping).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "collection_swap" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let mut board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            board.swap_threads().map_err(|error| CommandError::entity(&error))?;
            board.apply(&BoardEvent::Ping).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "spawn_duplicate" => {
            let args: CreateBoardArgs = decode_args(intent)?;
            let mut root = RootEntity::load_root().map_err(|error| CommandError::entity(&error))?;
            let genesis = BoardEvent::Genesis {
                slug: args.slug,
                title: args.title,
                admin: args.admin,
                first_thread: args.first_thread,
                corrupt_key: false,
            };
            let first = root.spawn::<BoardEntity>(&genesis).map_err(|error| CommandError::entity(&error))?;
            drop(first);
            let second = root.spawn::<BoardEntity>(&genesis).map_err(|error| CommandError::entity(&error))?;
            drop(second);
            out(&())
        }
        "bootstrap_again" => {
            let args: BootstrapArgs = decode_args(intent)?;
            let root = bootstrap_root::<RootEntity>(&RootEvent::Genesis {
                name: args.name,
                probe: false,
            })
            .map_err(|error| CommandError::entity(&error))?;
            drop(root);
            out(&())
        }
        "move_probe" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            apply_moved(board).map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "events_append_probe" => {
            let args: BoardIdArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let events = board
                .id()
                .map_err(|error| CommandError::entity(&error))?
                .events()
                .map_err(|error| CommandError::entity(&error))?;
            let bytes = bcs::to_bytes(&events).map_err(|error| CommandError::Encode {
                message: error.to_string(),
            })?;
            let mut feed: Feed<BoardEvent> = bcs::from_bytes(&bytes).map_err(|error| CommandError::Args {
                message: error.to_string(),
            })?;
            feed.append(&BoardEvent::Ping)
                .map_err(|error| CommandError::entity(&error))?;
            out(&())
        }
        "inspect" => {
            let args: InspectArgs = decode_args(intent)?;
            let board = BoardEntity::load(args.board).map_err(|error| CommandError::entity(&error))?;
            let board_view = BoardView::of(&board).map_err(|error| CommandError::entity(&error))?;
            drop(board);
            let thread_view = match args.thread {
                Some(id) => {
                    let thread = ThreadEntity::load(id).map_err(|error| CommandError::entity(&error))?;
                    Some(ThreadView::of(&thread).map_err(|error| CommandError::entity(&error))?)
                }
                None => None,
            };
            out(&Inspection {
                board: board_view,
                thread: thread_view,
            })
        }
        _ => Err(CommandError::entity(&ech_db_entity::Error::Rejected)),
    }
}

fn apply_moved(mut board: ech_db_entity::Guard<BoardEntity>) -> ech_db_entity::Result<()> {
    board.apply(&BoardEvent::Ping)
}

export_entity_program!(handle);
