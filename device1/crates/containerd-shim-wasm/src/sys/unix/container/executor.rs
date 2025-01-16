use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::prelude::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use libcontainer::workload::default::DefaultExecutor;
use libcontainer::workload::{
    Executor as LibcontainerExecutor, ExecutorError as LibcontainerExecutorError,
    ExecutorSetEnvsError, ExecutorValidationError,
};
use oci_spec::image::Platform;
use oci_spec::runtime::Spec;

use crate::container::{Engine, PathResolve, RuntimeContext, Source, WasiContext};
use crate::sandbox::oci::WasmLayer;

#[derive(Clone)]
enum InnerExecutor {
    Wasm,
    Linux,
    CantHandle,
}

#[derive(Clone)]
pub(crate) struct Executor<E: Engine> {
    engine: E,
    inner: OnceCell<InnerExecutor>,
    wasm_layers: Vec<WasmLayer>,
    platform: Platform,
}

impl<E: Engine> LibcontainerExecutor for Executor<E> {
    #[cfg_attr(feature = "tracing", tracing::instrument(parent = tracing::Span::current(), skip_all, level = "Info"))]
    fn validate(&self, spec: &Spec) -> Result<(), ExecutorValidationError> {
        // This function validates whether this executor can handle the given container spec
        // It checks the inner executor type determined from the spec:
        // - If it's CantHandle, returns an error saying this engine (E::name()) can't handle it
        // - For Linux or Wasm executor types, returns Ok since we can handle those
        match self.inner(spec) {
            InnerExecutor::CantHandle => Err(ExecutorValidationError::CantHandle(E::name())),
            _ => Ok(()),
        }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(parent = tracing::Span::current(), skip_all, level = "Info"))]
    fn exec(&self, spec: &Spec) -> Result<(), LibcontainerExecutorError> {
        // This function executes the container based on its determined type:
        //
        // 1. If the container type is CantHandle, returns an error indicating this engine can't handle it
        // 2. If it's a Linux container, delegates execution to the DefaultExecutor which handles standard Linux containers
        // 3. If it's a Wasm container:
        //    - Runs the Wasm module using the engine's run_wasi() method with the container context
        //    - On success, exits with the returned exit code
        //    - On error, logs the error and exits with code 137 (standard OCI error code)
        //
        // The container type is determined by self.inner(spec) which checks if it's a Linux binary/script
        // or a Wasm module that this engine can handle.
        match self.inner(spec) {
            InnerExecutor::CantHandle => Err(LibcontainerExecutorError::CantHandle(E::name())),
            InnerExecutor::Linux => {
                log::info!("executing linux container");
                DefaultExecutor {}.exec(spec)
            }
            InnerExecutor::Wasm => {
                log::info!("calling start function");
                match self.engine.run_wasi(&self.ctx(spec)) {
                    Ok(code) => std::process::exit(code),
                    Err(err) => {
                        log::info!("error running start function: {err}");
                        std::process::exit(137)
                    }
                };
            }
        }
    }

    // This is an no-op for the Wasm `Executor`. Instead of youki's libcontainer setting the envs
    // in the shim process, the shim will manage the envs itself. The expectation is that the shim will
    // call `RuntimeContext::envs()` to get the container's envs and set them in the `Engine::run_wasi`
    // function. This way, the shim can decide how to pass the envs to the WASI context.
    //
    // See the following issues for more context:
    // https://github.com/containerd/runwasi/issues/619
    // https://github.com/containers/youki/issues/2815
    fn setup_envs(
        &self,
        _: HashMap<String, String>,
    ) -> std::result::Result<(), ExecutorSetEnvsError> {
        Ok(())
    }
}

impl<E: Engine> Executor<E> {
    pub fn new(engine: E, wasm_layers: Vec<WasmLayer>, platform: Platform) -> Self {
        Self {
            engine,
            inner: Default::default(),
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
    fn ctx<'a>(&'a self, spec: &'a Spec) -> WasiContext<'a> {
        let wasm_layers = &self.wasm_layers;
        let platform = &self.platform;
        WasiContext {
            spec,
            wasm_layers,
            platform,
        }
    }

    /// Returns the appropriate InnerExecutor type for the container based on the spec.
    /// 
    /// This method determines whether the container should be handled as:
    /// 1. A Linux container - if is_linux_container() succeeds (i.e. entrypoint is an ELF binary or script)
    /// 2. A Wasm container - if is_linux_container() fails but the engine can handle the Wasm module
    /// 3. CantHandle - if neither Linux nor Wasm execution is possible
    ///
    /// The result is cached using get_or_init() to avoid re-checking on subsequent calls.
    
    fn inner(&self, spec: &Spec) -> &InnerExecutor {
        self.inner.get_or_init(|| {
            let ctx = &self.ctx(spec);
            match is_linux_container(ctx) {
                Ok(_) => InnerExecutor::Linux,
                Err(err) => {
                    log::debug!("error checking if linux container: {err}. Fallback to wasm container");
                    match self.engine.can_handle(ctx) {
                        Ok(_) => InnerExecutor::Wasm,
                        Err(err) => {
                            // log an error and return
                            log::error!("error checking if wasm container: {err}. Note: arg0 must be a path to a Wasm file");
                            InnerExecutor::CantHandle
                        }
                    }
                }
            }
        })
    }
}

fn is_linux_container(ctx: &impl RuntimeContext) -> Result<()> {
    if let Source::Oci(_) = ctx.entrypoint().source {
        bail!("the entry point contains wasm layers")
    };

    let executable = ctx
        .entrypoint()
        .arg0
        .context("no entrypoint provided")?
        .resolve_in_path()
        .find_map(|p| -> Option<PathBuf> {
            let mode = p.metadata().ok()?.permissions().mode();
            (mode & 0o001 != 0).then_some(p)
        })
        .context("entrypoint not found")?;

    // check the shebang and ELF magic number
    // https://en.wikipedia.org/wiki/Executable_and_Linkable_Format#File_header
    let mut buffer = [0; 4];
    File::open(executable)?.read_exact(&mut buffer)?;

    match buffer {
        [0x7f, 0x45, 0x4c, 0x46] => Ok(()), // ELF magic number
        [0x23, 0x21, ..] => Ok(()),         // shebang
        _ => bail!("not a valid script or elf file"),
    }
}
