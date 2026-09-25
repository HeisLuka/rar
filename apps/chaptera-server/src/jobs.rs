use std::{future::Future, pin::Pin};

use crate::runtime_error::RuntimeError;

pub type WorkerRuntimeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>>;

pub trait WorkerRuntime: Send + Sync {
    fn run(&self) -> WorkerRuntimeFuture<'_>;
}

pub struct UnconfiguredWorkerRuntime;

impl WorkerRuntime for UnconfiguredWorkerRuntime {
    fn run(&self) -> WorkerRuntimeFuture<'_> {
        Box::pin(async {
            Err(RuntimeError::new(
                "worker_runtime_not_configured",
                "CLOUD-ASYNC-RUNTIME-01 is not connected to the runtime shell",
            ))
        })
    }
}
