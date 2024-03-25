fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rb_sys_env::activate()?;
    prost_build::compile_protos(&["src/metrics.proto"], &["src/"])
        .expect("failed compile protobufs");

    Ok(())
}
