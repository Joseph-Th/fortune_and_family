//! Command-line adapter for creating, advancing, inspecting, and validating campaigns.

use civic_dynasty::core::StartingBackground;
use civic_dynasty::{
    CommandError, NewGameConfig, PersistenceError, PlayerCommand, Registry, SimulationError,
    advance_days, apply_player_command, build_campaign_projection, build_new_game,
    build_rivergate_registry, load_state, render_campaign_html, save_state, validate_invariants,
};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
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
    /// Print the complete read-only campaign projection as JSON.
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
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BackgroundArg {
    Baker,
    ClothTrader,
    Blacksmith,
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
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Simulation(#[from] SimulationError),
    #[error("failed to serialize summary: {source}")]
    SummarySerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to render campaign dashboard: {source}")]
    DashboardSerialization {
        #[source]
        source: serde_json::Error,
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
            let mut state = build_new_game(&registry, config);
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
                let summary = serde_json::to_string_pretty(&state.summary(&registry))
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
                .map_err(|source| CliError::SummarySerialization { source })?;
            println!("{json}");
        }
        Command::Dashboard { input, output } => {
            let state = load_state(input)?;
            validate_invariants(&registry, &state);
            let html = render_campaign_html(&registry, &state)
                .map_err(|source| CliError::DashboardSerialization { source })?;
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
    }
    Ok(())
}

fn print_human_summary(registry: &Registry, state: &civic_dynasty::AppState) {
    let summary = state.summary(registry);
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
        "Strategic: {} contracts | {} loans | {} properties | {} active crises | {} unread notices",
        summary.active_contracts,
        summary.current_loans,
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
