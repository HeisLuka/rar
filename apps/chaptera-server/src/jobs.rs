use crate::runtime_error::RuntimeError;

pub trait WorkerRuntime: Send + Sync {
    fn run(&self) -> Result<(), RuntimeError>;
}

pub struct UnconfiguredWorkerRuntime;

impl WorkerRuntime for UnconfiguredWorkerRuntime {
    fn run(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::new(
            "worker_runtime_not_configured",
            "CLOUD-ASYNC-RUNTIME-01 is not connected to the runtime shell",
        ))
    }
}
