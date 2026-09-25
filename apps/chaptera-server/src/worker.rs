use crate::{jobs::WorkerRuntime, runtime_error::RuntimeError};

pub async fn run(runtime: &dyn WorkerRuntime) -> Result<(), RuntimeError> {
    runtime.run().await
}
