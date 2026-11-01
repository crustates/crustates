//! Compiles the protocol with protox, so building needs no `protoc`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/crustates/agent/v1/node.proto";
    let descriptors = protox::compile([proto], ["proto"])?;
    tonic_prost_build::configure().compile_fds(descriptors)?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
