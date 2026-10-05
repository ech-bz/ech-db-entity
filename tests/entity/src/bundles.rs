use std::path::PathBuf;

use ech_db_protocol::values::ProgramBundle;

pub struct Bundles;

impl Bundles {
    pub fn board() -> ProgramBundle {
        Self::load(
            "../../target/wasm32-unknown-unknown/release/board_program.wasm",
            "cargo build -p board-program --release --target wasm32-unknown-unknown",
        )
    }

    pub fn board_legacy() -> ProgramBundle {
        Self::load(
            "../../target/legacy/wasm32-unknown-unknown/release/board_program.wasm",
            "CARGO_TARGET_DIR=target/legacy cargo build -p board-program --release --target wasm32-unknown-unknown --features legacy",
        )
    }

    fn load(relative: &str, build: &str) -> ProgramBundle {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
        let wasm = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("missing {}: run {build}", path.display()));
        ProgramBundle {
            format_version: 1,
            execution_profile: 1,
            wasm,
        }
    }
}
