
use std::fmt::Debug;

use executer::distributed_executer_server::{DistributedExecuter, DistributedExecuterServer};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use wamr_rust_sdk::function::Function;
//wamr sdk imports
use wamr_rust_sdk::module::Module;
use wamr_rust_sdk::runtime::Runtime;
use wamr_rust_sdk::instance::Instance as WamrInstance;


use executer::ExecResult;
use wamr_rust_sdk::wasi_context::WasiCtxBuilder;


pub mod executer {
    include!("../stubs/executer.rs");
}

#[derive(Debug,Default)]
pub struct Device2Executer {
}


impl Device2Executer {
    fn new() -> Self {
        return Device2Executer::default()
    }
}

#[tonic::async_trait]
impl DistributedExecuter for Device2Executer {
    async fn run_wasi(
        &self,
        request: Request<executer::WasiContext>,
    ) -> std::result::Result<Response<executer::ExecResult>, Status> {
        println!("request came");
        let request = request.into_inner();

        let wasm_bytes = request.wasm_bytes;
        let module_name = request.module_name; 
        let func_name = request.func_name;
        let args = request.args;
        let envs = request.envs;


        println!("{:?}",args);
        println!("{:?}",envs);
        println!("{:?}",wasm_bytes);
        println!("{}",func_name);
        println!("{}",module_name);

        let runtime = Runtime::new().expect("failed to create runtime");

        let mut module = Module::from_buf(&runtime, &wasm_bytes, &module_name).expect("failed to create module from bytes");

        let wasi_ctx = WasiCtxBuilder::new().set_pre_open_path(vec!["/"], vec![])
        .set_env_vars(envs.iter().map(String::as_str).collect())
        .set_arguments(args.iter().map(String::as_str).collect())
        .build();

        module.set_wasi_context(wasi_ctx);

        let instance = WamrInstance::new(&runtime, &module, 1024 * 64).expect("failed to create wamr instance");

        let function = Function::find_export_func(&instance, &func_name).expect("failed find function");

        let status= function.call(&instance, &vec![]).map(|_|0).map_err(|err|{
            println!("{:?}",err);
            err
        }).expect("failed to call function");
        

        let response = ExecResult {
            status
        };

        println!("function call status: {}",status);

        Ok(Response::new(response))
    }
}



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
   let addr = "0.0.0.0:8080".parse().unwrap();

   let device2_executer = Device2Executer::new();
   println!("Server is running on {}", addr);
   Server::builder().add_service(DistributedExecuterServer::new(device2_executer)).serve(addr).await.expect("Failed to serve");
   Ok(())
}
