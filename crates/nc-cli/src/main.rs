//! `nca` - read-only NC preflight checker.
//!
//! Commands:
//!
//! ```text
//! nca check <file> [--config <toml>] [--format json|markdown|text]
//!                  [--output <path>] [--fail-on warning|error]
//! nca view <file>
//! nca simulate <file> [--config <toml>] [--to-line N] [--out-heightmap <png>]
//! nca rules
//! ```
//!
//! Exit codes are `0` (no finding at or above `--fail-on`), `1` (at least one
//! such finding), and `2` (the tool could not produce a report). This crate
//! must never depend on GPUI or `nc-app`; `nca view` spawns the sibling
//! `nca-gui` binary instead.

#![forbid(unsafe_code)]

mod png_export;
mod render;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use nc_analysis::config_file::{ConfigError, LoadedConfig, load_config};
use nc_analysis::{
    AnalysisRequest, Basis, Diagnostic, EvidenceValue, REPORT_SCHEMA, RULESET_VERSION, Report,
    RuleId, RuleRegistry, Severity, StockRequirement, analyze, normalize_diagnostics, rules,
};
use nc_core::ToolTable;
use nc_gcode::{ParserConfig, Program, SkippedBlock, parse_program_lossy};
use nc_sim::{SimConfig, SimError, Simulation};
use nc_stock::{StockError, StockModel};

use render::Summary;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nca: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, CliError> {
    let Some(command) = args.first() else {
        eprintln!("{}", help_text().trim_end());
        return Ok(ExitCode::from(2));
    };

    if matches!(command.as_str(), "-h" | "--help" | "help") {
        print!("{}", help_text());
        return Ok(ExitCode::SUCCESS);
    }
    if matches!(command.as_str(), "-V" | "--version" | "version") {
        println!(
            "nca {} ({REPORT_SCHEMA}, rules {RULESET_VERSION})",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(ExitCode::SUCCESS);
    }
    // `nca check --help` and friends are a help request, not an unknown flag.
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        print!("{}", help_text());
        return Ok(ExitCode::SUCCESS);
    }

    match command.as_str() {
        "check" => command_check(&args[1..]),
        "view" => command_view(&args[1..]),
        "simulate" => command_simulate(&args[1..]),
        "rules" => command_rules(&args[1..]),
        other => Err(CliError::Usage(format!(
            "unknown command {other:?}; expected check, view, simulate or rules"
        ))),
    }
}

fn command_check(args: &[String]) -> Result<ExitCode, CliError> {
    let args = CheckArgs::parse(args)?;
    let loaded = effective_config(args.config.as_deref())?;
    let parser_config = ParserConfig::default().with_dialect(loaded.dialect);
    let source = read_source(&args.file, parser_config.max_file_bytes)?;
    let lossy = parse_program_lossy(&source, &parser_config);

    let mut config = loaded.analysis;
    let resolved = ToolTable::resolve(&config.tools, &lossy.program.tool_hints);
    config.tools = resolved;

    let mut report = analyze_program(&args.file, &source, &lossy.program, &config, loaded.dialect);
    add_parse_findings(&mut report, &lossy.skipped_blocks);
    let summary = Summary {
        report: &report,
        record_count: lossy.program.motions.len(),
        skipped_blocks: lossy.skipped_blocks.len(),
        dialect_assumed: loaded.dialect_assumed,
    };
    let rendered = match args.format {
        OutputFormat::Json => {
            let mut text = report.to_json_pretty();
            text.push('\n');
            text
        }
        OutputFormat::Markdown => render::markdown(&summary),
        OutputFormat::Text => render::text(&summary),
    };
    emit(&rendered, args.output.as_deref())?;

    Ok(if report.fails_at(args.fail_on) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn command_view(args: &[String]) -> Result<ExitCode, CliError> {
    let file = single_file_argument("view", args)?;
    let gui = sibling_gui()?;
    if !gui.is_file() {
        return Err(CliError::ViewMissing { path: gui });
    }
    Command::new(&gui)
        .arg(&file)
        .spawn()
        .map_err(|source| CliError::ViewSpawn { path: gui, source })?;
    Ok(ExitCode::SUCCESS)
}

fn command_simulate(args: &[String]) -> Result<ExitCode, CliError> {
    let args = SimulateArgs::parse(args)?;
    let loaded = effective_config(args.config.as_deref())?;
    let parser_config = ParserConfig::default().with_dialect(loaded.dialect);
    let source = read_source(&args.file, parser_config.max_file_bytes)?;
    let lossy = parse_program_lossy(&source, &parser_config);
    let program = &lossy.program;

    let mut config = loaded.analysis;
    let resolved = ToolTable::resolve(&config.tools, &program.tool_hints);
    config.tools = resolved;

    let stock = match &config.stock {
        Some(stock) => stock.to_stock()?,
        None => StockModel::infer(program),
    };
    let mut sim_config = SimConfig::default();
    if let Some(machine) = &config.machine {
        sim_config.rapid_rate_mm_min = machine.rapid_rate_mm_min();
    }

    if let Some(line) = args.to_line
        && (line == 0 || line > program.lines.len())
    {
        return Err(CliError::Usage(format!(
            "--to-line must be between 1 and {} (the program has {} lines)",
            program.lines.len(),
            program.lines.len()
        )));
    }

    let mut simulation = Simulation::new(program, stock, &config.tools, &sim_config)?;
    if let Some(line) = args.to_line {
        if simulation
            .timeline()
            .time_for_line_at_or_after(line)
            .is_some()
        {
            simulation.seek_to_line(line);
        } else {
            // A line after the last indexed motion (for example the final
            // `M30`) means "simulate through the end", not "stay at tick 0".
            let end_s = simulation.timeline().total_time_s();
            simulation.seek_to_time(end_s);
        }
    }

    if let Some(path) = args.out_heightmap.as_deref() {
        png_export::write_heightmap_png(simulation.heightmap(), path).map_err(|message| {
            CliError::Heightmap {
                path: path.to_path_buf(),
                message,
            }
        })?;
    }

    let snapshot = simulation.snapshot();
    let stats = simulation.stats();
    let heightmap = simulation.heightmap();
    let stock = simulation.stock();
    let assumed = stats.volume_assumed;
    let coarse = simulation.is_coarse();

    let mut out = String::new();
    out.push_str(&format!("simulate: {}\n", file_name(&args.file)));
    out.push_str(&format!(
        "stock: {} ({:.3} x {:.3} x {:.3} mm, top Z {:.3} mm)\n",
        match stock.source {
            nc_stock::StockSource::Inferred => "inferred",
            nc_stock::StockSource::Explicit => "explicit",
        },
        stock.max_mm[0] - stock.min_mm[0],
        stock.max_mm[1] - stock.min_mm[1],
        stock.max_mm[2] - stock.min_mm[2],
        stock.top_z_mm
    ));
    out.push_str(&format!("records: {}\n", program.motions.len()));
    out.push_str(&format!("tick: {}\n", snapshot.tick));
    out.push_str(&format!("time: {:.3} s\n", snapshot.time_s));
    out.push_str(&format!("tool: {}\n", snapshot.tool));
    out.push_str(&format!("volume: {:.3} mm3\n", stats.cut_volume_mm3));
    out.push_str(&format!("volume assumed: {assumed}\n"));
    out.push_str(&format!("assumed: {assumed}\n"));
    out.push_str(&format!("max depth: {:.3} mm\n", stats.max_depth_mm));
    out.push_str(&format!("coarse: {coarse}\n"));
    out.push_str(&format!(
        "heightmap: {} x {} cells at {:.3} mm\n",
        heightmap.cols(),
        heightmap.rows(),
        heightmap.cell_mm()
    ));
    if let Some(path) = args.out_heightmap {
        out.push_str(&format!("heightmap png: {}\n", path.display()));
    }
    write_stdout(&out)?;
    Ok(ExitCode::SUCCESS)
}

fn command_rules(args: &[String]) -> Result<ExitCode, CliError> {
    ensure_no_arguments("rules", args)?;
    let registry = RuleRegistry::from_rules(rules::all());

    let mut rows: Vec<RuleRow> = registry
        .iter()
        .map(|rule| {
            let requirements = rule.requirements();
            RuleRow {
                id: rule.id().to_string(),
                severity: rule.default_severity().label().to_string(),
                basis: rule.basis().label().to_string(),
                stock: match requirements.stock {
                    StockRequirement::None => "-".to_string(),
                    StockRequirement::InferredOk => "inferred ok".to_string(),
                    StockRequirement::Explicit => "explicit".to_string(),
                },
                machine: yes_no(requirements.machine),
                material: yes_no(requirements.material),
                tool_fields: if requirements.tool_fields.is_empty() {
                    "-".to_string()
                } else {
                    requirements
                        .tool_fields
                        .iter()
                        .map(|field| field.key())
                        .collect::<Vec<_>>()
                        .join(",")
                },
            }
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));

    let id_width = rows
        .iter()
        .map(|row| row.id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = String::new();
    out.push_str(&format!("rules: {}\n\n", rows.len()));
    out.push_str(&format!(
        "{:<id_width$}  {:<8}  {:<5}  {:<11}  {:<7}  {:<8}  {}\n",
        "rule", "severity", "basis", "stock", "machine", "material", "tool fields"
    ));
    for row in &rows {
        out.push_str(&format!(
            "{:<id_width$}  {:<8}  {:<5}  {:<11}  {:<7}  {:<8}  {}\n",
            row.id, row.severity, row.basis, row.stock, row.machine, row.material, row.tool_fields
        ));
    }
    out.push_str(
        "\nnote: lossy parser skips are reported as `parse.unsupported` findings\n\
         outside the rule registry; they are not included in the coverage counts.\n",
    );
    write_stdout(&out)?;
    Ok(ExitCode::SUCCESS)
}

fn analyze_program(
    path: &Path,
    source: &str,
    program: &Program,
    config: &nc_analysis::AnalysisConfig,
    dialect: nc_core::Dialect,
) -> nc_analysis::Report {
    let name = file_name(path);
    let request = AnalysisRequest::new(&name, source, config, dialect);
    let registry = RuleRegistry::from_rules(rules::all());
    analyze(program, &request, &registry)
}

/// Surfaces lossy parser skips as the reserved `parse.unsupported` rule.
///
/// The rule is not in the current registry yet, so it does not change the
/// coverage counts; each skipped source block becomes one error diagnostic
/// with the raw line in evidence. This keeps a file the parser could not fully
/// understand out of the "clean" exit code.
fn add_parse_findings(report: &mut Report, blocks: &[SkippedBlock]) {
    for block in blocks {
        let mut evidence = BTreeMap::new();
        evidence.insert("raw".to_string(), EvidenceValue::Text(block.source.clone()));
        evidence.insert(
            "reason".to_string(),
            EvidenceValue::Text(block.reason.clone()),
        );
        report.diagnostics.push(Diagnostic {
            rule: RuleId::new("parse.unsupported"),
            severity: Severity::Error,
            basis: Basis::Exact,
            line: block.line,
            column: 0,
            line_end: block.line,
            message: format!(
                "unsupported or unparseable block skipped ({})",
                block.reason
            ),
            evidence,
        });
    }
    normalize_diagnostics(&mut report.diagnostics);
}

fn effective_config(path: Option<&Path>) -> Result<LoadedConfig, CliError> {
    match path {
        Some(path) => Ok(load_config(path)?),
        None => Ok(LoadedConfig::default()),
    }
}

/// Reads at most `limit` bytes, refusing a larger input before parsing.
fn read_source(path: &Path, limit: usize) -> Result<String, CliError> {
    let file = File::open(path).map_err(|source| CliError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| CliError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > limit {
        return Err(CliError::InputTooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|_| CliError::NotUtf8 {
        path: path.to_path_buf(),
    })
}

fn emit(text: &str, output: Option<&Path>) -> Result<(), CliError> {
    match output {
        Some(path) => std::fs::write(path, text).map_err(|source| CliError::Write {
            path: path.to_path_buf(),
            source,
        }),
        None => write_stdout(text),
    }
}

fn write_stdout(text: &str) -> Result<(), CliError> {
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(CliError::Stdout)
}

fn sibling_gui() -> Result<PathBuf, CliError> {
    let executable = std::env::current_exe().map_err(CliError::CurrentExe)?;
    let directory = executable
        .parent()
        .ok_or_else(|| CliError::CurrentExe(std::io::Error::other("executable has no parent")))?;
    let name = if cfg!(windows) {
        "nca-gui.exe"
    } else {
        "nca-gui"
    };
    Ok(directory.join(name))
}

fn single_file_argument(command: &str, args: &[String]) -> Result<PathBuf, CliError> {
    let mut file: Option<PathBuf> = None;
    for arg in args {
        if arg.starts_with('-') {
            return Err(CliError::Usage(format!(
                "unknown option {arg:?} for {command}"
            )));
        }
        if file.replace(PathBuf::from(arg)).is_some() {
            return Err(CliError::Usage(format!("{command} takes exactly one file")));
        }
    }
    file.ok_or_else(|| CliError::Usage(format!("{command} needs a file")))
}

fn ensure_no_arguments(command: &str, args: &[String]) -> Result<(), CliError> {
    if let Some(arg) = args.first() {
        return Err(CliError::Usage(format!(
            "{command} takes no arguments, got {arg:?}"
        )));
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("source.nc")
        .to_string()
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "-".to_string()
    }
}

struct RuleRow {
    id: String,
    severity: String,
    basis: String,
    stock: String,
    machine: String,
    material: String,
    tool_fields: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Markdown,
    Text,
}

impl OutputFormat {
    fn parse(text: &str) -> Result<Self, CliError> {
        match text {
            "json" => Ok(Self::Json),
            "markdown" | "md" => Ok(Self::Markdown),
            "text" | "txt" => Ok(Self::Text),
            other => Err(CliError::Usage(format!(
                "unknown --format {other:?}; expected json, markdown or text"
            ))),
        }
    }
}

struct CheckArgs {
    file: PathBuf,
    config: Option<PathBuf>,
    format: OutputFormat,
    output: Option<PathBuf>,
    fail_on: Severity,
}

impl CheckArgs {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut parsed = Self {
            file: PathBuf::new(),
            config: None,
            format: OutputFormat::Json,
            output: None,
            fail_on: Severity::Error,
        };
        let mut file: Option<PathBuf> = None;
        let mut index = 0usize;
        while index < args.len() {
            match args[index].as_str() {
                "--config" => {
                    parsed.config = Some(PathBuf::from(next_value(args, &mut index, "--config")?));
                }
                "--format" => {
                    parsed.format =
                        OutputFormat::parse(&next_value(args, &mut index, "--format")?)?;
                }
                "--output" => {
                    parsed.output = Some(PathBuf::from(next_value(args, &mut index, "--output")?));
                }
                "--fail-on" => {
                    parsed.fail_on = parse_fail_on(&next_value(args, &mut index, "--fail-on")?)?;
                }
                argument if argument.starts_with('-') => {
                    return Err(CliError::Usage(format!(
                        "unknown option {argument:?} for check"
                    )));
                }
                argument => {
                    if file.replace(PathBuf::from(argument)).is_some() {
                        return Err(CliError::Usage(
                            "check takes exactly one G-code file".to_string(),
                        ));
                    }
                }
            }
            index += 1;
        }
        parsed.file =
            file.ok_or_else(|| CliError::Usage("check needs a G-code file".to_string()))?;
        Ok(parsed)
    }
}

struct SimulateArgs {
    file: PathBuf,
    config: Option<PathBuf>,
    to_line: Option<usize>,
    out_heightmap: Option<PathBuf>,
}

impl SimulateArgs {
    fn parse(args: &[String]) -> Result<Self, CliError> {
        let mut parsed = Self {
            file: PathBuf::new(),
            config: None,
            to_line: None,
            out_heightmap: None,
        };
        let mut file: Option<PathBuf> = None;
        let mut index = 0usize;
        while index < args.len() {
            match args[index].as_str() {
                "--config" => {
                    parsed.config = Some(PathBuf::from(next_value(args, &mut index, "--config")?));
                }
                "--to-line" => {
                    let value = next_value(args, &mut index, "--to-line")?;
                    parsed.to_line = Some(value.parse::<usize>().map_err(|_| {
                        CliError::Usage(format!(
                            "--to-line must be a positive integer, got {value:?}"
                        ))
                    })?);
                }
                "--out-heightmap" => {
                    parsed.out_heightmap = Some(PathBuf::from(next_value(
                        args,
                        &mut index,
                        "--out-heightmap",
                    )?));
                }
                argument if argument.starts_with('-') => {
                    return Err(CliError::Usage(format!(
                        "unknown option {argument:?} for simulate"
                    )));
                }
                argument => {
                    if file.replace(PathBuf::from(argument)).is_some() {
                        return Err(CliError::Usage(
                            "simulate takes exactly one G-code file".to_string(),
                        ));
                    }
                }
            }
            index += 1;
        }
        parsed.file =
            file.ok_or_else(|| CliError::Usage("simulate needs a G-code file".to_string()))?;
        Ok(parsed)
    }
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> Result<String, CliError> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| CliError::Usage(format!("{flag} needs a value")))
}

fn parse_fail_on(text: &str) -> Result<Severity, CliError> {
    match text {
        "warning" => Ok(Severity::Warning),
        "error" => Ok(Severity::Error),
        other => Err(CliError::Usage(format!(
            "unknown --fail-on {other:?}; expected warning or error"
        ))),
    }
}

fn help_text() -> String {
    format!(
        "\
nca {version} - read-only NC preflight checker

usage:
  nca check <file> [--config <toml>] [--format json|markdown|text]
                   [--output <path>] [--fail-on warning|error]
  nca view <file>
  nca simulate <file> [--config <toml>] [--to-line N] [--out-heightmap <png>]
  nca rules
  nca --help
  nca --version

check defaults: --format json --fail-on error

exit codes:
  0  no finding at or above --fail-on
  1  at least one finding at or above --fail-on
  2  no report could be produced (unreadable input, invalid config, limit,
     simulation failure, or internal error)

The report is advisory. nca never connects to a controller and never edits
G-code. The stable machine interface is the versioned JSON report
({schema}); human formats carry the same counts and every diagnostic.
",
        version = env!("CARGO_PKG_VERSION"),
        schema = REPORT_SCHEMA,
    )
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read {path}: input exceeds the {limit} byte limit")]
    InputTooLarge { path: PathBuf, limit: usize },
    #[error("cannot read {path}: input is not valid UTF-8")]
    NotUtf8 { path: PathBuf },
    #[error(transparent)]
    Config(Box<ConfigError>),
    #[error(transparent)]
    Stock(#[from] StockError),
    #[error(transparent)]
    Sim(#[from] SimError),
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write heightmap PNG {path}: {message}")]
    Heightmap { path: PathBuf, message: String },
    #[error("cannot determine the running executable: {0}")]
    CurrentExe(#[source] std::io::Error),
    #[error("cannot find the GUI next to this executable: {path} (build nca-gui first)")]
    ViewMissing { path: PathBuf },
    #[error("cannot start {path}: {source}")]
    ViewSpawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write to standard output: {0}")]
    Stdout(#[source] std::io::Error),
}

impl From<ConfigError> for CliError {
    fn from(error: ConfigError) -> Self {
        Self::Config(Box::new(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::ToolField;

    #[test]
    fn format_aliases_are_accepted() {
        assert!(matches!(
            OutputFormat::parse("json"),
            Ok(OutputFormat::Json)
        ));
        assert!(matches!(
            OutputFormat::parse("md"),
            Ok(OutputFormat::Markdown)
        ));
        assert!(matches!(
            OutputFormat::parse("text"),
            Ok(OutputFormat::Text)
        ));
        assert!(OutputFormat::parse("xml").is_err());
    }

    #[test]
    fn fail_on_only_accepts_warning_or_error() {
        assert!(matches!(parse_fail_on("warning"), Ok(Severity::Warning)));
        assert!(matches!(parse_fail_on("error"), Ok(Severity::Error)));
        assert!(parse_fail_on("info").is_err());
    }

    #[test]
    fn check_rejects_a_second_file() {
        let args = vec!["one.nc".to_string(), "two.nc".to_string()];
        assert!(CheckArgs::parse(&args).is_err());
    }

    #[test]
    fn check_defaults() {
        let args = vec!["part.nc".to_string()];
        let parsed = CheckArgs::parse(&args).expect("args");
        assert_eq!(parsed.format, OutputFormat::Json);
        assert_eq!(parsed.fail_on, Severity::Error);
        assert!(parsed.config.is_none());
    }

    #[test]
    fn tool_field_key_is_used_in_rule_table() {
        assert_eq!(ToolField::Diameter.key(), "diameter");
    }
}
