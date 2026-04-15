fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/shadow.proto");

    // shadow.proto now contains the full 'hydra' package interface.
    tonic_build::configure()
        .bytes(&["."]) // CRITICAL: Map all Protobuf 'bytes' to bytes::Bytes for zero-copy
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile_protos(&["proto/shadow.proto"], &["proto"])?;
    
    Ok(())
}