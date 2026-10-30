use std::{error::Error, path::Path};
fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    println!("cargo::rerun-if-changed={}", root.display());
    prost_build::Config::new()
        .include_file("protos.rs")
        .compile_protos(
            &[
                root.join("platform/exec/v1/exec.proto"),
                root.join("platform/errors/v1/errors.proto"),
            ],
            &[root],
        )?;
    Ok(())
}
