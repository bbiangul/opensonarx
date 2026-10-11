fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The proto lives outside this crate, so Cargo would not otherwise
    // regenerate the types when it changes.
    println!("cargo:rerun-if-changed=../../proto/opensonarx.proto");
    prost_build::compile_protos(&["../../proto/opensonarx.proto"], &["../../proto/"])?;
    Ok(())
}
