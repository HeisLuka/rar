use crate::{jobs::WorkerRuntime, runtime_error::RuntimeError};

pub fn run(runtime: &dyn WorkerRuntime) -> Result<(), RuntimeError> {
    runtime.run()
}
