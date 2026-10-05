use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};

use serde::de::DeserializeOwned;
use serde::Serialize;

use ech_db_protocol::values::IntentRef;

use crate::collections::{append_entry, read_counter, Binding, CollectionSlot};
use crate::error::{Error, Result};
use crate::ids::{CollectionId, EntityId};
use crate::record::{EntityRecord, EntityRecordRef, RecordError};
use crate::schema::EntitySchema;
use crate::state_keys::{CollectionKey, EntityKey};
use crate::storage;

pub trait EntityMeta: Serialize + DeserializeOwned + Sized + 'static {
    type Event: Serialize + DeserializeOwned + 'static;

    const TYPE_NAME: &'static str;
    const LATEST_VERSION: u32;

    fn version(&self) -> u32;
    fn key_bytes(&self) -> Result<Vec<u8>>;
    fn is_genesis(event: &Self::Event) -> bool;
    fn visit_collections(
        &mut self,
        visitor: &mut dyn FnMut(&'static str, &mut dyn CollectionSlot) -> Result<()>,
    ) -> Result<()>;
    fn migrate_step(state: &mut Self, from: u32, ctx: &mut EntityContext) -> Result<()>;
    fn schema() -> EntitySchema;
}

pub trait EntityHandlers: EntityMeta {
    fn genesis(event: &Self::Event) -> Result<Self>;
    fn body(&mut self, event: &Self::Event) -> Result<()>;
}

pub trait Entity: EntityMeta + EntityHandlers {}

impl<T: EntityMeta + EntityHandlers> Entity for T {}

#[derive(PartialEq, Eq, Debug)]
enum Mode {
    Loaded,
    Creating(Vec<Vec<u8>>),
    Migrating,
}

struct Slot {
    id: EntityId,
    parent: Option<EntityId>,
    type_name: &'static str,
    key: Vec<u8>,
    events: CollectionId,
    state_addr: usize,
    flag_addr: usize,
    epoch: u64,
    depth: u32,
    mode: Mode,
}

struct Runtime {
    generation: u64,
    active: bool,
    root: [u8; 32],
    source: Option<IntentRef>,
    poison: Option<Error>,
    slots: BTreeMap<EntityId, Slot>,
    next_epoch: u64,
    storage_lock: bool,
    storage_violation: bool,
}

impl Runtime {
    fn new() -> Self {
        Self {
            generation: 0,
            active: false,
            root: [0u8; 32],
            source: None,
            poison: None,
            slots: BTreeMap::new(),
            next_epoch: 0,
            storage_lock: false,
            storage_violation: false,
        }
    }
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

fn with<R>(operation: impl FnOnce(&mut Runtime) -> R) -> R {
    RUNTIME.with(|runtime| operation(&mut runtime.borrow_mut()))
}

fn slot_by_addr(runtime: &Runtime, addr: usize) -> Option<&Slot> {
    runtime.slots.values().find(|slot| slot.state_addr == addr)
}

fn slot_mut_by_addr(runtime: &mut Runtime, addr: usize) -> Option<&mut Slot> {
    runtime
        .slots
        .values_mut()
        .find(|slot| slot.state_addr == addr)
}

pub fn begin(root: [u8; 32], author: [u8; 32], seq: u64) {
    with(|runtime| {
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.active = true;
        runtime.root = root;
        runtime.source = Some(IntentRef { author, seq });
        runtime.poison = None;
        runtime.slots.clear();
        runtime.storage_lock = false;
        runtime.storage_violation = false;
    });
}

pub fn finish(outcome: core::result::Result<Vec<u8>, Vec<u8>>) -> core::result::Result<Vec<u8>, Vec<u8>> {
    let poison = with(|runtime| {
        runtime.active = false;
        runtime.source = None;
        runtime.slots.clear();
        runtime.poison.take()
    });
    match (outcome, poison) {
        (Err(error), _) => Err(error),
        (Ok(bytes), None) => Ok(bytes),
        (Ok(_), Some(error)) => Err(format!("entity intent poisoned: {error}").into_bytes()),
    }
}

pub(crate) fn intent_source() -> Option<IntentRef> {
    with(|runtime| runtime.source.clone())
}

pub(crate) fn ensure_storage_allowed() -> Result<()> {
    with(|runtime| {
        if runtime.storage_lock {
            runtime.storage_violation = true;
            return Err(Error::StorageForbidden);
        }
        Ok(())
    })
}

pub(crate) fn poison_on_error<T>(result: Result<T>) -> Result<T> {
    if let Err(error) = &result {
        with(|runtime| {
            if runtime.poison.is_none() {
                runtime.poison = Some(error.clone());
            }
        });
    }
    result
}

pub(crate) fn ensure_write_allowed() -> Result<()> {
    with(|runtime| match runtime.poison {
        Some(_) => Err(Error::Poisoned),
        None => Ok(()),
    })
}

pub(crate) fn binding_valid(binding: &Binding) -> bool {
    with(|runtime| {
        if binding.generation != runtime.generation {
            return false;
        }
        match runtime.slots.get(&binding.owner) {
            Some(slot) => slot.epoch == binding.epoch,
            None => false,
        }
    })
}

struct StateSlot<T> {
    taken: Cell<bool>,
    state: ManuallyDrop<T>,
}

impl<T> StateSlot<T> {
    fn new(state: T) -> Self {
        Self {
            taken: Cell::new(false),
            state: ManuallyDrop::new(state),
        }
    }

    fn state_ptr(&self) -> *mut T {
        let pointer: *const ManuallyDrop<T> = &self.state;
        pointer as *mut T
    }

    fn state_addr(&self) -> usize {
        self.state_ptr() as usize
    }

    fn flag_addr(&self) -> usize {
        (&self.taken) as *const Cell<bool> as usize
    }
}

pub struct Guard<T: EntityMeta> {
    id: EntityId,
    epoch: u64,
    state: Box<StateSlot<T>>,
}

impl<T: EntityMeta> Deref for Guard<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.state.state_ptr() }
    }
}

impl<T: EntityMeta> DerefMut for Guard<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.state.state_ptr() }
    }
}

impl<T: EntityMeta> Drop for Guard<T> {
    fn drop(&mut self) {
        with(|runtime| {
            let owned = runtime
                .slots
                .get(&self.id)
                .map(|slot| slot.epoch == self.epoch)
                .unwrap_or(false);
            if owned {
                runtime.slots.remove(&self.id);
            }
        });
        if !self.state.taken.get() {
            unsafe { ManuallyDrop::drop(&mut self.state.state) };
        }
    }
}

pub fn load<T: EntityMeta>(id: EntityId) -> Result<Guard<T>> {
    load_inner::<T>(id)
}

fn load_inner<T: EntityMeta>(id: EntityId) -> Result<Guard<T>> {
    with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        if runtime.slots.contains_key(&id) {
            return Err(Error::AlreadyLoaded(id));
        }
        Ok(())
    })?;
    let record_key = EntityKey::record(&id);
    let Some(bytes) = storage::get(&record_key)? else {
        return Err(Error::Missing(id));
    };
    let record = EntityRecord::<T>::decode(&bytes, T::TYPE_NAME).map_err(|error| match error {
        RecordError::TypeMismatch { expected, found } => Error::TypeMismatch { expected, found },
        RecordError::Decode => Error::Decode,
    })?;
    let state = record.state;
    let version = state.version();
    if version == 0 || version > T::LATEST_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    let key = T::key_bytes(&state)?;
    let mut guard = register_new::<T>(id, record.parent, key, state, Mode::Loaded)?;
    bind_collections::<T>(&mut *guard, id, false)?;
    Ok(guard)
}

pub fn load_root<T: EntityMeta>() -> Result<Guard<T>> {
    let root = with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        Ok(runtime.root)
    })?;
    load::<T>(EntityId::root(&root))
}

pub fn attached_id<T: EntityMeta>(state: &T) -> Result<EntityId> {
    let addr = state as *const T as usize;
    with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        let slot = slot_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
        if slot.type_name != T::TYPE_NAME {
            return Err(Error::NotAttached);
        }
        Ok(slot.id)
    })
}

pub fn events<T: EntityMeta>(state: &T) -> Result<EventLog<T::Event>> {
    let id = attached_id(state)?;
    let events = with(|runtime| runtime.slots.get(&id).map(|slot| slot.events).ok_or(Error::NotAttached))?;
    Ok(EventLog {
        id: events,
        marker: PhantomData,
    })
}

pub fn feed_source(collection: &CollectionId, seq: u64) -> Result<Option<IntentRef>> {
    match storage::get(&CollectionKey::source(collection, seq))? {
        None => Ok(None),
        Some(bytes) => bcs::from_bytes(&bytes).map(Some).map_err(|_| Error::ValueDecode),
    }
}

pub struct EventLog<E> {
    id: CollectionId,
    marker: PhantomData<fn() -> E>,
}

impl<E: DeserializeOwned> EventLog<E> {
    pub fn len(&self) -> Result<u64> {
        read_counter(&CollectionKey::length(&self.id))
    }

    pub fn get(&self, seq: u64) -> Result<Option<E>> {
        match storage::get(&CollectionKey::feed_entry(&self.id, seq))? {
            None => Ok(None),
            Some(bytes) => bcs::from_bytes(&bytes).map(Some).map_err(|_| Error::ValueDecode),
        }
    }

    pub fn source(&self, seq: u64) -> Result<Option<IntentRef>> {
        feed_source(&self.id, seq)
    }
}

pub fn spawn_child<E: Entity>(parent: EntityId, event: &E::Event) -> Result<Guard<E>> {
    poison_on_error(spawn_inner::<E>(parent, event))
}

fn spawn_inner<E: Entity>(parent: EntityId, event: &E::Event) -> Result<Guard<E>> {
    with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        if runtime.poison.is_some() {
            return Err(Error::Poisoned);
        }
        if !runtime.slots.contains_key(&parent) {
            return Err(Error::NotAttached);
        }
        Ok(())
    })?;
    if !E::is_genesis(event) {
        return Err(Error::ExpectedGenesis);
    }
    let state = run_genesis::<E>(event)?;
    if state.version() != E::LATEST_VERSION {
        return Err(Error::UnsupportedVersion(state.version()));
    }
    let key = E::key_bytes(&state)?;
    let id = EntityId::child_key_bytes(parent, E::TYPE_NAME, &key)?;
    with(|runtime| {
        if runtime.slots.contains_key(&id) {
            return Err(Error::AlreadyLoaded(id));
        }
        Ok(())
    })?;
    if storage::get(&EntityKey::record(&id))?.is_some() {
        return Err(Error::AlreadyExists(id));
    }
    let mut guard = register_new::<E>(id, Some(parent), key, state, Mode::Creating(Vec::new()))?;
    bind_collections::<E>(&mut *guard, id, true)?;
    apply::<E>(&mut *guard, event)?;
    Ok(guard)
}

pub fn bootstrap_root<E: Entity>(event: &E::Event) -> Result<Guard<E>> {
    poison_on_error(bootstrap_inner::<E>(event))
}

fn bootstrap_inner<E: Entity>(event: &E::Event) -> Result<Guard<E>> {
    let root = with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        if runtime.poison.is_some() {
            return Err(Error::Poisoned);
        }
        Ok(runtime.root)
    })?;
    let id = EntityId::root(&root);
    with(|runtime| {
        if runtime.slots.contains_key(&id) {
            return Err(Error::AlreadyLoaded(id));
        }
        Ok(())
    })?;
    if storage::get(&EntityKey::record(&id))?.is_some() {
        return Err(Error::AlreadyExists(id));
    }
    if !E::is_genesis(event) {
        return Err(Error::ExpectedGenesis);
    }
    let state = run_genesis::<E>(event)?;
    if state.version() != E::LATEST_VERSION {
        return Err(Error::UnsupportedVersion(state.version()));
    }
    let key = E::key_bytes(&state)?;
    let mut guard = register_new::<E>(id, None, key, state, Mode::Creating(Vec::new()))?;
    bind_collections::<E>(&mut *guard, id, true)?;
    apply::<E>(&mut *guard, event)?;
    Ok(guard)
}

fn run_genesis<E: Entity>(event: &E::Event) -> Result<E> {
    with(|runtime| {
        runtime.storage_lock = true;
        runtime.storage_violation = false;
    });
    let outcome = E::genesis(event);
    let violated = with(|runtime| {
        runtime.storage_lock = false;
        core::mem::take(&mut runtime.storage_violation)
    });
    if violated {
        return Err(Error::StorageForbidden);
    }
    outcome
}

fn register_new<T: EntityMeta>(
    id: EntityId,
    parent: Option<EntityId>,
    key: Vec<u8>,
    state: T,
    mode: Mode,
) -> Result<Guard<T>> {
    let events = id.events()?;
    let boxed = Box::new(StateSlot::new(state));
    let epoch = with(|runtime| {
        if runtime.slots.contains_key(&id) {
            return Err(Error::AlreadyLoaded(id));
        }
        let epoch = runtime.next_epoch;
        runtime.next_epoch = runtime.next_epoch.wrapping_add(1);
        runtime.slots.insert(
            id,
            Slot {
                id,
                parent,
                type_name: T::TYPE_NAME,
                key,
                events,
                state_addr: boxed.state_addr(),
                flag_addr: boxed.flag_addr(),
                epoch,
                depth: 0,
                mode,
            },
        );
        Ok(epoch)
    })?;
    Ok(Guard {
        id,
        epoch,
        state: boxed,
    })
}

pub fn apply<T: Entity>(state: &mut T, event: &T::Event) -> Result<()> {
    poison_on_error(apply_inner::<T>(state, event))
}

fn apply_inner<T: Entity>(state: &mut T, event: &T::Event) -> Result<()> {
    let addr = state as *mut T as usize;
    let (id, depth, creating) = with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        if runtime.poison.is_some() {
            return Err(Error::Poisoned);
        }
        let slot = slot_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
        if slot.type_name != T::TYPE_NAME {
            return Err(Error::NotAttached);
        }
        if matches!(slot.mode, Mode::Migrating) {
            return Err(Error::Nested);
        }
        Ok((slot.id, slot.depth, matches!(slot.mode, Mode::Creating(_))))
    })?;
    let genesis = T::is_genesis(event);
    if genesis && !(creating && depth == 0) {
        return Err(Error::GenesisRejected);
    }
    with(|runtime| {
        let slot = slot_mut_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
        slot.depth = slot.depth.checked_add(1).ok_or(Error::DepthExceeded)?;
        Ok(())
    })?;
    let outcome = (|| -> Result<()> {
        if depth == 0 && !creating {
            check_key::<T>(state, addr)?;
            check_version::<T>(state)?;
            migrate_to_latest::<T>(state, addr)?;
            bind_collections::<T>(state, id, false)?;
        } else {
            check_key::<T>(state, addr)?;
            check_version_exact::<T>(state)?;
            bind_collections::<T>(state, id, false)?;
        }
        T::body(state, event)?;
        check_key::<T>(state, addr)?;
        check_version_exact::<T>(state)?;
        bind_collections::<T>(state, id, false)?;
        ensure_write_allowed()?;
        let event_bytes = bcs::to_bytes(event).map_err(|_| Error::Encode)?;
        if creating {
            if depth == 0 {
                let events = slot_events(addr)?;
                append_entry(&events, &event_bytes)?;
                flush_creation_events(addr)?;
            } else {
                with(|runtime| {
                    let slot = slot_mut_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
                    match &mut slot.mode {
                        Mode::Creating(buffer) => {
                            buffer.push(event_bytes);
                            Ok(())
                        }
                        _ => Err(Error::NotAttached),
                    }
                })?;
            }
        } else {
            let events = slot_events(addr)?;
            append_entry(&events, &event_bytes)?;
        }
        if depth == 0 {
            save::<T>(state, id)?;
        }
        Ok(())
    })();
    with(|runtime| {
        if let Some(slot) = slot_mut_by_addr(runtime, addr) {
            slot.depth = slot.depth.saturating_sub(1);
        }
    });
    outcome
}

pub fn upgrade<T: EntityMeta>(state: &mut T) -> Result<()> {
    poison_on_error(upgrade_inner::<T>(state))
}

fn upgrade_inner<T: EntityMeta>(state: &mut T) -> Result<()> {
    let addr = state as *mut T as usize;
    let id = with(|runtime| {
        if !runtime.active {
            return Err(Error::NotAttached);
        }
        if runtime.poison.is_some() {
            return Err(Error::Poisoned);
        }
        let slot = slot_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
        if slot.type_name != T::TYPE_NAME {
            return Err(Error::NotAttached);
        }
        if slot.depth != 0 || matches!(slot.mode, Mode::Migrating) {
            return Err(Error::Nested);
        }
        Ok(slot.id)
    })?;
    check_key::<T>(state, addr)?;
    check_version::<T>(state)?;
    if state.version() == T::LATEST_VERSION {
        bind_collections::<T>(state, id, false)?;
        return Ok(());
    }
    migrate_to_latest::<T>(state, addr)?;
    check_key::<T>(state, addr)?;
    bind_collections::<T>(state, id, false)?;
    save::<T>(state, id)
}

fn check_key<T: EntityMeta>(state: &T, addr: usize) -> Result<()> {
    let expected = with(|runtime| {
        slot_by_addr(runtime, addr)
            .map(|slot| slot.key.clone())
            .ok_or(Error::NotAttached)
    })?;
    if T::key_bytes(state)? != expected {
        return Err(Error::KeyChanged);
    }
    Ok(())
}

fn check_version<T: EntityMeta>(state: &T) -> Result<()> {
    let version = state.version();
    if version == 0 || version > T::LATEST_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    Ok(())
}

fn check_version_exact<T: EntityMeta>(state: &T) -> Result<()> {
    let version = state.version();
    if version != T::LATEST_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    Ok(())
}

fn migrate_to_latest<T: EntityMeta>(state: &mut T, addr: usize) -> Result<()> {
    while state.version() != T::LATEST_VERSION {
        let from = state.version();
        if from == 0 || from > T::LATEST_VERSION {
            return Err(Error::UnsupportedVersion(from));
        }
        let (id, state_addr, flag_addr) = with(|runtime| {
            let slot = slot_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
            Ok((slot.id, slot.state_addr, slot.flag_addr))
        })?;
        let mut ctx = EntityContext {
            state_addr,
            flag_addr,
        };
        with(|runtime| {
            slot_mut_by_addr(runtime, addr)
                .ok_or(Error::NotAttached)?
                .mode = Mode::Migrating;
            Ok(())
        })?;
        let outcome = T::migrate_step(state, from, &mut ctx);
        with(|runtime| {
            if let Some(slot) = slot_mut_by_addr(runtime, addr) {
                if matches!(slot.mode, Mode::Migrating) {
                    slot.mode = Mode::Loaded;
                }
            }
        });
        outcome?;
        if migration_taken(flag_addr) {
            return Err(Error::MigrationInvalid);
        }
        if state.version() != from + 1 {
            return Err(Error::MigrationInvalid);
        }
        check_key::<T>(state, addr)?;
        bind_collections::<T>(state, id, true)?;
    }
    Ok(())
}

fn migration_taken(flag_addr: usize) -> bool {
    unsafe { (*(flag_addr as *const Cell<bool>)).get() }
}

fn bind_collections<T: EntityMeta>(state: &mut T, id: EntityId, attach: bool) -> Result<()> {
    let (epoch, generation) = with(|runtime| {
        let slot = runtime.slots.get(&id).ok_or(Error::NotAttached)?;
        Ok((slot.epoch, runtime.generation))
    })?;
    let binding = Binding::new(id, epoch, generation);
    T::visit_collections(state, &mut |field, slot| {
        let expected = id.collection(field)?;
        match slot.address() {
            Some(found) if *found == expected => {
                slot.bind(binding);
                Ok(())
            }
            None if attach => {
                slot.attach(expected, binding);
                Ok(())
            }
            _ => Err(Error::CollectionAddress),
        }
    })
}

fn save<T: EntityMeta>(state: &T, id: EntityId) -> Result<()> {
    let parent = with(|runtime| {
        runtime
            .slots
            .get(&id)
            .map(|slot| slot.parent)
            .ok_or(Error::NotAttached)
    })?;
    let record = EntityRecordRef {
        type_name: T::TYPE_NAME,
        parent,
        state,
    };
    let encoded = record.encode().map_err(|_| Error::Encode)?;
    storage::set(&EntityKey::record(&id), &encoded)
}

fn slot_events(addr: usize) -> Result<CollectionId> {
    with(|runtime| {
        slot_by_addr(runtime, addr)
            .map(|slot| slot.events)
            .ok_or(Error::NotAttached)
    })
}

fn flush_creation_events(addr: usize) -> Result<()> {
    let (events, buffered) = with(|runtime| {
        let slot = slot_mut_by_addr(runtime, addr).ok_or(Error::NotAttached)?;
        let events = slot.events;
        match core::mem::replace(&mut slot.mode, Mode::Loaded) {
            Mode::Creating(buffer) => Ok((events, buffer)),
            other => {
                slot.mode = other;
                Err(Error::NotAttached)
            }
        }
    })?;
    for bytes in buffered {
        append_entry(&events, &bytes)?;
    }
    Ok(())
}

pub struct EntityContext {
    state_addr: usize,
    flag_addr: usize,
}

impl EntityContext {
    pub fn take<T>(&self, state: &mut T) -> Result<T> {
        self.verify(state)?;
        let taken = unsafe { &*(self.flag_addr as *const Cell<bool>) };
        if taken.get() {
            return Err(Error::MigrationInvalid);
        }
        taken.set(true);
        Ok(unsafe { core::ptr::read(state) })
    }

    pub fn install<T>(&self, state: &mut T, value: T) -> Result<()> {
        self.verify(state)?;
        let taken = unsafe { &*(self.flag_addr as *const Cell<bool>) };
        if !taken.get() {
            return Err(Error::MigrationInvalid);
        }
        unsafe {
            core::ptr::write(state, value);
        }
        taken.set(false);
        Ok(())
    }

    fn verify<T>(&self, state: &mut T) -> Result<()> {
        if state as *mut T as usize != self.state_addr {
            return Err(Error::NotAttached);
        }
        Ok(())
    }
}
