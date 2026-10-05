use ech_db_entity::{
    entity, entity_impl, BcsSchema, EntityId, EntityMeta, Error, Feed, Map, Result, Schema,
};
#[cfg(not(feature = "legacy"))]
use ech_db_entity::Set;
use serde::{Deserialize, Serialize};

pub type Pubkey = [u8; 32];

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct FirstThread {
    pub num: u64,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub enum RootEvent {
    Genesis { name: String, probe: bool },
}

#[entity(event = RootEvent, key = ())]
pub enum RootEntity {
    V1 { name: String },
}

#[entity_impl]
impl RootEntity {
    fn genesis(event: &RootEvent) -> Result<Self> {
        match event {
            RootEvent::Genesis { name, probe } => {
                if *probe {
                    let _ = RootEntity::load_root();
                }
                Ok(Self::V1 { name: name.clone() })
            }
        }
    }

    fn apply(&mut self, event: &RootEvent) -> Result<()> {
        match event {
            RootEvent::Genesis { .. } => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub enum BoardEvent {
    Genesis {
        slug: String,
        title: String,
        admin: Pubkey,
        first_thread: Option<FirstThread>,
        corrupt_key: bool,
    },
    Renamed {
        title: String,
    },
    ThreadRegistered {
        num: u64,
        thread: EntityId,
    },
    PostSubmitted {
        thread: EntityId,
        post: u64,
        text: String,
    },
    Locked,
    Recursive {
        remaining: u64,
        corrupt_at: Option<u64>,
    },
    Reject,
    Ping,
}

#[cfg(feature = "legacy")]
#[entity(event = BoardEvent, key = (slug,))]
pub enum BoardEntity {
    V1 {
        slug: String,
        title: String,
        admin: Pubkey,
        threads: Map<u64, EntityId>,
    },
}

#[cfg(not(feature = "legacy"))]
#[entity(event = BoardEvent, key = (slug,))]
pub enum BoardEntity {
    V1 {
        slug: String,
        title: String,
        admin: Pubkey,
        threads: Map<u64, EntityId>,
    },
    V2 {
        slug: String,
        title: String,
        admin: Pubkey,
        threads: Map<u64, EntityId>,
        #[migrate]
        locked: bool,
    },
    V3 {
        slug: String,
        title: String,
        threads: Map<u64, EntityId>,
        locked: bool,
        #[migrate]
        moderators: Set<Pubkey>,
    },
}

#[entity_impl]
impl BoardEntity {
    fn genesis(event: &BoardEvent) -> Result<Self> {
        if let BoardEvent::Genesis { slug, title, admin, .. } = event {
            #[cfg(feature = "legacy")]
            {
                Ok(Self::V1 {
                    slug: slug.clone(),
                    title: title.clone(),
                    admin: *admin,
                    threads: Map::default(),
                })
            }
            #[cfg(not(feature = "legacy"))]
            {
                let _ = admin;
                Ok(Self::V3 {
                    slug: slug.clone(),
                    title: title.clone(),
                    threads: Map::default(),
                    locked: false,
                    moderators: Set::default(),
                })
            }
        } else {
            Err(Error::Rejected)
        }
    }

    fn apply(&mut self, event: &BoardEvent) -> Result<()> {
        match event {
            BoardEvent::Genesis { admin, first_thread, corrupt_key, .. } => {
                #[cfg(not(feature = "legacy"))]
                {
                    if let Self::V3 { moderators, .. } = self {
                        moderators.set(admin)?;
                    }
                }
                #[cfg(feature = "legacy")]
                {
                    let _ = admin;
                }
                if *corrupt_key {
                    self.corrupt_slug();
                }
                if let Some(seed) = first_thread {
                    let board = self.id()?;
                    let thread = self.spawn::<ThreadEntity>(&ThreadEvent::Genesis {
                        board,
                        num: seed.num,
                        title: seed.title.clone(),
                    })?;
                    self.threads_mut()?.set(&seed.num, &thread.id()?)?;
                }
                Ok(())
            }
            BoardEvent::Renamed { title } => {
                match self {
                    Self::V1 { title: current, .. } => *current = title.clone(),
                    #[cfg(not(feature = "legacy"))]
                    Self::V2 { title: current, .. } => *current = title.clone(),
                    #[cfg(not(feature = "legacy"))]
                    Self::V3 { title: current, .. } => *current = title.clone(),
                }
                Ok(())
            }
            BoardEvent::ThreadRegistered { num, thread } => {
                self.threads_mut()?.set(num, thread)
            }
            BoardEvent::PostSubmitted { thread, post, text } => {
                let board = self.id()?;
                let mut thread = ThreadEntity::load(*thread)?;
                let post_entity = thread.spawn::<PostEntity>(&PostEvent::Genesis {
                    board,
                    num: *post,
                    text: text.clone(),
                })?;
                drop(post_entity);
                thread.apply(&ThreadEvent::PostAdded { post: *post })?;
                Ok(())
            }
            BoardEvent::Locked => {
                #[cfg(not(feature = "legacy"))]
                match self {
                    Self::V2 { locked, .. } | Self::V3 { locked, .. } => *locked = true,
                    Self::V1 { .. } => {}
                }
                Ok(())
            }
            BoardEvent::Recursive { remaining, corrupt_at } => {
                if let Some(at) = corrupt_at {
                    if remaining == at {
                        self.corrupt_slug();
                    }
                }
                if *remaining > 0 {
                    self.apply(&BoardEvent::Recursive {
                        remaining: remaining - 1,
                        corrupt_at: *corrupt_at,
                    })?;
                }
                Ok(())
            }
            BoardEvent::Reject => {
                let board = self.id()?;
                self.threads_mut()?.set(&999, &board)?;
                Err(Error::Rejected)
            }
            BoardEvent::Ping => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub enum ThreadEvent {
    Genesis {
        board: EntityId,
        num: u64,
        title: String,
    },
    PostAdded {
        post: u64,
    },
}

#[entity(event = ThreadEvent, key = (board, num))]
pub enum ThreadEntity {
    V1 {
        board: EntityId,
        num: u64,
        title: String,
        posts: Feed<u64>,
    },
}

#[entity_impl]
impl ThreadEntity {
    fn genesis(event: &ThreadEvent) -> Result<Self> {
        if let ThreadEvent::Genesis { board, num, title } = event {
            Ok(Self::V1 {
                board: *board,
                num: *num,
                title: title.clone(),
                posts: Feed::default(),
            })
        } else {
            Err(Error::Rejected)
        }
    }

    fn apply(&mut self, event: &ThreadEvent) -> Result<()> {
        match event {
            ThreadEvent::Genesis { .. } => Ok(()),
            ThreadEvent::PostAdded { post } => match self {
                Self::V1 { posts, .. } => {
                    posts.append(post)?;
                    Ok(())
                }
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub enum PostEvent {
    Genesis {
        board: EntityId,
        num: u64,
        text: String,
    },
}

#[entity(event = PostEvent, key = (board, num))]
pub enum PostEntity {
    V1 {
        board: EntityId,
        num: u64,
        text: String,
    },
}

#[entity_impl]
impl PostEntity {
    fn genesis(event: &PostEvent) -> Result<Self> {
        match event {
            PostEvent::Genesis { board, num, text } => Ok(Self::V1 {
                board: *board,
                num: *num,
                text: text.clone(),
            }),
        }
    }

    fn apply(&mut self, _event: &PostEvent) -> Result<()> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct BootstrapArgs {
    pub name: String,
    pub probe: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct CreateBoardArgs {
    pub slug: String,
    pub title: String,
    pub admin: Pubkey,
    pub first_thread: Option<FirstThread>,
    pub corrupt_key: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct CreatedBoard {
    pub board: EntityId,
    pub thread: Option<EntityId>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct RenameArgs {
    pub board: EntityId,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct InspectArgs {
    pub board: EntityId,
    pub thread: Option<EntityId>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct Inspection {
    pub board: BoardView,
    pub thread: Option<ThreadView>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct BoardIdArgs {
    pub board: EntityId,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct CreateThreadArgs {
    pub board: EntityId,
    pub num: u64,
    pub title: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct SubmitArgs {
    pub board: EntityId,
    pub thread: EntityId,
    pub post: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct NestedArgs {
    pub board: EntityId,
    pub thread: EntityId,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct RepeatArgs {
    pub board: EntityId,
    pub remaining: u64,
    pub corrupt_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct BoardView {
    pub version: u32,
    pub title: String,
    pub slug: String,
    pub locked: Option<bool>,
    pub threads: u64,
    pub moderators: u64,
    pub events: u64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub struct ThreadView {
    pub num: u64,
    pub title: String,
    pub posts: u64,
    pub events: u64,
}

impl BoardView {
    pub fn of(board: &BoardEntity) -> Result<Self> {
        let (version, slug, title, locked) = match board {
            BoardEntity::V1 { slug, title, .. } => (1, slug.clone(), title.clone(), None),
            #[cfg(not(feature = "legacy"))]
            BoardEntity::V2 { slug, title, locked, .. } => (2, slug.clone(), title.clone(), Some(*locked)),
            #[cfg(not(feature = "legacy"))]
            BoardEntity::V3 { slug, title, locked, .. } => (3, slug.clone(), title.clone(), Some(*locked)),
        };
        Ok(Self {
            version,
            title,
            slug,
            locked,
            threads: board.threads()?.count()?,
            moderators: board.moderators_count()?,
            events: board.events()?.len()?,
        })
    }
}

impl ThreadView {
    pub fn of(thread: &ThreadEntity) -> Result<Self> {
        match thread {
            ThreadEntity::V1 { num, title, posts, .. } => Ok(Self {
                num: *num,
                title: title.clone(),
                posts: posts.len()?,
                events: thread.events()?.len()?,
            }),
        }
    }
}

impl BoardEntity {
    pub fn thread_of(&self, num: u64) -> Result<Option<EntityId>> {
        self.threads()?.get(&num)
    }

    pub fn moderators_count(&self) -> Result<u64> {
        #[cfg(feature = "legacy")]
        {
            Ok(0)
        }
        #[cfg(not(feature = "legacy"))]
        {
            match self {
                Self::V3 { moderators, .. } => moderators.count(),
                _ => Ok(0),
            }
        }
    }

    pub fn corrupt_slug(&mut self) {
        match self {
            Self::V1 { slug, .. } => *slug = "corrupt".to_string(),
            #[cfg(not(feature = "legacy"))]
            Self::V2 { slug, .. } => *slug = "corrupt".to_string(),
            #[cfg(not(feature = "legacy"))]
            Self::V3 { slug, .. } => *slug = "corrupt".to_string(),
        }
    }

    pub fn swap_threads(&mut self) -> Result<()> {
        *self.threads_mut()? = Map::default();
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, BcsSchema)]
pub enum CommandError {
    Args { message: String },
    Encode { message: String },
    Entity { code: String, message: String },
}

impl CommandError {
    pub fn entity(error: &Error) -> Self {
        Self::Entity {
            code: error.as_ref().to_string(),
            message: error.to_string(),
        }
    }
}

pub struct Schemas;

impl Schemas {
    pub fn entities() -> Vec<ech_db_entity::EntitySchema> {
        vec![
            <RootEntity as EntityMeta>::schema(),
            <BoardEntity as EntityMeta>::schema(),
            <ThreadEntity as EntityMeta>::schema(),
            <PostEntity as EntityMeta>::schema(),
        ]
    }

    pub fn events() -> Vec<Schema> {
        vec![
            <RootEvent as BcsSchema>::schema(),
            <BoardEvent as BcsSchema>::schema(),
            <ThreadEvent as BcsSchema>::schema(),
            <PostEvent as BcsSchema>::schema(),
            <FirstThread as BcsSchema>::schema(),
            <CommandError as BcsSchema>::schema(),
            <BoardView as BcsSchema>::schema(),
            <ThreadView as BcsSchema>::schema(),
        ]
    }
}
