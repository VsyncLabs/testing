fn main() -> Result<(),Box<dyn std::error::Error>> {
    tonic_build::configure().build_server(false).build_client(true).out_dir("stubs").compile_protos(&["proto/executer.proto"], &["."])?;
    Ok(())
}