use std::path::PathBuf;

#[test]
fn schema_gen_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/board-frontend/src/schema.gen.ts");
    let checked_in = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let generated = board_schema::frontend::Frontend::source();
    assert_eq!(
        checked_in,
        generated,
        "{} is stale: run cargo run -p board-schema -- --emit-frontend examples/board-frontend/src/schema.gen.ts",
        path.display()
    );
}
