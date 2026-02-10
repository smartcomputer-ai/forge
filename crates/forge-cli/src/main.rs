use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::handlers::wait_human::{
    AutoApproveInterviewer, ConsoleInterviewer, HumanAnswer, QueueInterviewer, WaitHumanHandler,
};
use forge_attractor::{
    CheckpointState, PipelineRunResult, PipelineRunner, PipelineStatus, RunConfig, RuntimeEvent,
    RuntimeEventKind, RuntimeEventSink, parse_dot, runtime_event_channel,
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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

async fn run_command(args: RunArgs) -> Result<ExitCode, String> {
    let source = load_dot_source(args.dot_file.as_deref(), args.dot_source.as_deref())?;
    let graph = parse_dot(&source).map_err(|error| error.to_string())?;

    let (event_sink, event_task) = event_stream(!args.no_stream_events, args.event_json);

    let executor = build_executor(args.interviewer, args.human_answers);
    let run_result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: args.run_id,
                logs_root: args.logs_root,
                events: event_sink,
                executor,
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

    let (event_sink, event_task) = event_stream(!args.no_stream_events, args.event_json);

    let executor = build_executor(args.interviewer, args.human_answers);
    let run_result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: args.run_id,
                logs_root: args.logs_root,
                resume_from_checkpoint: Some(args.checkpoint),
                events: event_sink,
                executor,
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
    human_answers: Vec<String>,
) -> Arc<dyn forge_attractor::NodeExecutor> {
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

    let mut registry = forge_attractor::handlers::core_registry();
    registry.register_type("wait.human", Arc::new(WaitHumanHandler::new(interviewer)));
    Arc::new(RegistryNodeExecutor::new(registry))
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
