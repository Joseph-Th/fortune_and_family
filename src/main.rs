//! Command-line adapter for creating, advancing, inspecting, and validating campaigns.

use civic_dynasty::core::StartingBackground;
use civic_dynasty::{
    CommandError, GameplayFindingSeverity, GameplayHarnessConfig, GameplayHarnessError,
    GameplayPersona, NewGameConfig, NewGameError, PersistenceError, PlayerCommand, Registry,
    SimulationError, advance_days, apply_player_command, build_campaign_projection, build_new_game,
    build_rivergate_registry, build_state_summary, load_state, render_campaign_html,
    render_gameplay_report, run_gameplay_harness, save_state, validate_invariants,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(name = "civic-dynasty")]
#[command(about = "Deterministic headless simulation for Civic Dynasty")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new Rivergate campaign.
    New {
        #[arg(long, default_value = "campaign.json")]
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value = "Valeri")]
        dynasty: String,
        #[arg(long, default_value = "Elian Valeri")]
        founder: String,
        #[arg(long, value_enum, default_value_t = BackgroundArg::Baker)]
        background: BackgroundArg,
        #[arg(long, default_value_t = 0)]
        advance: u32,
    },
    /// Advance an existing campaign and persist the result.
    Simulate {
        input: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        days: u32,
    },
    /// Print a campaign summary, market state, and recent chronicle.
    Summary {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Print the adapter-facing read-only campaign projection as JSON.
    Inspect { input: PathBuf },
    /// Generate a self-contained HTML campaign dashboard.
    Dashboard {
        input: PathBuf,
        #[arg(long, default_value = "campaign-dashboard.html")]
        output: PathBuf,
    },
    /// Apply one JSON-encoded player command and persist the result.
    Execute {
        input: PathBuf,
        #[arg(long)]
        command: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Load a campaign and run all debug invariant assertions.
    Validate { input: PathBuf },
    /// Run deterministic player agents and report gameplay reachability and system reactions.
    Playtest(PlaytestArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackgroundArg {
    Baker,
    ClothTrader,
    Blacksmith,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum GameplayPersonaArg {
    Steward,
    Entrepreneur,
    PowerBroker,
    Opportunist,
}

impl From<GameplayPersonaArg> for GameplayPersona {
    fn from(value: GameplayPersonaArg) -> Self {
        match value {
            GameplayPersonaArg::Steward => Self::Steward,
            GameplayPersonaArg::Entrepreneur => Self::Entrepreneur,
            GameplayPersonaArg::PowerBroker => Self::PowerBroker,
            GameplayPersonaArg::Opportunist => Self::Opportunist,
        }
    }
}

impl From<BackgroundArg> for StartingBackground {
    fn from(value: BackgroundArg) -> Self {
        match value {
            BackgroundArg::Baker => Self::Baker,
            BackgroundArg::ClothTrader => Self::ClothTrader,
            BackgroundArg::Blacksmith => Self::Blacksmith,
        }
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    NewGame(#[from] NewGameError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Simulation(#[from] SimulationError),
    #[error("failed to serialize summary: {source}")]
    SummarySerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize campaign projection: {source}")]
    ProjectionSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to render campaign dashboard: {source}")]
    DashboardSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to create output directory {path}: {source}")]
    OutputDirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write dashboard {path}: {source}")]
    DashboardWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse player command: {source}")]
    CommandParse {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    GameplayHarness(#[from] GameplayHarnessError),
    #[error("failed to serialize gameplay report: {source}")]
    GameplayReportSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write gameplay report {path}: {source}")]
    GameplayReportWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("gameplay quality gate failed: {reason}")]
    GameplayQualityGate { reason: String },
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    let registry = build_rivergate_registry();
    match cli.command {
        Command::New {
            output,
            seed,
            dynasty,
            founder,
            background,
            advance,
        } => {
            let config = NewGameConfig {
                seed,
                dynasty_name: dynasty,
                founder_name: founder,
                background: background.into(),
            };
            let mut state = build_new_game(&registry, config)?;
            if advance > 0 {
                advance_days(&registry, &mut state, advance)?;
            }
            save_state(&output, &state)?;
            print_human_summary(&registry, &state);
            println!("Saved {}", output.display());
        }
        Command::Simulate {
            input,
            output,
            days,
        } => {
            let mut state = load_state(&input)?;
            validate_invariants(&registry, &state);
            advance_days(&registry, &mut state, days)?;
            let output = output.unwrap_or(input);
            save_state(&output, &state)?;
            print_human_summary(&registry, &state);
            println!("Saved {}", output.display());
        }
        Command::Summary { input, json } => {
            let state = load_state(input)?;
            validate_invariants(&registry, &state);
            if json {
                let summary = serde_json::to_string_pretty(&build_state_summary(&registry, &state))
                    .map_err(|source| CliError::SummarySerialization { source })?;
                println!("{summary}");
            } else {
                print_human_summary(&registry, &state);
            }
        }
        Command::Inspect { input } => {
            let state = load_state(input)?;
            validate_invariants(&registry, &state);
            let projection = build_campaign_projection(&registry, &state);
            let json = serde_json::to_string_pretty(&projection)
                .map_err(|source| CliError::ProjectionSerialization { source })?;
            println!("{json}");
        }
        Command::Dashboard { input, output } => {
            let state = load_state(input)?;
            validate_invariants(&registry, &state);
            let html = render_campaign_html(&registry, &state)
                .map_err(|source| CliError::DashboardSerialization { source })?;
            ensure_output_parent(&output)?;
            std::fs::write(&output, html).map_err(|source| CliError::DashboardWrite {
                path: output.clone(),
                source,
            })?;
            println!("Wrote {}", output.display());
        }
        Command::Execute {
            input,
            command,
            output,
        } => {
            let mut state = load_state(&input)?;
            validate_invariants(&registry, &state);
            let command: PlayerCommand = serde_json::from_str(&command)
                .map_err(|source| CliError::CommandParse { source })?;
            let outcome = apply_player_command(&registry, &mut state, command)?;
            validate_invariants(&registry, &state);
            let output = output.unwrap_or(input);
            save_state(&output, &state)?;
            println!("{}", outcome.summary);
            print_human_summary(&registry, &state);
            println!("Saved {}", output.display());
        }
        Command::Validate { input } => {
            let state = load_state(&input)?;
            validate_invariants(&registry, &state);
            println!(
                "Validated {} at simulation day {} with schema version {}",
                input.display(),
                state.clock().day(),
                state.schema_version()
            );
        }
        Command::Playtest(args) => run_playtest(&registry, args)?,
    }
    Ok(())
}

#[derive(Args, Debug)]
struct PlaytestArgs {
    /// First deterministic campaign seed.
    #[arg(long, default_value_t = 1)]
    start_seed: u64,
    /// Number of consecutive seeds to run.
    #[arg(long, default_value_t = 1)]
    seeds: u16,
    /// Simulated days per campaign.
    #[arg(long, default_value_t = 1_080)]
    days: u32,
    /// Days advanced after each player decision.
    #[arg(long, default_value_t = 30)]
    decision_interval: u16,
    /// Maximum candidate commands validated per decision.
    #[arg(long, default_value_t = 24)]
    max_probes: u16,
    /// Maximum simulated days used to attribute delayed command consequences.
    #[arg(long, default_value_t = 360)]
    consequence_horizon: u16,
    /// Representative decisions retained per campaign.
    #[arg(long, default_value_t = 40)]
    trace_limit: u16,
    /// Player personas to run; repeat to select several. Omit to run all.
    #[arg(long, value_enum)]
    persona: Vec<GameplayPersonaArg>,
    /// Starting backgrounds to run; repeat to select several. Omit to run all.
    #[arg(long, value_enum)]
    background: Vec<BackgroundArg>,
    /// Emit the versioned structured JSON report instead of human-readable text.
    #[arg(long)]
    json: bool,
    /// Write the report to a file instead of standard output.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Return failure after writing the report when any critical finding exists.
    #[arg(long)]
    fail_on_critical: bool,
    /// Return failure after writing the report when its overall score is lower.
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..=100))]
    minimum_overall: Option<u16>,
}

fn run_playtest(registry: &Registry, args: PlaytestArgs) -> Result<(), CliError> {
    let personas = if args.persona.is_empty() {
        GameplayPersona::all().to_vec()
    } else {
        args.persona.into_iter().map(Into::into).collect()
    };
    let backgrounds = if args.background.is_empty() {
        vec![
            StartingBackground::Baker,
            StartingBackground::ClothTrader,
            StartingBackground::Blacksmith,
        ]
    } else {
        args.background.into_iter().map(Into::into).collect()
    };
    let config = GameplayHarnessConfig {
        start_seed: args.start_seed,
        seed_count: args.seeds,
        days_per_campaign: args.days,
        decision_interval_days: args.decision_interval,
        max_candidate_probes: args.max_probes,
        max_consequence_horizon_days: args.consequence_horizon,
        trace_limit_per_campaign: args.trace_limit,
        personas,
        backgrounds,
    };
    let started = Instant::now();
    let report = run_gameplay_harness(registry, config)?;
    let rendered = if args.json {
        serde_json::to_string_pretty(&report)
            .map_err(|source| CliError::GameplayReportSerialization { source })?
    } else {
        render_gameplay_report(&report)
    };
    if let Some(path) = args.output {
        ensure_output_parent(&path)?;
        std::fs::write(&path, rendered).map_err(|source| CliError::GameplayReportWrite {
            path: path.clone(),
            source,
        })?;
        println!("Wrote {}", path.display());
    } else {
        println!("{rendered}");
    }
    let elapsed = started.elapsed();
    let elapsed_micros = elapsed.as_micros().max(1);
    let days_per_second =
        u128::from(report.aggregate.simulated_days).saturating_mul(1_000_000) / elapsed_micros;
    eprintln!(
        "playtest completed in {:.3}s ({days_per_second} simulated days/s)",
        elapsed.as_secs_f64(),
    );
    if let Some(minimum) = args.minimum_overall
        && report.aggregate.scores.overall < minimum
    {
        return Err(CliError::GameplayQualityGate {
            reason: format!(
                "overall score {} is below required minimum {minimum}",
                report.aggregate.scores.overall
            ),
        });
    }
    if args.fail_on_critical {
        let critical = report
            .findings
            .iter()
            .filter(|finding| finding.severity == GameplayFindingSeverity::Critical)
            .count();
        if critical > 0 {
            return Err(CliError::GameplayQualityGate {
                reason: format!("report contains {critical} critical findings"),
            });
        }
    }
    Ok(())
}

fn ensure_output_parent(path: &Path) -> Result<(), CliError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent != Path::new(".") {
        std::fs::create_dir_all(parent).map_err(|source| CliError::OutputDirectoryCreate {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn print_human_summary(registry: &Registry, state: &civic_dynasty::AppState) {
    let summary = build_state_summary(registry, state);
    println!(
        "{} | year {}, day {} | elapsed {} days",
        summary.scenario_name, summary.year, summary.day_of_year, summary.elapsed_days
    );
    println!(
        "House {} | {:?} | treasury {} | business cash {}",
        summary.dynasty_name, summary.phase, summary.dynasty_treasury, summary.business_cash
    );
    println!(
        "Businesses: {} total, {} active | household groups: {} | food satisfaction: {:.1}%",
        summary.businesses,
        summary.active_businesses,
        summary.population_groups,
        f64::from(summary.average_food_satisfaction_basis_points) / 100.0
    );
    println!(
        "Strategic: {} contracts | {} private loans | {} civic debts ({}) | {} properties | {} active crises | {} unread notices",
        summary.active_contracts,
        summary.current_loans,
        summary.outstanding_civic_debts,
        summary.civic_debt_balance,
        summary.properties,
        summary.active_crises,
        summary.unread_notifications
    );

    println!("Market:");
    for quote in state.market().quotes() {
        let good = registry
            .get_good(quote.good_id())
            .expect("market quote good must exist");
        println!(
            "  {:<10} {:>10} | stock {:>10} | {:?}",
            good.name(),
            quote.price(),
            quote.stock(),
            quote.causes()
        );
    }

    println!("Recent chronicle:");
    for entry in state.chronicle().iter().rev().take(8).rev() {
        println!("  day {:>5}: {}", entry.day(), entry.summary());
    }
    println!("Recent notices:");
    for message in state.outbox().iter().rev().take(6).rev() {
        println!(
            "  day {:>5} [{:?}] {}: {}",
            message.day(),
            message.kind(),
            message.subject(),
            message.body()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_parent_creation_supports_nested_adapter_outputs() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let output = directory
            .path()
            .join("reports")
            .join("nested")
            .join("report.json");

        ensure_output_parent(&output).expect("nested output parent must be created");

        assert!(
            output.parent().expect("output must have a parent").is_dir(),
            "all adapter file outputs must support missing nested parent directories"
        );
    }
}
