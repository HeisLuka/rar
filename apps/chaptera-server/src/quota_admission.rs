use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    job_queue::{JobKind, JobRecord},
    job_worker::{
        AdmissionDecision, AdmissionFuture, AdmissionReservation, JobAdmission, ReservationFuture,
    },
    quota_store::{
        QuotaWorkClass, ReserveOutcome, ReserveRequest, SqliteQuotaAuthority,
    },
    runtime_error::RuntimeError,
};

#[derive(Debug, Clone)]
pub struct SqliteQuotaAdmissionConfig {
    pub lease_duration: Duration,
    pub amount_per_job: i64,
}

impl SqliteQuotaAdmissionConfig {
    pub fn conservative_v0(lease_duration: Duration) -> Self {
        Self {
            lease_duration,
            amount_per_job: 1,
        }
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.lease_duration.is_zero() || self.lease_duration > Duration::from_secs(24 * 60 * 60) {
            return Err(RuntimeError::new(
                "invalid_quota_admission_lease",
                "quota admission lease must be >0 and <=24 hours",
            ));
        }
        if self.amount_per_job <= 0 {
            return Err(RuntimeError::new(
                "invalid_quota_admission_amount",
                "quota admission amount must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct SqliteQuotaJobAdmission {
    authority: SqliteQuotaAuthority,
    config: SqliteQuotaAdmissionConfig,
}

impl SqliteQuotaJobAdmission {
    pub fn new(
        authority: SqliteQuotaAuthority,
        config: SqliteQuotaAdmissionConfig,
    ) -> Result<Self, RuntimeError> {
        config.validate()?;
        Ok(Self { authority, config })
    }
}

impl JobAdmission for SqliteQuotaJobAdmission {
    fn admit<'a>(&'a self, job: &'a JobRecord) -> AdmissionFuture<'a> {
        Box::pin(async move {
            let now_ms = unix_now_ms()?;
            let reservation_id = reservation_id(job);
            let request = ReserveRequest {
                tenant_id: job.tenant_id.clone(),
                reservation_id: reservation_id.clone(),
                work_class: work_class(job.job_kind),
                amount: self.config.amount_per_job,
                request_hash: job.request_hash.clone(),
            };

            match self
                .authority
                .reserve(request, now_ms, self.config.lease_duration)
                .await
            {
                Ok(ReserveOutcome::Reserved(record)) | Ok(ReserveOutcome::Existing(record)) => {
                    Ok(AdmissionDecision::Admit {
                        reservation: Arc::new(SqliteQuotaReservation {
                            authority: self.authority.clone(),
                            tenant_id: job.tenant_id.clone(),
                            reservation_id,
                            lease_duration: self.config.lease_duration,
                            state: Mutex::new(ReservationState::Active {
                                generation: record.lease_generation,
                            }),
                        }),
                    })
                }
                Err(error) if retryable_quota_code(error.code) => Ok(AdmissionDecision::RetryLater {
                    code: quota_rejection_code(error.code),
                }),
                Err(error) => Err(quota_error(error)),
            }
        })
    }
}

struct SqliteQuotaReservation {
    authority: SqliteQuotaAuthority,
    tenant_id: String,
    reservation_id: String,
    lease_duration: Duration,
    state: Mutex<ReservationState>,
}

enum ReservationState {
    Active { generation: i64 },
    Released,
}

impl AdmissionReservation for SqliteQuotaReservation {
    fn renew<'a>(&'a self) -> ReservationFuture<'a> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let generation = match *state {
                ReservationState::Active { generation } => generation,
                ReservationState::Released => {
                    return Err(RuntimeError::new(
                        "quota_reservation_released",
                        "released quota reservation cannot be renewed",
                    ));
                }
            };

            let renewed = self
                .authority
                .renew(
                    &self.tenant_id,
                    &self.reservation_id,
                    generation,
                    unix_now_ms()?,
                    self.lease_duration,
                )
                .await
                .map_err(quota_error)?;

            *state = ReservationState::Active {
                generation: renewed.lease_generation,
            };
            Ok(())
        })
    }

    fn release<'a>(&'a self) -> ReservationFuture<'a> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let generation = match *state {
                ReservationState::Active { generation } => generation,
                ReservationState::Released => {
                    return Err(RuntimeError::new(
                        "quota_reservation_double_release",
                        "quota reservation release is exact-once",
                    ));
                }
            };

            self.authority
                .release(
                    &self.tenant_id,
                    &self.reservation_id,
                    generation,
                    unix_now_ms()?,
                )
                .await
                .map_err(quota_error)?;

            *state = ReservationState::Released;
            Ok(())
        })
    }
}

fn work_class(kind: JobKind) -> QuotaWorkClass {
    match kind {
        JobKind::Export => QuotaWorkClass::Export,
        JobKind::Parse | JobKind::Snapshot | JobKind::Projection | JobKind::BlobGc => {
            QuotaWorkClass::Background
        }
    }
}

fn reservation_id(job: &JobRecord) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"chaptera-quota-job-reservation-v1\0");
    hasher.update(job.job_id.as_bytes());
    hasher.update([0]);
    hasher.update(job.lease_generation.to_be_bytes());
    format!("quota-job:{:x}", hasher.finalize())
}

fn retryable_quota_code(code: &str) -> bool {
    matches!(
        code,
        "export_concurrency_quota"
            | "shared_capacity_exhausted"
            | "background_budget_paused"
            | "semantic_headroom_exhausted"
    )
}

fn quota_rejection_code(code: &'static str) -> &'static str {
    match code {
        "export_concurrency_quota" => "quota_export_capacity",
        "shared_capacity_exhausted" => "quota_shared_capacity",
        "background_budget_paused" => "quota_background_paused",
        "semantic_headroom_exhausted" => "quota_semantic_headroom",
        _ => "quota_capacity",
    }
}

fn quota_error(error: crate::quota_store::QuotaError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
}

fn unix_now_ms() -> Result<i64, RuntimeError> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        RuntimeError::new("clock_before_epoch", "system clock is before UNIX epoch")
    })?;
    i64::try_from(elapsed.as_millis()).map_err(|_| {
        RuntimeError::new("clock_overflow", "system clock does not fit i64 milliseconds")
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use crate::{
        job_queue::{EnqueueRequest, JobKind, SqliteJobQueue},
        job_worker::{
            CancellationFlag, JobExecutor, JobFailure, JobFuture, JobSuccess, WorkerControl,
            WorkerLoop, WorkerLoopConfig,
        },
        quota_store::{QuotaConfig, SqliteQuotaAuthority},
        schema_migration::SqliteMigrationRuntime,
    };

    use super::*;

    fn temp_db(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "chaptera-quota-admission-{label}-{}-{}.sqlite",
            std::process::id(),
            unix_now_ms().unwrap()
        ))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn quota_config(shared: i64, export: i64, background: i64) -> QuotaConfig {
        QuotaConfig {
            shared_capacity: shared,
            semantic_headroom: 2,
            export_cap: export,
            background_cap: background,
        }
    }

    async fn runtime(
        label: &str,
        config: QuotaConfig,
    ) -> (SqliteJobQueue, SqliteQuotaAuthority, PathBuf) {
        let path = temp_db(label);
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
        let queue = SqliteJobQueue::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        let quota = SqliteQuotaAuthority::open(&path, 4, Duration::from_secs(2), config)
            .await
            .unwrap();
        (queue, quota, path)
    }

    async fn enqueue(queue: &SqliteJobQueue, id: &str, kind: JobKind) {
        queue
            .enqueue(EnqueueRequest {
                job_id: id.into(),
                tenant_id: "tenant-a".into(),
                job_kind: kind,
                payload_schema_version: 1,
                payload: br#"{"revision":"rev-1"}"#.to_vec(),
                idempotency_key: format!("idem-{id}"),
                max_attempts: 3,
                now_ms: unix_now_ms().unwrap(),
            })
            .await
            .unwrap();
    }

    fn worker_config(owner: &str, kind: JobKind) -> WorkerLoopConfig {
        WorkerLoopConfig {
            owner: owner.into(),
            allowed_kinds: vec![kind],
            lease_duration: Duration::from_millis(500),
            heartbeat_interval: Duration::from_millis(40),
            idle_poll_interval: Duration::from_millis(5),
            drain_timeout: Duration::from_millis(100),
        }
    }

    struct SuccessAndDrain {
        control: WorkerControl,
        delayed: bool,
    }

    impl JobExecutor for SuccessAndDrain {
        fn execute<'a>(
            &'a self,
            job: &'a JobRecord,
            _cancellation: CancellationFlag,
        ) -> JobFuture<'a> {
            let effect_key = format!("effect-{}", job.job_id);
            let control = self.control.clone();
            let delayed = self.delayed;
            Box::pin(async move {
                if delayed {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
                control.request_drain();
                Ok(JobSuccess { effect_key })
            })
        }
    }

    #[tokio::test]
    async fn export_job_consumes_and_releases_real_sqlite_quota() {
        let (queue, quota, path) = runtime("export", quota_config(1, 1, 1)).await;
        enqueue(&queue, "job-export", JobKind::Export).await;

        let admission = Arc::new(
            SqliteQuotaJobAdmission::new(
                quota.clone(),
                SqliteQuotaAdmissionConfig::conservative_v0(Duration::from_secs(2)),
            )
            .unwrap(),
        );
        let control = WorkerControl::default();
        let worker = WorkerLoop::new(
            queue.clone(),
            Arc::new(SuccessAndDrain {
                control: control.clone(),
                delayed: false,
            }),
            admission,
            control,
            worker_config("worker-a", JobKind::Export),
        )
        .unwrap();

        let receipt = worker.run().await.unwrap();
        assert_eq!(receipt.admitted, 1);
        assert_eq!(receipt.succeeded, 1);
        assert_eq!(quota.usage("tenant-a", unix_now_ms().unwrap()).await.unwrap().export, 0);

        queue.close().await;
        quota.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn heartbeat_renews_quota_then_terminal_success_releases_latest_generation() {
        let (queue, quota, path) = runtime("renew", quota_config(1, 1, 1)).await;
        enqueue(&queue, "job-renew", JobKind::Export).await;

        let admission = Arc::new(
            SqliteQuotaJobAdmission::new(
                quota.clone(),
                SqliteQuotaAdmissionConfig::conservative_v0(Duration::from_millis(250)),
            )
            .unwrap(),
        );
        let control = WorkerControl::default();
        let worker = WorkerLoop::new(
            queue.clone(),
            Arc::new(SuccessAndDrain {
                control: control.clone(),
                delayed: true,
            }),
            admission,
            control,
            worker_config("worker-renew", JobKind::Export),
        )
        .unwrap();

        let receipt = worker.run().await.unwrap();
        assert_eq!(receipt.succeeded, 1);
        assert_eq!(receipt.lease_lost, 0);
        assert_eq!(quota.usage("tenant-a", unix_now_ms().unwrap()).await.unwrap().export, 0);

        queue.close().await;
        quota.close().await;
        cleanup(&path);
    }

    #[tokio::test]
    async fn exhausted_export_capacity_requeues_before_executor_runs() {
        let (queue, quota, path) = runtime("reject", quota_config(1, 1, 1)).await;

        quota
            .reserve(
                ReserveRequest {
                    tenant_id: "tenant-a".into(),
                    reservation_id: "external-export".into(),
                    work_class: QuotaWorkClass::Export,
                    amount: 1,
                    request_hash: "b".repeat(64),
                },
                unix_now_ms().unwrap(),
                Duration::from_secs(10),
            )
            .await
            .unwrap();

        enqueue(&queue, "job-blocked", JobKind::Export).await;

        struct MustNotRun(Arc<AtomicBool>);
        impl JobExecutor for MustNotRun {
            fn execute<'a>(
                &'a self,
                _job: &'a JobRecord,
                _cancellation: CancellationFlag,
            ) -> JobFuture<'a> {
                self.0.store(true, Ordering::SeqCst);
                Box::pin(async {
                    Err(JobFailure {
                        retryable: false,
                        terminal_code: "must_not_run",
                    })
                })
            }
        }

        let ran = Arc::new(AtomicBool::new(false));
        let admission = Arc::new(
            SqliteQuotaJobAdmission::new(
                quota.clone(),
                SqliteQuotaAdmissionConfig::conservative_v0(Duration::from_secs(2)),
            )
            .unwrap(),
        );
        let control = WorkerControl::default();
        let worker = WorkerLoop::new(
            queue.clone(),
            Arc::new(MustNotRun(ran.clone())),
            admission,
            control.clone(),
            worker_config("worker-blocked", JobKind::Export),
        )
        .unwrap();

        let task = tokio::spawn(async move { worker.run().await.unwrap() });
        tokio::time::sleep(Duration::from_millis(25)).await;
        control.request_drain();
        let receipt = task.await.unwrap();

        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(receipt.admission_requeued, 1);

        queue.close().await;
        quota.close().await;
        cleanup(&path);
    }

    #[test]
    fn async_worker_never_consumes_interactive_headroom() {
        assert_eq!(work_class(JobKind::Export), QuotaWorkClass::Export);
        for kind in [
            JobKind::Parse,
            JobKind::Snapshot,
            JobKind::Projection,
            JobKind::BlobGc,
        ] {
            assert_eq!(work_class(kind), QuotaWorkClass::Background);
        }
    }
}
