use std::cell::OnceCell;
use anyhow::{Context, Result};
use libcontainer::workload::Executor as LibcontainerExecutor;
use crate::container::{WasiContext as ContainerWasiContext,Engine,RuntimeContext,Entrypoint};
use oci_spec::image::Platform;
use crate::sandbox::oci::WasmLayer;

use distributed_executer::distributed_executer_client::DistributedExecuterClient;
use tonic::Request;
use distributed_executer::WasiContext;
use tokio::runtime::Runtime as TokioRuntime;

pub mod distributed_executer {
    include!("../../../../stubs/executer.rs");
}

#[derive(Clone)]
enum InnerExecutor {
    Wasm,
    Linux,
    CantHandle,
}

#[derive(Clone)]
pub struct DistributedExecuter<E: Engine> {
    engine: E,
    inner: OnceCell<InnerExecutor>,
    wasm_layers: Vec<WasmLayer>,
    platform: Platform,
}


impl<E: Engine> LibcontainerExecutor for DistributedExecuter<E> {
    fn exec(&self, spec: &oci_spec::runtime::Spec) -> Result<(), libcontainer::workload::ExecutorError> {

        let server_address="http://127.0.0.1:8080";

        let wasi_context = &self.ctx(spec);



        let args = wasi_context.args().to_vec();


        let envs = wasi_context.envs().to_vec();


        let Entrypoint {source,func,name,..} = wasi_context.entrypoint();

        let wasm_bytes = source.as_bytes().expect("failed to get bytes from source").to_vec();

        let module_name = name.unwrap_or_else(|| String::from("main"));

        let func_name = func;

        let tokio_runtime = TokioRuntime::new().expect("failed to create tokio runtime");

        tokio_runtime.block_on(
            async {

                let mut client = DistributedExecuterClient::connect(server_address).await.expect("failed to connect to gRPC server");

                let wasi_context_request = WasiContext {
                    args:args,
                    envs:envs,
                    wasm_bytes:wasm_bytes,
                    func_name:func_name,
                    module_name:module_name,
                };

                let request = Request::new(wasi_context_request);

                let response = client.run_wasi(request).await.expect("error while calling the rpc function");

                println!("{:?}",response);

                return Ok(());
            }
        )
    }

    fn setup_envs(&self, envs: std::collections::HashMap<String, String>) -> Result<(), libcontainer::workload::ExecutorSetEnvsError> {
        return Ok(())
    }

    fn validate(&self, spec: &oci_spec::runtime::Spec) -> Result<(), libcontainer::workload::ExecutorValidationError> {
        return Ok(())
    }
}

impl<E: Engine> DistributedExecuter<E> {
    pub fn new(engine: E, wasm_layers: Vec<WasmLayer>, platform: Platform) -> Self {
        Self {
            engine,
            inner:Default::default(),
            wasm_layers,
            platform,
        }
    }


        /// Creates a new WasiContext from the container spec and executor state
    /// 
    /// This helper method constructs a WasiContext that provides access to:
    /// - The OCI container spec defining the container configuration
    /// - Any Wasm layers that need to be mounted/composed
    /// - Platform-specific details needed for execution
    ///
    /// The WasiContext is used by the engine to properly configure and run
    /// the Wasm module with the right environment and capabilities.
    fn ctx<'a>(&'a self, spec: &'a oci_spec::runtime::Spec) ->  ContainerWasiContext<'a> {
        let wasm_layers = &self.wasm_layers;
        let platform = &self.platform;
        ContainerWasiContext {
            spec,
            wasm_layers,
            platform,
        }
    }
    
}
