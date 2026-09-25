use std::{error::Error, process::ExitCode, sync::Arc, time::Duration};

use chaptera_server::{
    auth_runtime::AuthRuntime,
    authz_runtime::SqliteAuthzAuthority,
    blob_runtime::BlobStoreRuntime,
    cli::{Cli, Command},
    config::{ChapteraConfig, EnvironmentMode, SecretResolver},
    doctor,
    edge::EdgePolicy,
    jobs::UnconfiguredWorkerRuntime,
    jobs_runtime::JobsRuntime,
    migrate,
    project_persistence_sqlite::SqliteProjectPersistence,
    runtime_readiness::{ports_with_configured_serve, ports_with_revision_stream},
    schema_migration::SqliteMigrationRuntime,
    serve, source_baseline,
    source_baseline::IsolatedSourceBaselineProducer,
    source_ingress_http::{self, SourceIngressHttpState},
    source_ingress_sqlite::SqliteSourceIngressRepository,
    sqlite_store::SqliteRevisionStore,
    state::{AppState, RuntimePorts},
    upload_admission::SqliteUploadAdmissionAuthority,
    worker,
    worker_runtime::ConfiguredWorkerRuntime,
    workspace_context::SqliteWorkspaceContextResolver,
};
use clap::Parser;

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Command::SourceBaseline {
        document_id,
        expected_sha256,
        expected_byte_len,
    } = &cli.command
    {
        return match source_baseline::run_source_baseline_worker(
            document_id,
            expected_sha256,
            *expected_byte_len,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("chaptera: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("chaptera: failed to initialize runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chaptera: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    let explicit_config = cli
        .config
        .as_deref()
        .map(ChapteraConfig::load)
        .transpose()?;

    match cli.command {
        Command::Serve => {
            let config = match explicit_config.as_ref() {
                Some(config) => config.clone(),
                None => ChapteraConfig::development_from_env()?,
            };
            let secrets = config.resolve_required_secrets(&SecretResolver::from_process())?;

            let edge_policy = EdgePolicy::from_config(&config)?;
            if explicit_config.is_some() {
                let revision_stream = SqliteRevisionStore::open(
                    &config.sqlite.path,
                    config.sqlite.pool_max,
                    Duration::from_millis(config.sqlite.busy_timeout_ms),
                )
                .await?;
                if config.auth.is_some() {
                    let auth_runtime = AuthRuntime::open(&config, &secrets).await?;
                    drop(secrets);
                    let auth_http = auth_runtime.http_state();
                    let busy_timeout = Duration::from_millis(config.sqlite.busy_timeout_ms);
                    let authz = SqliteAuthzAuthority::open(
                        &config.sqlite.path,
                        config.sqlite.pool_max,
                        busy_timeout,
                    )
                    .await?;
                    let jobs = JobsRuntime::open_with_authz(
                        &config.sqlite.path,
                        config.sqlite.pool_max,
                        busy_timeout,
                        authz.clone(),
                    )
                    .await?;
                    let blob_store = BlobStoreRuntime::open(&config).await?;
                    let product_router = if let Some(source_config) = &config.source_ingress {
                        let workspace = SqliteWorkspaceContextResolver::open(
                            &config.sqlite.path,
                            config.sqlite.pool_max,
                            busy_timeout,
                        )
                        .await?;
                        let admission = SqliteUploadAdmissionAuthority::open(
                            &config.sqlite.path,
                            config.sqlite.pool_max,
                            busy_timeout,
                            source_ingress_http::upload_admission_config(source_config),
                        )
                        .await?;
                        let source_repo = SqliteSourceIngressRepository::open(
                            &config.sqlite.path,
                            config.sqlite.pool_max,
                            busy_timeout,
                        )
                        .await?;
                        let scanner =
                            Arc::new(source_ingress_http::production_scanner(source_config)?);
                        let baseline = IsolatedSourceBaselineProducer::new(
                            source_ingress_http::baseline_config(source_config),
                            blob_store.service().clone(),
                        )?;
                        let projects = SqliteProjectPersistence::open(
                            &config.sqlite.path,
                            config.sqlite.pool_max,
                            busy_timeout,
                        )
                        .await?;
                        let source_state = SourceIngressHttpState::new(
                            auth_http.clone(),
                            workspace,
                            admission,
                            source_repo,
                            blob_store.service().clone(),
                            scanner,
                            baseline,
                            projects,
                            source_ingress_http::http_config(source_config),
                        )?;
                        Some(source_ingress_http::router(source_state))
                    } else {
                        None
                    };

                    let assembled = ports_with_configured_serve(
                        revision_stream,
                        auth_runtime,
                        authz,
                        jobs,
                        blob_store,
                    );
                    let state = AppState::new(assembled.ports);
                    serve::run_with_auth_local_product(
                        config.runtime_config(),
                        edge_policy,
                        state,
                        Some(auth_http),
                        !matches!(config.environment, EnvironmentMode::Prod),
                        product_router,
                    )
                    .await?;
                } else {
                    drop(secrets);
                    let assembled = ports_with_revision_stream(revision_stream);
                    let state = AppState::new(assembled.ports);
                    serve::run_with_auth_local(
                        config.runtime_config(),
                        edge_policy,
                        state,
                        None,
                        !matches!(config.environment, EnvironmentMode::Prod),
                    )
                    .await?;
                }
            } else {
                drop(secrets);
                let state = AppState::new(RuntimePorts::unconfigured());
                serve::run_with_auth_local(config.runtime_config(), edge_policy, state, None, true)
                    .await?;
            }
        }
        Command::Worker => {
            if let Some(config) = explicit_config.as_ref() {
                // The background worker consumes durable AuthZ grants, not the
                // browser OIDC client secret. Do not resolve web-auth secrets
                // here merely for symmetry with serve; BlobStore credentials
                // are owned by the AWS SDK provider credential chain.
                let runtime = ConfiguredWorkerRuntime::new(config.clone());
                worker::run(&runtime).await?;
            } else {
                worker::run(&UnconfiguredWorkerRuntime).await?;
            }
        }
        Command::Migrate { action } => {
            if let Some(config) = explicit_config.as_ref() {
                let runtime = SqliteMigrationRuntime::new(
                    &config.sqlite.path,
                    Duration::from_millis(config.sqlite.busy_timeout_ms),
                )?;
                migrate::run(action, &runtime).await?;
            } else {
                let runtime = SqliteMigrationRuntime::from_env()?;
                migrate::run(action, &runtime).await?;
            }
        }
        Command::Doctor => {
            if let Some(config) = explicit_config.as_ref() {
                let secrets = config.resolve_required_secrets(&SecretResolver::from_process())?;
                drop(secrets);
            }
            doctor::run(&AppState::new(RuntimePorts::unconfigured()))?;
        }
        Command::SourceBaseline { .. } => unreachable!(
            "source-baseline is dispatched synchronously before Tokio runtime creation"
        ),
    }

    Ok(())
}
