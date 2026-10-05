use ech_db_protocol::values::{AttestedIntent, CallOutput};

use crate::runtime;

pub fn alloc(len: i32) -> i32 {
    let len = len as u32 as usize;
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    core::mem::forget(buffer);
    pointer as u32 as i32
}

fn pack_response(output: &CallOutput) -> i64 {
    let encoded = bcs::to_bytes(output).expect("call output encodes");
    let len = encoded.len();
    let pointer = alloc(len as i32);
    unsafe {
        core::ptr::copy_nonoverlapping(encoded.as_ptr(), pointer as u32 as usize as *mut u8, len);
    }
    (((pointer as u32 as u64) << 32) | (len as u32 as u64)) as i64
}

pub fn call_entity<F>(ptr: i32, len: i32, handler: F) -> i64
where
    F: FnOnce(AttestedIntent) -> Result<Vec<u8>, Vec<u8>>,
{
    let input = unsafe {
        core::slice::from_raw_parts(ptr as u32 as usize as *const u8, len as u32 as usize)
    };
    let attested = match bcs::from_bytes::<AttestedIntent>(input) {
        Ok(attested) => attested,
        Err(_) => return pack_response(&CallOutput::Err(b"bad intent".to_vec())),
    };
    runtime::begin(
        attested.intent.body.root,
        attested.intent.body.author,
        attested.intent.body.expected_seq,
    );
    let output = match runtime::finish(handler(attested)) {
        Ok(bytes) => CallOutput::Ok(bytes),
        Err(bytes) => CallOutput::Err(bytes),
    };
    pack_response(&output)
}

#[macro_export]
macro_rules! export_entity_program {
    ($handler:path) => {
        #[no_mangle]
        pub extern "C" fn ech_alloc(len: i32) -> i32 {
            $crate::entry::alloc(len)
        }

        #[no_mangle]
        pub extern "C" fn ech_call(ptr: i32, len: i32) -> i64 {
            $crate::entry::call_entity(ptr, len, $handler)
        }
    };
}
