use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use forge_agent::{
    AnthropicProviderProfile, CxdbPersistenceMode as AgentCxdbPersistenceMode,
    LocalExecutionEnvironment, OpenAiProviderProfile, ProviderProfile, Session, SessionConfig,
};
use forge_attractor::forge_agent::{ForgeAgentCodergenAdapter, ForgeAgentSessionBackend};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::handlers::wait_human::{
    AutoApproveInterviewer, ConsoleInterviewer, HumanAnswer, QueueInterviewer, WaitHumanHandler,
};
use forge_attractor::{
    CheckpointState, CxdbPersistenceMode as AttractorCxdbPersistenceMode, PipelineRunResult,
    PipelineRunner, PipelineStatus, RunConfig, RuntimeEvent, RuntimeEventKind, RuntimeEventSink,
    parse_dot, runtime_event_channel,
};
use forge_llm::Client;
use forge_turnstore_cxdb::{
    CxdbBinaryClient, CxdbHttpClient, CxdbReqwestHttpClient, CxdbSdkBinaryClient,
    DEFAULT_CXDB_BINARY_ADDR, DEFAULT_CXDB_HTTP_BASE_URL,
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "forge-cli")]
#[command(about = "In-process CLI host for Forge Attractor")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run(RunArgs),
    Resume(ResumeArgs),
    InspectCheckpoint(InspectCheckpointArgs),
}

#[derive(clap::Args, Debug)]
struct RunArgs {
    #[arg(long)]
    dot_file: Option<PathBuf>,
    #[arg(long)]
    dot_source: Option<String>,
    #[arg(long)]
    run_id: Option<String>,
    #[arg(long)]
    logs_root: Option<PathBuf>,
    #[arg(long = "no-stream-events", action = ArgAction::SetTrue)]
    no_stream_events: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    event_json: bool,
    #[arg(long, value_enum, default_value_t = InterviewerMode::Auto)]
    interviewer: InterviewerMode,
    #[arg(long, value_enum, default_value_t = BackendMode::Agent)]
    backend: BackendMode,
    #[arg(long = "human-answer")]
    human_answers: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct ResumeArgs {
    #[arg(long)]
    dot_file: Option<PathBuf>,
    #[arg(long)]
    dot_source: Option<String>,
    #[arg(long)]
    checkpoint: PathBuf,
    #[arg(long)]
    run_id: Option<String>,
    #[arg(long)]
    logs_root: Option<PathBuf>,
    #[arg(long = "no-stream-events", action = ArgAction::SetTrue)]
    no_stream_events: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    event_json: bool,
    #[arg(long, value_enum, default_value_t = InterviewerMode::Auto)]
    interviewer: InterviewerMode,
    #[arg(long, value_enum, default_value_t = BackendMode::Agent)]
    backend: BackendMode,
    #[arg(long = "human-answer")]
    human_answers: Vec<String>,
}

#[derive(clap::Args, Debug)]
struct InspectCheckpointArgs {
    #[arg(long)]
    checkpoint: PathBuf,
    #[arg(long, action = ArgAction::SetTrue)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InterviewerMode {
    Auto,
    Console,
    Queue,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackendMode {
    Agent,
    Mock,
}

#[derive(Clone, Debug)]
struct CxdbHostConfig {
    persistence: AttractorCxdbPersistenceMode,
    binary_addr: String,
    http_base_url: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    load_env_files();
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Run(args) => run_command(args).await,
        Commands::Resume(args) => resume_command(args).await,
        Commands::InspectCheckpoint(args) => inspect_checkpoint_command(args),
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

fn load_env_files() {
    let _ = dotenvy::from_filename(".env.local");
    let _ = dotenvy::from_filename(".env");
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn cxdb_host_config_from_env() -> Result<CxdbHostConfig, String> {
    let persistence_raw = first_non_empty_env(&["FORGE_CXDB_PERSISTENCE", "CXDB_PERSISTENCE_MODE"])
        .unwrap_or_else(|| "off".to_string())
        .to_ascii_lowercase();
    let persistence = match persistence_raw.as_str() {
        "off" => AttractorCxdbPersistenceMode::Off,
        "required" => AttractorCxdbPersistenceMode::Required,
        _ => {
            return Err(format!(
                "invalid FORGE_CXDB_PERSISTENCE value '{}'; expected 'off' or 'required'",
                persistence_raw
            ));
        }
    };

    let binary_addr =
        first_non_empty_env(&["FORGE_CXDB_BINARY_ADDR", "CXDB_BINARY_ADDR", "CXDB_ADDR"])
            .unwrap_or_else(|| DEFAULT_CXDB_BINARY_ADDR.to_string());
    let http_base_url = first_non_empty_env(&["FORGE_CXDB_HTTP_BASE_URL", "CXDB_HTTP_BASE_URL"])
        .unwrap_or_else(|| DEFAULT_CXDB_HTTP_BASE_URL.to_string());

    Ok(CxdbHostConfig {
        persistence,
        binary_addr,
        http_base_url,
    })
}

fn build_cxdb_clients(
    cxdb: &CxdbHostConfig,
) -> Result<(Arc<dyn CxdbBinaryClient>, Arc<dyn CxdbHttpClient>), String> {
    let binary: Arc<dyn CxdbBinaryClient> = Arc::new(
        CxdbSdkBinaryClient::connect(&cxdb.binary_addr).map_err(|error| {
            format!(
                "failed to connect CXDB binary '{}': {error}",
                cxdb.binary_addr
            )
        })?,
    );
    let http: Arc<dyn CxdbHttpClient> =
        Arc::new(CxdbReqwestHttpClient::new(cxdb.http_base_url.clone()));
    Ok((binary, http))
}

fn build_runtime_persistence(
    cxdb: &CxdbHostConfig,
) -> Result<
    (
        Option<forge_attractor::SharedAttractorStorageWriter>,
        Option<Arc<dyn forge_attractor::AttractorArtifactWriter>>,
    ),
    String,
> {
    if cxdb.persistence == AttractorCxdbPersistenceMode::Off {
        return Ok((None, None));
    }

    let (binary, http) = build_cxdb_clients(cxdb)?;
    let storage = forge_attractor::cxdb_storage_writer(binary.clone(), http.clone());
    let artifacts = forge_attractor::cxdb_artifact_writer(binary, http);
    Ok((Some(storage), Some(artifacts)))
}

async fn run_command(args: RunArgs) -> Result<ExitCode, String> {
    let source = load_dot_source(args.dot_file.as_deref(), args.dot_source.as_deref())?;
    let graph = parse_dot(&source).map_err(|error| error.to_string())?;
    let cxdb = cxdb_host_config_from_env()?;
    let (storage, artifacts) = build_runtime_persistence(&cxdb)?;

    let (event_sink, event_task) = event_stream(!args.no_stream_events, args.event_json);

    let executor = build_executor(
        args.interviewer,
        args.backend,
        args.human_answers,
        &cxdb,
        storage.clone(),
    )?;
    let run_result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: args.run_id,
                logs_root: args.logs_root,
                events: event_sink,
                executor,
                storage,
                artifacts,
                cxdb_persistence: cxdb.persistence,
                ..RunConfig::default()
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    if let Some(task) = event_task {
        task.await.map_err(|error| error.to_string())?;
    }

    print_run_summary(&run_result);
    Ok(exit_code_for_status(run_result.status))
}

async fn resume_command(args: ResumeArgs) -> Result<ExitCode, String> {
    let source = load_dot_source(args.dot_file.as_deref(), args.dot_source.as_deref())?;
    let graph = parse_dot(&source).map_err(|error| error.to_string())?;
    let cxdb = cxdb_host_config_from_env()?;
    let (storage, artifacts) = build_runtime_persistence(&cxdb)?;

    let (event_sink, event_task) = event_stream(!args.no_stream_events, args.event_json);

    let executor = build_executor(
        args.interviewer,
        args.backend,
        args.human_answers,
        &cxdb,
        storage.clone(),
    )?;
    let run_result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: args.run_id,
                logs_root: args.logs_root,
                resume_from_checkpoint: Some(args.checkpoint),
                events: event_sink,
                executor,
                storage,
                artifacts,
                cxdb_persistence: cxdb.persistence,
                ..RunConfig::default()
            },
        )
        .await
        .map_err(|error| error.to_string())?;

    if let Some(task) = event_task {
        task.await.map_err(|error| error.to_string())?;
    }

    print_run_summary(&run_result);
    Ok(exit_code_for_status(run_result.status))
}

fn inspect_checkpoint_command(args: InspectCheckpointArgs) -> Result<ExitCode, String> {
    let checkpoint =
        CheckpointState::load_from_path(&args.checkpoint).map_err(|e| e.to_string())?;
    if args.json {
        let json = serde_json::to_string_pretty(&checkpoint).map_err(|e| e.to_string())?;
        println!("{json}");
    } else {
        println!("checkpoint: {}", args.checkpoint.display());
        println!("run_id: {}", checkpoint.metadata.run_id);
        println!("checkpoint_id: {}", checkpoint.metadata.checkpoint_id);
        println!("sequence_no: {}", checkpoint.metadata.sequence_no);
        println!("timestamp: {}", checkpoint.metadata.timestamp);
        println!("current_node: {}", checkpoint.current_node);
        println!(
            "next_node: {}",
            checkpoint.next_node.as_deref().unwrap_or("<none>")
        );
        println!("completed_nodes: {}", checkpoint.completed_nodes.len());
        println!("context_keys: {}", checkpoint.context_values.len());
        println!("log_entries: {}", checkpoint.logs.len());
        println!(
            "terminal_status: {}",
            checkpoint
                .terminal_status
                .as_deref()
                .unwrap_or("<in_progress>")
        );
        if let Some(reason) = checkpoint.terminal_failure_reason.as_deref() {
            println!("failure_reason: {reason}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn load_dot_source(dot_file: Option<&Path>, dot_source: Option<&str>) -> Result<String, String> {
    match (dot_file, dot_source) {
        (Some(_), Some(_)) => Err("provide only one of --dot-file or --dot-source".to_string()),
        (None, None) => Err("one of --dot-file or --dot-source is required".to_string()),
        (Some(path), None) => std::fs::read_to_string(path)
            .map_err(|e| format!("failed reading DOT file '{}': {e}", path.display())),
        (None, Some(source)) => Ok(source.to_string()),
    }
}

fn event_stream(
    stream_events: bool,
    event_json: bool,
) -> (RuntimeEventSink, Option<tokio::task::JoinHandle<()>>) {
    if !stream_events {
        return (RuntimeEventSink::default(), None);
    }

    let (tx, mut rx) = runtime_event_channel();
    let task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if event_json {
                match serde_json::to_string(&event) {
                    Ok(line) => println!("{line}"),
                    Err(_) => print_event_text(&event),
                }
            } else {
                print_event_text(&event);
            }
        }
    });
    (RuntimeEventSink::with_sender(tx), Some(task))
}

fn build_executor(
    mode: InterviewerMode,
    backend_mode: BackendMode,
    human_answers: Vec<String>,
    cxdb: &CxdbHostConfig,
    stage_link_writer: Option<forge_attractor::SharedAttractorStorageWriter>,
) -> Result<Arc<dyn forge_attractor::NodeExecutor>, String> {
    let interviewer: Arc<dyn forge_attractor::Interviewer> = match mode {
        InterviewerMode::Auto => {
            if is_interactive_terminal() {
                Arc::new(ConsoleInterviewer)
            } else {
                Arc::new(AutoApproveInterviewer)
            }
        }
        InterviewerMode::Console => Arc::new(ConsoleInterviewer),
        InterviewerMode::Queue => {
            let answers = human_answers.into_iter().map(HumanAnswer::Selected);
            Arc::new(QueueInterviewer::with_answers(answers))
        }
    };

    let codergen_backend = match backend_mode {
        BackendMode::Mock => None,
        BackendMode::Agent => Some(build_agent_codergen_backend(cxdb, stage_link_writer)?),
    };
    let mut registry =
        forge_attractor::handlers::core_registry_with_codergen_backend(codergen_backend);
    registry.register_type("wait.human", Arc::new(WaitHumanHandler::new(interviewer)));
    Ok(Arc::new(RegistryNodeExecutor::new(registry)))
}

fn print_event_text(event: &RuntimeEvent) {
    println!(
        "[event seq={}] {} {}",
        event.sequence_no,
        event.timestamp,
        event_kind_label(&event.kind)
    );
}

fn event_kind_label(kind: &RuntimeEventKind) -> &'static str {
    match kind {
        RuntimeEventKind::Pipeline(_) => "pipeline",
        RuntimeEventKind::Stage(_) => "stage",
        RuntimeEventKind::Parallel(_) => "parallel",
        RuntimeEventKind::Interview(_) => "interview",
        RuntimeEventKind::Checkpoint(_) => "checkpoint",
    }
}

fn print_run_summary(result: &PipelineRunResult) {
    println!("run_id: {}", result.run_id);
    println!(
        "status: {}",
        match result.status {
            PipelineStatus::Success => "success",
            PipelineStatus::Fail => "fail",
        }
    );
    println!("completed_nodes: {}", result.completed_nodes.join(", "));
    if let Some(reason) = result.failure_reason.as_deref() {
        println!("failure_reason: {reason}");
    }
}

fn exit_code_for_status(status: PipelineStatus) -> ExitCode {
    match status {
        PipelineStatus::Success => ExitCode::SUCCESS,
        PipelineStatus::Fail => ExitCode::from(2),
    }
}

fn is_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn build_agent_codergen_backend(
    cxdb: &CxdbHostConfig,
    stage_link_writer: Option<forge_attractor::SharedAttractorStorageWriter>,
) -> Result<Arc<dyn forge_attractor::handlers::codergen::CodergenBackend>, String> {
    let provider_profile = select_provider_profile_from_env()?;
    let llm_client =
        Arc::new(Client::from_env().map_err(|error| {
            format!("failed to initialize LLM client from environment: {error}")
        })?);
    let cwd = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current directory for agent env: {error}"))?;
    let execution_env = Arc::new(LocalExecutionEnvironment::new(cwd));
    let mut session_config = SessionConfig::default();
    session_config.cxdb_persistence = if cxdb.persistence == AttractorCxdbPersistenceMode::Required
    {
        AgentCxdbPersistenceMode::Required
    } else {
        AgentCxdbPersistenceMode::Off
    };

    let session = if cxdb.persistence == AttractorCxdbPersistenceMode::Required {
        let (binary_client, http_client) = build_cxdb_clients(cxdb)?;
        Session::new_with_cxdb_persistence(
            provider_profile,
            execution_env,
            llm_client,
            session_config,
            binary_client,
            http_client,
        )
    } else {
        Session::new(provider_profile, execution_env, llm_client, session_config)
    }
    .map_err(|error| format!("failed to initialize forge-agent session: {error}"))?;

    let backend =
        ForgeAgentSessionBackend::new(ForgeAgentCodergenAdapter::default(), Box::new(session));
    let backend = if let Some(writer) = stage_link_writer {
        backend.with_stage_link_writer(writer, cxdb.persistence)
    } else {
        backend
    };
    Ok(Arc::new(backend))
}

fn select_provider_profile_from_env() -> Result<Arc<dyn ProviderProfile>, String> {
    if std::env::var("OPENAI_API_KEY").ok().is_some() {
        return Ok(Arc::new(OpenAiProviderProfile::with_default_tools(
            "gpt-5.2-codex",
        )));
    }
    if std::env::var("ANTHROPIC_API_KEY").ok().is_some() {
        return Ok(Arc::new(AnthropicProviderProfile::with_default_tools(
            "claude-sonnet-4.5",
        )));
    }

    Err(
        "no supported provider credentials found for agent backend; set OPENAI_API_KEY or ANTHROPIC_API_KEY, or pass --backend mock".to_string(),
    )
}
