use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::{
    sync::Notify,
    time::{Instant, MissedTickBehavior, interval, sleep},
};

use crate::{
    job_queue::{FailureOutcome, JobKind, JobRecord, Lease, PublishOutcome, SqliteJobQueue},
    runtime_error::RuntimeError,
};

pub type JobFuture<'a> =
    Pin<Box<dyn Future<Output = Result<JobSuccess, JobFailure>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSuccess {
    pub effect_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFailure {
    pub retryable: bool,
    pub terminal_code: &'static str,
}

pub trait JobExecutor: Send + Sync {
    fn execute<'a>(&'a self, job: &'a JobRecord, cancellation: CancellationFlag)
        -> JobFuture<'a>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDecision {
    Admit,
    RetryLater { code: &'static str },
    Reject { code: &'static str },
}

pub trait JobAdmission: Send + Sync {
    fn admit(&self, job: &JobRecord) -> Result<AdmissionDecision, RuntimeError>;
}

#[derive(Clone, Default)]
pub struct WorkerControl {
    inner: Arc<WorkerControlInner>,
}

#[derive(Default)]
struct WorkerControlInner {
    draining: AtomicBool,
    notify: Notify,
}

impl WorkerControl {
    pub fn request_drain(&self) {
        if !self.inner.draining.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::SeqCst)
    }

    async fn wait_for_drain(&self) {
        if self.is_draining() {
            return;
        }
        self.inner.notify.notified().await;
    }
}

#[derive(Clone, Default)]
pub struct CancellationFlag {
    cancelled: Arc<AtomicBool>,
}

impl CancellationFlag {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLoopConfig {
    pub owner: String,
    pub allowed_kinds: Vec<JobKind>,
    pub lease_duration: Duration,
    pub heartbeat_interval: Duration,
    pub idle_poll_interval: Duration,
    pub drain_timeout: Duration,
}

impl WorkerLoopConfig {
    pub fn conservative_v0(owner: impl Into<String>, allowed_kinds: Vec<JobKind>) -> Self {
        Self {
            owner: owner.into(),
            allowed_kinds,
            lease_duration: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(5),
            idle_poll_interval: Duration::from_millis(250),
            drain_timeout: Duration::from_secs(20),
        }
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.owner.is_empty() || self.owner.len() > 128 {
            return Err(RuntimeError::new(
                "invalid_worker_owner",
                "worker owner must be 1..=128 bytes",
            ));
        }
        if self.allowed_kinds.is_empty() {
            return Err(RuntimeError::new(
                "empty_worker_job_classes",
                "worker must declare at least one allowed job class",
            ));
        }
        if self.lease_duration.is_zero() || self.lease_duration > Duration::from_secs(300) {
            return Err(RuntimeError::new(
                "invalid_worker_lease",
                "lease duration must be >0 and <=300 seconds",
            ));
        }
        if self.heartbeat_interval.is_zero()
            || self.heartbeat_interval >= self.lease_duration
        {
            return Err(RuntimeError::new(
                "invalid_worker_heartbeat",
                "heartbeat interval must be >0 and shorter than the lease",
            ));
        }
        if self.idle_poll_interval.is_zero() || self.idle_poll_interval > Duration::from_secs(30) {
            return Err(RuntimeError::new(
                "invalid_worker_idle_poll",
                "idle poll interval must be >0 and <=30 seconds",
            ));
        }
        if self.drain_timeout.is_zero() || self.drain_timeout > Duration::from_secs(300) {
            return Err(RuntimeError::new(
                "invalid_worker_drain_timeout",
                "drain timeout must be >0 and <=300 seconds",
            ));
        }
        Ok(())
    }

    fn lease_ms(&self) -> Result<i64, RuntimeError> {
        duration_ms(self.lease_duration, "worker lease")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerRunReceipt {
    pub claimed: u64,
    pub admitted: u64,
    pub admission_requeued: u64,
    pub admission_rejected: u64,
    pub succeeded: u64,
    pub already_published: u64,
    pub requeued: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub lease_lost: u64,
    pub drain_deadline_abandoned: u64,
}

pub struct WorkerLoop {
    queue: SqliteJobQueue,
    executor: Arc<dyn JobExecutor>,
    admission: Arc<dyn JobAdmission>,
    control: WorkerControl,
    config: WorkerLoopConfig,
}

impl WorkerLoop {
    pub fn new(
        queue: SqliteJobQueue,
        executor: Arc<dyn JobExecutor>,
        admission: Arc<dyn JobAdmission>,
        control: WorkerControl,
        config: WorkerLoopConfig,
    ) -> Result<Self, RuntimeError> {
        config.validate()?;
        Ok(Self {
            queue,
            executor,
            admission,
            control,
            config,
        })
    }

    pub fn control(&self) -> WorkerControl {
        self.control.clone()
    }

    pub async fn run(&self) -> Result<WorkerRunReceipt, RuntimeError> {
        let mut receipt = WorkerRunReceipt::default();
        let lease_ms = self.config.lease_ms()?;

        while !self.control.is_draining() {
            let now_ms = unix_now_ms()?;
            let lease = self
                .queue
                .claim_one(
                    &self.config.owner,
                    now_ms,
                    lease_ms,
                    &self.config.allowed_kinds,
                )
                .await
                .map_err(queue_error)?;

            let Some(lease) = lease else {
                tokio::select! {
                    _ = sleep(self.config.idle_poll_interval) => {}
                    _ = self.control.wait_for_drain() => {}
                }
                continue;
            };

            receipt.claimed += 1;

            match self.admission.admit(&lease.job)? {
                AdmissionDecision::Admit => {
                    receipt.admitted += 1;
                }
                AdmissionDecision::RetryLater { code } => {
                    match self
                        .queue
                        .fail(&lease, unix_now_ms()?, true, code)
                        .await
                        .map_err(queue_error)?
                    {
                        FailureOutcome::Requeued(_) => receipt.admission_requeued += 1,
                        FailureOutcome::Failed(_) => receipt.failed += 1,
                        FailureOutcome::Cancelled(_) => receipt.cancelled += 1,
                    }
                    continue;
                }
                AdmissionDecision::Reject { code } => {
                    match self
                        .queue
                        .fail(&lease, unix_now_ms()?, false, code)
                        .await
                        .map_err(queue_error)?
                    {
                        FailureOutcome::Requeued(_) => receipt.requeued += 1,
                        FailureOutcome::Failed(_) => receipt.admission_rejected += 1,
                        FailureOutcome::Cancelled(_) => receipt.cancelled += 1,
                    }
                    continue;
                }
            }

            match self.execute_lease(&lease).await? {
                ExecutionCompletion::Succeeded {
                    already_published: false,
                } => receipt.succeeded += 1,
                ExecutionCompletion::Succeeded {
                    already_published: true,
                } => receipt.already_published += 1,
                ExecutionCompletion::Requeued => receipt.requeued += 1,
                ExecutionCompletion::Failed => receipt.failed += 1,
                ExecutionCompletion::Cancelled => receipt.cancelled += 1,
                ExecutionCompletion::LeaseLost => receipt.lease_lost += 1,
                ExecutionCompletion::DrainDeadline => {
                    receipt.drain_deadline_abandoned += 1;
                    break;
                }
            }
        }

        Ok(receipt)
    }

    async fn execute_lease(&self, lease: &Lease) -> Result<ExecutionCompletion, RuntimeError> {
        let cancellation = CancellationFlag::default();
        let execution = self.executor.execute(&lease.job, cancellation.clone());
        tokio::pin!(execution);

        let mut heartbeat = interval(self.config.heartbeat_interval);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        heartbeat.tick().await;

        let far_future = Duration::from_secs(365 * 24 * 60 * 60);
        let drain_sleep = sleep(far_future);
        tokio::pin!(drain_sleep);
        let mut drain_started = false;

        loop {
            tokio::select! {
                result = &mut execution => {
                    return self.finish_execution(lease, result).await;
                }
                _ = heartbeat.tick() => {
                    let now_ms = unix_now_ms()?;
                    match self.queue.heartbeat(
                        &lease.job.job_id,
                        &lease.lease_owner,
                        lease.lease_generation,
                        now_ms,
                        self.config.lease_ms()?,
                    ).await {
                        Ok(job) => {
                            if job.cancel_requested_at_ms.is_some() {
                                cancellation.cancel();
                            }
                        }
                        Err(error) if error.code == "stale_lease" => {
                            cancellation.cancel();
                            return Ok(ExecutionCompletion::LeaseLost);
                        }
                        Err(error) => return Err(queue_error(error)),
                    }
                }
                _ = self.control.wait_for_drain(), if !drain_started => {
                    drain_started = true;
                    cancellation.cancel();
                    drain_sleep.as_mut().reset(Instant::now() + self.config.drain_timeout);
                }
                _ = &mut drain_sleep, if drain_started => {
                    cancellation.cancel();
                    return Ok(ExecutionCompletion::DrainDeadline);
                }
            }
        }
    }

    async fn finish_execution(
        &self,
        lease: &Lease,
        result: Result<JobSuccess, JobFailure>,
    ) -> Result<ExecutionCompletion, RuntimeError> {
        let now_ms = unix_now_ms()?;
        match result {
            Ok(success) => {
                match self
                    .queue
                    .publish_success(lease, now_ms, &success.effect_key)
                    .await
                {
                    Ok(PublishOutcome::Published(_)) => Ok(ExecutionCompletion::Succeeded {
                        already_published: false,
                    }),
                    Ok(PublishOutcome::AlreadyPublished(_)) => Ok(ExecutionCompletion::Succeeded {
                        already_published: true,
                    }),
                    Err(error) if error.code == "stale_lease" => {
                        Ok(ExecutionCompletion::LeaseLost)
                    }
                    Err(error) if error.code == "cancel_requested" => {
                        match self
                            .queue
                            .fail(lease, now_ms, false, "cancel_requested")
                            .await
                            .map_err(queue_error)?
                        {
                            FailureOutcome::Cancelled(_) => Ok(ExecutionCompletion::Cancelled),
                            FailureOutcome::Failed(_) => Ok(ExecutionCompletion::Failed),
                            FailureOutcome::Requeued(_) => Ok(ExecutionCompletion::Requeued),
                        }
                    }
                    Err(error) => Err(queue_error(error)),
                }
            }
            Err(failure) => {
                match self
                    .queue
                    .fail(
                        lease,
                        now_ms,
                        failure.retryable,
                        failure.terminal_code,
                    )
                    .await
                    .map_err(queue_error)?
                {
                    FailureOutcome::Requeued(_) => Ok(ExecutionCompletion::Requeued),
                    FailureOutcome::Failed(_) => Ok(ExecutionCompletion::Failed),
                    FailureOutcome::Cancelled(_) => Ok(ExecutionCompletion::Cancelled),
                }
            }
        }
    }
}

enum ExecutionCompletion {
    Succeeded { already_published: bool },
    Requeued,
    Failed,
    Cancelled,
    LeaseLost,
    DrainDeadline,
}

fn duration_ms(duration: Duration, label: &str) -> Result<i64, RuntimeError> {
    i64::try_from(duration.as_millis()).map_err(|_| {
        RuntimeError::new(
            "worker_duration_overflow",
            format!("{label} does not fit in i64 milliseconds"),
        )
    })
}

fn unix_now_ms() -> Result<i64, RuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::new("clock_before_epoch", "system clock is before UNIX epoch"))?;
    duration_ms(duration, "system clock")
}

fn queue_error(error: crate::job_queue::JobQueueError) -> RuntimeError {
    RuntimeError::new(error.code, error.message)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use tokio::sync::Notify;

    use super::*;
    use crate::job_queue::{EnqueueRequest, JobStatus};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    async fn queue() -> (SqliteJobQueue, PathBuf) {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("chaptera-worker-{}-{n}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let queue = SqliteJobQueue::open(&path, 4, Duration::from_secs(2))
            .await
            .unwrap();
        (queue, path)
    }

    async fn enqueue(queue: &SqliteJobQueue, id: &str) {
        queue
            .enqueue(EnqueueRequest {
                job_id: id.into(),
                tenant_id: "tenant-a".into(),
                job_kind: JobKind::Export,
                payload_schema_version: 1,
                payload: br#"{"revision":"rev-1"}"#.to_vec(),
                idempotency_key: format!("idem-{id}"),
                max_attempts: 3,
                now_ms: unix_now_ms().unwrap(),
            })
            .await
            .unwrap();
    }

    fn config(owner: &str) -> WorkerLoopConfig {
        WorkerLoopConfig {
            owner: owner.into(),
            allowed_kinds: vec![JobKind::Export],
            lease_duration: Duration::from_millis(500),
            heartbeat_interval: Duration::from_millis(50),
            idle_poll_interval: Duration::from_millis(10),
            drain_timeout: Duration::from_millis(100),
        }
    }

    struct Allow;

    impl JobAdmission for Allow {
        fn admit(&self, _job: &JobRecord) -> Result<AdmissionDecision, RuntimeError> {
            Ok(AdmissionDecision::Admit)
        }
    }

    struct Reject;

    impl JobAdmission for Reject {
        fn admit(&self, _job: &JobRecord) -> Result<AdmissionDecision, RuntimeError> {
            Ok(AdmissionDecision::Reject {
                code: "quota_denied",
            })
        }
    }

    struct ImmediateSuccess;

    impl JobExecutor for ImmediateSuccess {
        fn execute<'a>(
            &'a self,
            job: &'a JobRecord,
            _cancellation: CancellationFlag,
        ) -> JobFuture<'a> {
            let effect = format!("effect-{}", job.job_id);
            Box::pin(async move { Ok(JobSuccess { effect_key: effect }) })
        }
    }

    struct HangingExecutor {
        started: Arc<Notify>,
        cancelled_seen: Arc<AtomicBool>,
    }

    impl JobExecutor for HangingExecutor {
        fn execute<'a>(
            &'a self,
            _job: &'a JobRecord,
            cancellation: CancellationFlag,
        ) -> JobFuture<'a> {
            let started = self.started.clone();
            let seen = self.cancelled_seen.clone();
            Box::pin(async move {
                started.notify_one();
                loop {
                    if cancellation.is_cancelled() {
                        seen.store(true, Ordering::SeqCst);
                    }
                    sleep(Duration::from_millis(5)).await;
                }
            })
        }
    }

    #[tokio::test]
    async fn successful_job_publishes_once_then_worker_drains() {
        let (queue, path) = queue().await;
        enqueue(&queue, "job-success").await;
        let control = WorkerControl::default();
        let worker = Arc::new(
            WorkerLoop::new(
                queue.clone(),
                Arc::new(ImmediateSuccess),
                Arc::new(Allow),
                control.clone(),
                config("worker-success"),
            )
            .unwrap(),
        );

        let task = {
            let worker = worker.clone();
            tokio::spawn(async move { worker.run().await.unwrap() })
        };

        for _ in 0..100 {
            let job = queue.get("job-success").await.unwrap().unwrap();
            if job.status == JobStatus::Succeeded {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        control.request_drain();
        let receipt = task.await.unwrap();
        let job = queue.get("job-success").await.unwrap().unwrap();

        assert_eq!(job.status, JobStatus::Succeeded);
        assert_eq!(receipt.claimed, 1);
        assert_eq!(receipt.succeeded, 1);

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn admission_rejection_is_terminal_and_executor_never_runs() {
        let (queue, path) = queue().await;
        enqueue(&queue, "job-reject").await;
        let control = WorkerControl::default();
        let worker = Arc::new(
            WorkerLoop::new(
                queue.clone(),
                Arc::new(ImmediateSuccess),
                Arc::new(Reject),
                control.clone(),
                config("worker-reject"),
            )
            .unwrap(),
        );

        let task = {
            let worker = worker.clone();
            tokio::spawn(async move { worker.run().await.unwrap() })
        };

        for _ in 0..100 {
            let job = queue.get("job-reject").await.unwrap().unwrap();
            if job.status == JobStatus::Failed {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
        control.request_drain();
        let receipt = task.await.unwrap();
        let job = queue.get("job-reject").await.unwrap().unwrap();

        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.terminal_code.as_deref(), Some("quota_denied"));
        assert_eq!(receipt.admission_rejected, 1);
        assert_eq!(receipt.succeeded, 0);

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn drain_deadline_stops_heartbeat_and_leaves_lease_reclaimable() {
        let (queue, path) = queue().await;
        enqueue(&queue, "job-drain").await;
        let started = Arc::new(Notify::new());
        let cancelled_seen = Arc::new(AtomicBool::new(false));
        let control = WorkerControl::default();
        let worker = Arc::new(
            WorkerLoop::new(
                queue.clone(),
                Arc::new(HangingExecutor {
                    started: started.clone(),
                    cancelled_seen: cancelled_seen.clone(),
                }),
                Arc::new(Allow),
                control.clone(),
                config("worker-drain"),
            )
            .unwrap(),
        );

        let task = {
            let worker = worker.clone();
            tokio::spawn(async move { worker.run().await.unwrap() })
        };

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        control.request_drain();
        let receipt = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(receipt.drain_deadline_abandoned, 1);
        assert!(cancelled_seen.load(Ordering::SeqCst));

        let running = queue.get("job-drain").await.unwrap().unwrap();
        assert_eq!(running.status, JobStatus::Running);
        let expiry = running.lease_expires_at_ms.unwrap();
        let wait_ms = (expiry - unix_now_ms().unwrap()).max(0) as u64 + 10;
        sleep(Duration::from_millis(wait_ms)).await;

        let reclaimed = queue
            .claim_one(
                "worker-restart",
                unix_now_ms().unwrap(),
                500,
                &[JobKind::Export],
            )
            .await
            .unwrap()
            .expect("expired lease must be reclaimable");
        assert!(reclaimed.lease_generation > running.lease_generation);

        queue.close().await;
        let _ = std::fs::remove_file(path);
    }
}
