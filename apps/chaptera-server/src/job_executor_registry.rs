use std::sync::Arc;

use crate::{
    job_queue::{JobKind, JobRecord},
    job_worker::{CancellationFlag, JobExecutor, JobFailure, JobFuture},
    runtime_error::RuntimeError,
};

pub struct JobExecutorRegistry {
    entries: Vec<(JobKind, Arc<dyn JobExecutor>)>,
}

impl JobExecutorRegistry {
    pub fn new(
        entries: Vec<(JobKind, Arc<dyn JobExecutor>)>,
    ) -> Result<Self, RuntimeError> {
        for (index, (kind, _)) in entries.iter().enumerate() {
            if entries[..index]
                .iter()
                .any(|(existing, _)| existing == kind)
            {
                return Err(RuntimeError::new(
                    "duplicate_job_executor",
                    format!("job executor for {} is registered more than once", kind_name(*kind)),
                ));
            }
        }
        Ok(Self { entries })
    }

    pub fn require_kinds(&self, required: &[JobKind]) -> Result<(), RuntimeError> {
        for kind in required {
            if self.executor_for(*kind).is_none() {
                return Err(RuntimeError::new(
                    "job_executor_not_configured",
                    format!("no concrete executor is registered for {}", kind_name(*kind)),
                ));
            }
        }
        Ok(())
    }

    fn executor_for(&self, kind: JobKind) -> Option<&Arc<dyn JobExecutor>> {
        self.entries
            .iter()
            .find(|(registered, _)| *registered == kind)
            .map(|(_, executor)| executor)
    }
}

impl JobExecutor for JobExecutorRegistry {
    fn execute<'a>(
        &'a self,
        job: &'a JobRecord,
        cancellation: CancellationFlag,
    ) -> JobFuture<'a> {
        match self.executor_for(job.job_kind) {
            Some(executor) => executor.execute(job, cancellation),
            None => Box::pin(async {
                Err(JobFailure {
                    retryable: false,
                    terminal_code: "job_executor_not_configured",
                })
            }),
        }
    }
}

fn kind_name(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Parse => "parse",
        JobKind::Export => "export",
        JobKind::Snapshot => "snapshot",
        JobKind::Projection => "projection",
        JobKind::BlobGc => "blob_gc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_worker::{JobSuccess, JobFuture};

    struct NamedExecutor(&'static str);

    impl JobExecutor for NamedExecutor {
        fn execute<'a>(
            &'a self,
            job: &'a JobRecord,
            _cancellation: CancellationFlag,
        ) -> JobFuture<'a> {
            let effect_key = format!("{}:{}", self.0, job.job_id);
            Box::pin(async move { Ok(JobSuccess { effect_key }) })
        }
    }

    fn job(kind: JobKind) -> JobRecord {
        JobRecord {
            job_id: "job-1".into(),
            tenant_id: "tenant-a".into(),
            job_kind: kind,
            payload_schema_version: 1,
            payload: br#"{"input":"bounded"}"#.to_vec(),
            request_hash: "a".repeat(64),
            status: crate::job_queue::JobStatus::Running,
            available_at_ms: 0,
            attempt: 1,
            max_attempts: 3,
            lease_owner: Some("worker-a".into()),
            lease_generation: 1,
            lease_expires_at_ms: Some(1_000),
            cancel_requested_at_ms: None,
            idempotency_key: "idem-1".into(),
            created_at_ms: 0,
            started_at_ms: Some(0),
            finished_at_ms: None,
            terminal_code: None,
        }
    }

    #[test]
    fn duplicate_kind_is_rejected() {
        let result = JobExecutorRegistry::new(vec![
            (JobKind::Export, Arc::new(NamedExecutor("left"))),
            (JobKind::Export, Arc::new(NamedExecutor("right"))),
        ]);
        let error = result.err().expect("duplicate registration must fail");
        assert_eq!(error.code, "duplicate_job_executor");
    }

    #[test]
    fn required_kind_must_have_concrete_executor() {
        let registry = JobExecutorRegistry::new(vec![(
            JobKind::Export,
            Arc::new(NamedExecutor("export")),
        )])
        .unwrap();

        registry.require_kinds(&[JobKind::Export]).unwrap();
        let error = registry
            .require_kinds(&[JobKind::Export, JobKind::Parse])
            .unwrap_err();
        assert_eq!(error.code, "job_executor_not_configured");
    }

    #[tokio::test]
    async fn dispatches_by_exact_job_kind() {
        let registry = JobExecutorRegistry::new(vec![
            (JobKind::Parse, Arc::new(NamedExecutor("parse"))),
            (JobKind::Export, Arc::new(NamedExecutor("export"))),
        ])
        .unwrap();

        let result = registry
            .execute(&job(JobKind::Export), CancellationFlag::default())
            .await
            .unwrap();

        assert_eq!(result.effect_key, "export:job-1");
    }

    #[tokio::test]
    async fn execute_fails_closed_when_kind_is_missing() {
        let registry = JobExecutorRegistry::new(vec![]).unwrap();
        let failure = registry
            .execute(&job(JobKind::BlobGc), CancellationFlag::default())
            .await
            .unwrap_err();

        assert!(!failure.retryable);
        assert_eq!(failure.terminal_code, "job_executor_not_configured");
    }
}
