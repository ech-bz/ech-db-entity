use crate::error::Result;
use crate::runtime;

#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub mod host_store {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Store {
        data: BTreeMap<Vec<u8>, Vec<u8>>,
        reads: usize,
    }

    thread_local! {
        static STORE: RefCell<Store> = RefCell::new(Store::default());
    }

    pub fn reset() {
        STORE.with(|store| *store.borrow_mut() = Store::default());
    }

    pub fn put(key: &[u8], value: &[u8]) {
        STORE.with(|store| {
            store.borrow_mut().data.insert(key.to_vec(), value.to_vec());
        });
    }

    pub fn peek(key: &[u8]) -> Option<Vec<u8>> {
        STORE.with(|store| store.borrow().data.get(key).cloned())
    }

    pub fn reads() -> usize {
        STORE.with(|store| store.borrow().reads)
    }

    pub(super) fn get(key: &[u8]) -> Option<Vec<u8>> {
        STORE.with(|store| {
            let mut store = store.borrow_mut();
            store.reads += 1;
            store.data.get(key).cloned()
        })
    }

    pub(super) fn set(key: &[u8], value: &[u8]) {
        STORE.with(|store| {
            store
                .borrow_mut()
                .data
                .insert(key.to_vec(), value.to_vec());
        });
    }

    pub(super) fn remove(key: &[u8]) {
        STORE.with(|store| {
            store.borrow_mut().data.remove(key);
        });
    }
}

pub(crate) fn get(key: &[u8]) -> Result<Option<Vec<u8>>> {
    runtime::ensure_storage_allowed()?;
    #[cfg(target_arch = "wasm32")]
    {
        use ech_db_guest::{Host, ReadKey};
        let mut values = Host::get_raw(&[ReadKey::state(key.to_vec())]);
        Ok(values.pop().flatten())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Ok(host_store::get(key))
    }
}

pub(crate) fn set(key: &[u8], value: &[u8]) -> Result<()> {
    runtime::ensure_storage_allowed()?;
    #[cfg(target_arch = "wasm32")]
    {
        use ech_db_guest::Host;
        Host::set(key, value);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        host_store::set(key, value);
    }
    Ok(())
}

pub(crate) fn del(key: &[u8]) -> Result<()> {
    runtime::ensure_storage_allowed()?;
    #[cfg(target_arch = "wasm32")]
    {
        use ech_db_guest::Host;
        Host::del(key);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        host_store::remove(key);
    }
    Ok(())
}
