//! Command-line adapter for creating, advancing, inspecting, and validating campaigns.

use civic_dynasty::core::StartingBackground;
use civic_dynasty::{
    ArtReviewConfig, ArtReviewError, ArtSeverity, CharacterRole, CommandError,
    GameplayFindingSeverity, GameplayHarnessConfig, GameplayHarnessError, GameplayPersona,
    NewGameConfig, NewGameError, PersistenceError, PlayerCommand, Registry, SimulationError,
    advance_days, apply_player_command, build_art_review, build_art_review_report,
    build_campaign_projection, build_new_game, build_rivergate_registry, build_state_summary,
    load_state, load_state_with_revision, render_art_review_html, render_campaign_html,
    render_gameplay_report, run_gameplay_harness, save_state, save_state_cas, save_state_new,
    validate_invariants,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::Builder;
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
        #[arg(long, default_value_t = false)]
        overwrite: bool,
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
    /// Render procedural sprites and write a self-contained visual review sheet.
    Art(ArtArgs),
}

#[derive(Debug, Args)]
struct ArtArgs {
    /// Write the review sheet here.
    #[arg(long, default_value = "sprite-review.html")]
    output: PathBuf,
    /// First character seed.
    #[arg(long, default_value_t = 1)]
    start_seed: u64,
    /// Characters generated per role.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    seeds: u32,
    /// Sprite height in pixels.
    #[arg(long, default_value_t = 48, value_parser = clap::value_parser!(i32).range(16..=256))]
    height: i32,
    /// Magnification used by the review views.
    #[arg(long, default_value_t = 6, value_parser = clap::value_parser!(u32).range(1..=16))]
    scale: u32,
    /// Roles to render; repeat to select several. Omit to render all.
    #[arg(long, value_enum)]
    role: Vec<CharacterRoleArg>,
    /// Emit the versioned structured JSON report instead of the HTML sheet.
    #[arg(long)]
    json: bool,
    /// Return failure after writing the output when any critical finding exists.
    #[arg(long)]
    fail_on_critical: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CharacterRoleArg {
    Baker,
    Merchant,
    Laborer,
    Official,
}

impl From<CharacterRoleArg> for CharacterRole {
    fn from(value: CharacterRoleArg) -> Self {
        match value {
            CharacterRoleArg::Baker => Self::Baker,
            CharacterRoleArg::Merchant => Self::Merchant,
            CharacterRoleArg::Laborer => Self::Laborer,
            CharacterRoleArg::Official => Self::Official,
        }
    }
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
    #[error(transparent)]
    ArtReview(#[from] ArtReviewError),
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
    #[error("failed to serialize art review report: {source}")]
    ArtReportSerialization {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write art review {path}: {source}")]
    ArtReviewWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("dashboard input {input} and output {output} identify the same filesystem path")]
    DashboardPathAliasing { input: PathBuf, output: PathBuf },
    #[error("art review gate failed: {reason}")]
    ArtQualityGate { reason: String },
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
    let started = Instant::now();
    let result = run_cli(cli, &registry);
    eprintln!(
        "civic-dynasty finished in {:.2}s",
        started.elapsed().as_secs_f64()
    );
    result
}

#[expect(
    clippy::too_many_lines,
    reason = "the dispatch keeps the full decision path in one auditable function"
)]
fn run_cli(cli: Cli, registry: &Registry) -> Result<(), CliError> {
    match cli.command {
        Command::New {
            output,
            seed,
            dynasty,
            founder,
            background,
            advance,
            overwrite,
        } => {
            let config = NewGameConfig {
                seed,
                dynasty_name: dynasty,
                founder_name: founder,
                background: background.into(),
            };
            let mut state = build_new_game(registry, config)?;
            if advance > 0 {
                advance_days(registry, &mut state, advance)?;
            }
            save_state_new(&output, &state, overwrite)?;
            print_human_summary(registry, &state);
            println!("Saved {}", output.display());
        }
        Command::Simulate {
            input,
            output,
            days,
        } => {
            let in_place = output.is_none() || output.as_ref() == Some(&input);
            let (mut state, revision) = load_state_with_revision(&input)?;
            validate_invariants(registry, &state);
            advance_days(registry, &mut state, days)?;
            let output_path = output.unwrap_or(input);
            if in_place {
                save_state_cas(&output_path, &state, &revision)?;
            } else {
                save_state(&output_path, &state)?;
            }
            print_human_summary(registry, &state);
            println!("Saved {}", output_path.display());
        }
        Command::Summary { input, json } => {
            let state = load_state(input)?;
            validate_invariants(registry, &state);
            if json {
                let summary = serde_json::to_string_pretty(&build_state_summary(registry, &state))
                    .map_err(|source| CliError::SummarySerialization { source })?;
                println!("{summary}");
            } else {
                print_human_summary(registry, &state);
            }
        }
        Command::Inspect { input } => {
            let state = load_state(input)?;
            validate_invariants(registry, &state);
            let projection = build_campaign_projection(registry, &state);
            let json = serde_json::to_string_pretty(&projection)
                .map_err(|source| CliError::ProjectionSerialization { source })?;
            println!("{json}");
        }
        Command::Dashboard { input, output } => {
            write_dashboard(registry, &input, &output)?;
            println!("Wrote {}", output.display());
        }
        Command::Execute {
            input,
            command,
            output,
        } => {
            let in_place = output.is_none() || output.as_ref() == Some(&input);
            let (mut state, revision) = load_state_with_revision(&input)?;
            validate_invariants(registry, &state);
            let command: PlayerCommand = serde_json::from_str(&command)
                .map_err(|source| CliError::CommandParse { source })?;
            let outcome = apply_player_command(registry, &mut state, command)?;
            validate_invariants(registry, &state);
            let output_path = output.unwrap_or(input);
            if in_place {
                save_state_cas(&output_path, &state, &revision)?;
            } else {
                save_state(&output_path, &state)?;
            }
            println!("{}", outcome.summary);
            print_human_summary(registry, &state);
            println!("Saved {}", output_path.display());
        }
        Command::Validate { input } => {
            let state = load_state(&input)?;
            validate_invariants(registry, &state);
            println!(
                "Validated {} at simulation day {} with schema version {}",
                input.display(),
                state.clock().day(),
                state.schema_version()
            );
        }
        Command::Playtest(args) => run_playtest(registry, args)?,
        Command::Art(args) => run_art(args)?,
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
    #[arg(long, default_value_t = 16)]
    max_probes: u16,
    /// Maximum simulated days used to attribute delayed command consequences.
    #[arg(long, default_value_t = 360)]
    consequence_horizon: u16,
    /// Representative decisions retained per campaign.
    #[arg(long, default_value_t = 40)]
    trace_limit: u16,
    /// Campaigns whose retained decision trace is rendered in the human-readable report. 0 disables the decision log.
    #[arg(long, default_value_t = 3)]
    decision_log: u8,
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
        decision_log_campaigns: args.decision_log,
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
        let outcome = write_generated_output(&path, rendered.as_bytes()).map_err(|source| {
            CliError::GameplayReportWrite {
                path: path.clone(),
                source,
            }
        })?;
        if outcome == GeneratedOutputOutcome::CommittedWithDegradedDurability {
            eprintln!(
                "warning: directory durability synchronization degraded for {}",
                path.display()
            );
        }
        println!("Wrote {}", path.display());
    } else {
        println!("{rendered}");
    }
    let elapsed = started.elapsed();
    let elapsed_micros = elapsed.as_micros().max(1);
    let days_per_second =
        u128::from(report.aggregate.simulated_days).saturating_mul(1_000_000) / elapsed_micros;
    eprintln!(
        "playtest {:.3}s ({} campaigns, {} simulated days, {} actions, score {}/100, {} findings, {days_per_second} simulated days/s)",
        elapsed.as_secs_f64(),
        report.aggregate.campaigns,
        report.aggregate.simulated_days,
        report.aggregate.successful_actions,
        report.aggregate.scores.overall,
        report.findings.len(),
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

fn run_art(args: ArtArgs) -> Result<(), CliError> {
    let roles = if args.role.is_empty() {
        CharacterRole::ALL.to_vec()
    } else {
        args.role.into_iter().map(Into::into).collect()
    };
    let review = build_art_review(ArtReviewConfig {
        roles,
        start_seed: args.start_seed,
        seeds: args.seeds,
        height: args.height,
        scale: args.scale,
    })?;
    let rendered = if args.json {
        serde_json::to_string_pretty(&build_art_review_report(&review))
            .map_err(|source| CliError::ArtReportSerialization { source })?
    } else {
        render_art_review_html(&review)
    };
    ensure_output_parent(&args.output)?;
    let outcome = write_generated_output(&args.output, rendered.as_bytes()).map_err(|source| {
        CliError::ArtReviewWrite {
            path: args.output.clone(),
            source,
        }
    })?;
    if outcome == GeneratedOutputOutcome::CommittedWithDegradedDurability {
        eprintln!(
            "warning: directory durability synchronization degraded for {}",
            args.output.display()
        );
    }
    println!(
        "Wrote {} ({} subjects, {} critical, {} warning or worse)",
        args.output.display(),
        review.subject_count(),
        review.count_at_least(ArtSeverity::Critical),
        review.count_at_least(ArtSeverity::Warning)
    );
    if args.fail_on_critical {
        let critical = review.count_at_least(ArtSeverity::Critical);
        if critical > 0 {
            return Err(CliError::ArtQualityGate {
                reason: format!("review contains {critical} critical findings"),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratedOutputOutcome {
    Committed,
    CommittedWithDegradedDurability,
}

#[cfg(test)]
std::thread_local! {
    static INJECT_GENERATED_OUTPUT_SYNC_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_inject_generated_output_sync_failure_for_test(inject: bool) {
    INJECT_GENERATED_OUTPUT_SYNC_FAILURE.with(|cell| cell.set(inject));
}

#[allow(clippy::unnecessary_wraps)]
fn sync_generated_output_directory_with_injection(
    #[allow(unused_variables)] parent: &Path,
) -> io::Result<()> {
    #[cfg(test)]
    if INJECT_GENERATED_OUTPUT_SYNC_FAILURE.with(std::cell::Cell::get) {
        return Err(io::Error::other(
            "injected generated output directory sync failure",
        ));
    }
    #[cfg(unix)]
    sync_generated_output_directory(parent)?;
    Ok(())
}

fn check_dashboard_path_aliasing(input: &Path, output: &Path) -> Result<(), CliError> {
    if input == output {
        return Err(CliError::DashboardPathAliasing {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
        });
    }
    let input_canonical = input.canonicalize().unwrap_or_else(|_| input.to_path_buf());
    if let Ok(output_canonical) = output.canonicalize() {
        if input_canonical == output_canonical {
            return Err(CliError::DashboardPathAliasing {
                input: input.to_path_buf(),
                output: output.to_path_buf(),
            });
        }
    } else if let Some(parent) = output.parent() {
        let parent_canonical = parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf());
        if let Some(file_name) = output.file_name() {
            let synthetic_output = parent_canonical.join(file_name);
            if input_canonical == synthetic_output {
                return Err(CliError::DashboardPathAliasing {
                    input: input.to_path_buf(),
                    output: output.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn write_dashboard(registry: &Registry, input: &Path, output: &Path) -> Result<(), CliError> {
    check_dashboard_path_aliasing(input, output)?;
    let state = load_state(input)?;
    validate_invariants(registry, &state);
    let html = render_campaign_html(registry, &state)
        .map_err(|source| CliError::DashboardSerialization { source })?;
    ensure_output_parent(output)?;
    let outcome = write_generated_output(output, html.as_bytes()).map_err(|source| {
        CliError::DashboardWrite {
            path: output.to_path_buf(),
            source,
        }
    })?;
    if outcome == GeneratedOutputOutcome::CommittedWithDegradedDurability {
        eprintln!(
            "warning: directory durability synchronization degraded for {}",
            output.display()
        );
    }
    Ok(())
}

fn write_generated_output(path: &Path, contents: &[u8]) -> io::Result<GeneratedOutputOutcome> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "generated output path is not a regular file: {}",
                    path.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let prefix = path.file_name().and_then(|name| name.to_str()).map_or_else(
        || ".generated-output-".to_owned(),
        |name| format!(".{name}."),
    );
    let mut temporary = Builder::new().prefix(&prefix).tempfile_in(parent)?;
    temporary.write_all(contents)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    let outcome = match sync_generated_output_directory_with_injection(parent) {
        Ok(()) => GeneratedOutputOutcome::Committed,
        Err(_) => GeneratedOutputOutcome::CommittedWithDegradedDurability,
    };
    Ok(outcome)
}

#[cfg(unix)]
fn sync_generated_output_directory(parent: &Path) -> io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
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
    let projection = build_campaign_projection(registry, state);
    println!(
        "{} | year {}, day {} | elapsed {} days",
        summary.scenario_name, summary.year, summary.day_of_year, summary.elapsed_days
    );
    println!(
        "House {} | {} | treasury {} | business cash {}",
        summary.dynasty_name,
        summary.phase.label(),
        summary.dynasty_treasury,
        summary.business_cash
    );
    println!(
        "Capacity {} / {} | {} businesses | {} properties | {:.1}% family unity",
        projection.player.effective_administrative_load,
        projection.player.administrative_capacity,
        projection.player.businesses,
        projection.player.properties,
        f64::from(projection.family.unity_basis_points) / 100.0
    );
    print_cli_attention(&projection);
    print_cli_market(&projection);
    print_cli_intelligence(&projection);
    println!("Recent chronicle:");
    for entry in state.chronicle().iter().rev().take(6).rev() {
        println!("  day {:>5}: {}", entry.day(), entry.summary());
    }
    print_cli_notices(&projection);
}

fn print_cli_attention(projection: &civic_dynasty::CampaignProjection) {
    use civic_dynasty::core::{BusinessStatus, EmploymentStatus, LegalCaseStatus, LoanStatus};

    let player_id = projection.player.id;
    let disputed_labor = projection.employment.iter().filter(|agreement| {
        agreement.owner_dynasty_id == player_id && agreement.status == EmploymentStatus::Disputed
    });
    let distressed = projection.businesses.iter().filter(|business| {
        business.owner_dynasty_id == player_id
            && matches!(
                business.status,
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    });
    let adverse_loans = projection.loans.iter().filter(|loan| {
        loan.borrower_dynasty_id == player_id
            && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
    });
    let defendant_cases = projection.legal_cases.iter().filter(|case| {
        case.defendant_dynasty_id == player_id
            && matches!(
                case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            )
    });

    println!("Needs attention:");
    let mut found = false;
    if projection.player.unmet_office_duties > 0 {
        println!(
            "  ! {} unmet office duties",
            projection.player.unmet_office_duties
        );
        found = true;
    }
    for agreement in disputed_labor {
        println!(
            "  ! labor dispute #{} at {}",
            agreement.id, agreement.business
        );
        found = true;
    }
    for business in distressed {
        println!(
            "  ! business #{} {} is {} with {} cash",
            business.id,
            business.name,
            cli_business_status_label(business.status),
            business.cash
        );
        found = true;
    }
    for loan in adverse_loans {
        println!(
            "  ! loan #{} is {}: {} outstanding, {} missed payments",
            loan.id,
            cli_loan_status_label(loan.status),
            loan.balance,
            loan.missed_payments
        );
        found = true;
    }
    for case in defendant_cases {
        let settlement = case
            .settlement_amount
            .map_or_else(String::new, |amount| format!("; settlement {amount}"));
        println!(
            "  ! legal case #{} by {}: hearing day {}, damages {}{}",
            case.id, case.plaintiff, case.hearing_day, case.damages, settlement
        );
        found = true;
    }
    for crisis in projection
        .crises
        .iter()
        .filter(|crisis| crisis.status.is_active())
    {
        println!(
            "  ! crisis #{} {}: {:.1}% severity",
            crisis.id,
            cli_crisis_kind_label(crisis.kind),
            f64::from(crisis.severity_basis_points) / 100.0
        );
        found = true;
    }
    if !found {
        println!("  none flagged by current campaign state");
    }
}

const fn cli_business_status_label(status: civic_dynasty::core::BusinessStatus) -> &'static str {
    use civic_dynasty::core::BusinessStatus;
    match status {
        BusinessStatus::Active => "active",
        BusinessStatus::Distressed => "distressed",
        BusinessStatus::Insolvent => "insolvent",
        BusinessStatus::Closed => "closed",
    }
}

const fn cli_loan_status_label(status: civic_dynasty::core::LoanStatus) -> &'static str {
    use civic_dynasty::core::LoanStatus;
    match status {
        LoanStatus::Current => "current",
        LoanStatus::Delinquent => "delinquent",
        LoanStatus::Defaulted => "defaulted",
        LoanStatus::Repaid => "repaid",
        LoanStatus::Restructured => "restructured",
    }
}

const fn cli_crisis_kind_label(kind: civic_dynasty::core::CrisisKind) -> &'static str {
    use civic_dynasty::core::CrisisKind;
    match kind {
        CrisisKind::GrainShortage => "grain shortage",
        CrisisKind::BankingPanic => "banking panic",
        CrisisKind::UrbanFire => "urban fire",
        CrisisKind::GuildRevolt => "guild revolt",
        CrisisKind::NobleDemand => "noble demand",
        CrisisKind::Epidemic => "epidemic",
        CrisisKind::TradeDisruption => "trade disruption",
    }
}

fn print_cli_market(projection: &civic_dynasty::CampaignProjection) {
    println!("Market watch:");
    let mut notable = projection
        .market
        .iter()
        .filter(|quote| {
            quote.price != quote.previous_price
                || quote.stock != quote.target_stock
                || quote.demand_today != quote.supply_today
        })
        .take(8)
        .peekable();
    if notable.peek().is_none() {
        println!("  no material market movement");
        return;
    }
    for quote in notable {
        let movement = match quote.price.cmp(&quote.previous_price) {
            std::cmp::Ordering::Greater => "up",
            std::cmp::Ordering::Less => "down",
            std::cmp::Ordering::Equal => "flat",
        };
        println!(
            "  {:<10} {:>10} ({movement:<4}) | stock {} / target {} | demand {} / supply {}",
            quote.good,
            quote.price,
            quote.stock,
            quote.target_stock,
            quote.demand_today,
            quote.supply_today
        );
    }
}

const fn cli_information_confidence_label(
    confidence: civic_dynasty::core::InformationConfidence,
) -> &'static str {
    use civic_dynasty::core::InformationConfidence;
    match confidence {
        InformationConfidence::Rumored => "rumored",
        InformationConfidence::Probable => "probable",
        InformationConfidence::Confirmed => "confirmed",
    }
}

fn print_cli_intelligence(projection: &civic_dynasty::CampaignProjection) {
    println!("Current intelligence:");
    let mut reports = projection.information.iter().rev().take(4).peekable();
    if reports.peek().is_none() {
        println!("  none");
        return;
    }
    for report in reports {
        println!(
            "  #{} [{}] {}: {} (expires day {})",
            report.id,
            cli_information_confidence_label(report.confidence),
            report.subject,
            report.summary,
            report.expires_day
        );
    }
}

fn print_cli_notices(projection: &civic_dynasty::CampaignProjection) {
    println!("Unread notices:");
    let mut unread = projection
        .notifications
        .iter()
        .rev()
        .filter(|message| !message.acknowledged)
        .take(6)
        .peekable();
    if unread.peek().is_none() {
        println!("  none");
        return;
    }
    for message in unread {
        println!(
            "  #{} day {:>5}: {}: {}",
            message.id, message.day, message.subject, message.body
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

    #[test]
    fn generated_output_replaces_existing_file_without_work_artifacts() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let output = directory.path().join("report.json");
        std::fs::write(&output, b"old report").expect("existing report fixture must be written");

        write_generated_output(&output, b"new report").expect("generated output must publish");

        assert_eq!(std::fs::read(&output).unwrap(), b"new report");
        let entries = std::fs::read_dir(directory.path())
            .expect("temporary directory must be readable")
            .map(|entry| entry.expect("directory entry must be readable").file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, ["report.json"]);
    }

    #[test]
    fn generated_output_rejects_directory_destination_without_mutating_it() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let output = directory.path().join("report.json");
        std::fs::create_dir(&output).expect("directory destination fixture must be created");
        std::fs::write(output.join("sentinel"), b"preserve")
            .expect("directory sentinel fixture must be written");

        let error = write_generated_output(&output, b"new report")
            .expect_err("directory destination must be rejected");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(output.join("sentinel")).unwrap(), b"preserve");
        let entries = std::fs::read_dir(directory.path())
            .expect("temporary directory must be readable")
            .map(|entry| entry.expect("directory entry must be readable").file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, ["report.json"]);
    }

    #[test]
    fn generated_output_reports_degraded_durability_when_directory_sync_fails() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let output = directory.path().join("report.json");

        set_inject_generated_output_sync_failure_for_test(true);
        let outcome = write_generated_output(&output, b"degraded report");
        set_inject_generated_output_sync_failure_for_test(false);

        assert_eq!(
            outcome.expect("write must commit even if directory sync degrades"),
            GeneratedOutputOutcome::CommittedWithDegradedDurability
        );
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"degraded report",
            "file must be visible and readable on disk despite degraded sync"
        );
    }

    #[test]
    fn dashboard_path_aliasing_detects_same_and_canonicalized_paths() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let campaign_path = directory.path().join("campaign.json");
        std::fs::write(&campaign_path, b"{}").expect("fixture must write");

        let direct_err = check_dashboard_path_aliasing(&campaign_path, &campaign_path);
        assert!(matches!(
            direct_err,
            Err(CliError::DashboardPathAliasing { .. })
        ));

        let dot_slash = directory.path().join(".").join("campaign.json");
        let canonical_err = check_dashboard_path_aliasing(&campaign_path, &dot_slash);
        assert!(matches!(
            canonical_err,
            Err(CliError::DashboardPathAliasing { .. })
        ));

        let distinct_output = directory.path().join("dashboard.html");
        assert!(check_dashboard_path_aliasing(&campaign_path, &distinct_output).is_ok());
    }
}
