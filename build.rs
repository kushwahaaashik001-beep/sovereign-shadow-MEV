fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .bytes(&["hydra.RawOpportunity"])
        .compile_protos(&["proto/hydra.proto"], &["proto"])?;
    Ok(())
}