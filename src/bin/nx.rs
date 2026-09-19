use nx::{
    AppPaths, CheckReport, CheckRequest, Config, DaemonRequest, DaemonResponse, LogMode,
    ReportTarget, check, init_logging, load_latest_report,
};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

#[derive(Clone, Copy)]
enum CliCommand {
    Check,
    List,
}

struct CliOptions {
    command: CliCommand,
    request: CheckRequest,
    report_verbose: bool,
    log_verbosity: u8,
    direct: bool,
    socket: PathBuf,
}

#[derive(Default)]
struct ChangeCounts {
    changed: usize,
    added: usize,
    removed: usize,
}

struct Colours {
    enabled: bool,
}

impl Colours {
    fn paint(&self, code: &str, text: impl AsRef<str>) -> String {
        if self.enabled {
            format!("\x1b[{code}m{}\x1b[0m", text.as_ref())
        } else {
            text.as_ref().to_owned()
        }
    }

    fn heading(&self, text: &str) -> String {
        self.paint("1", text)
    }

    fn marker(&self, marker: &str) -> String {
        let code = match marker {
            "+" => "32",
            "-" => "31",
            "~" => "33",
            _ => "0",
        };
        self.paint(code, marker)
    }
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = init_logging(LogMode::Cli {
        verbosity: options.log_verbosity,
    }) {
        eprintln!("error: failed to initialize logging: {error}");
        std::process::exit(1);
    }
    if let Err(error) = run(options) {
        error!(%error, "command failed");
        std::process::exit(1);
    }
}

fn run(options: CliOptions) -> nx::Result<()> {
    let target = ReportTarget {
        flake: options.request.flake.clone(),
        configuration: options.request.host.clone(),
    };
    let report = if options.direct {
        match options.command {
            CliCommand::Check => {
                info!("running check directly");
                check(&options.request)?
            }
            CliCommand::List => load_latest_report(&target)?.ok_or_else(|| {
                format!(
                    "no update report found for {}#{}; run `nx update check` first",
                    target.flake.display(),
                    target.configuration
                )
            })?,
        }
    } else {
        let request = match options.command {
            CliCommand::Check => DaemonRequest::Check(options.request),
            CliCommand::List => DaemonRequest::List(target),
        };
        request_daemon(&options.socket, request)?
    };
    print_report(&report, options.report_verbose);
    Ok(())
}

fn parse_args() -> nx::Result<CliOptions> {
    let mut args = env::args().skip(1);
    let mut command = CliCommand::Check;
    let mut flake = None;
    let mut host = None;
    let mut offline = false;
    let mut report_verbose = false;
    let mut log_verbosity = 0_u8;
    let mut direct = false;
    let mut socket = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "update" | "check" => {}
            "list" => command = CliCommand::List,
            "--flake" => flake = Some(PathBuf::from(args.next().ok_or("--flake needs a path")?)),
            "--configuration" | "--host" => {
                host = Some(args.next().ok_or("--configuration needs a name")?)
            }
            "--offline" => offline = true,
            "--verbose" => {
                report_verbose = true;
                log_verbosity = log_verbosity.max(1);
            }
            "-v" => log_verbosity = log_verbosity.saturating_add(1),
            "-vv" => log_verbosity = log_verbosity.saturating_add(2),
            "-vvv" => log_verbosity = log_verbosity.saturating_add(3),
            "--direct" => direct = true,
            "--socket" => socket = Some(PathBuf::from(args.next().ok_or("--socket needs a path")?)),
            "-h" | "--help" => {
                println!(
                    "Usage:\n  nx update check [OPTIONS]\n  nx update list [OPTIONS]\n\nOptions:\n  --flake PATH\n  --configuration NAME\n  --offline\n  --verbose\n  -v, -vv\n  --direct\n  --socket PATH"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    let paths = AppPaths::discover()?;
    let config = Config::load(&paths.config_file)?;

    Ok(CliOptions {
        command,
        request: CheckRequest {
            flake: flake
                .or(config.system.flake)
                .ok_or("flake path is required: use --flake or set system.flake in config.toml")?,
            host: host.or(config.system.configuration).ok_or(
                "configuration name is required: use --configuration or set system.configuration in config.toml",
            )?,
            offline,
        },
        report_verbose,
        log_verbosity,
        direct,
        socket: socket.or(config.daemon.socket).unwrap_or(paths.socket),
    })
}

fn request_daemon(socket: &PathBuf, request: DaemonRequest) -> nx::Result<CheckReport> {
    info!(socket = %socket.display(), "requesting report from daemon");
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        format!(
            "cannot connect to daemon at {}: {error}; start nxd or use --direct",
            socket.display()
        )
    })?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    debug!(bytes = line.len(), "received daemon response");
    let response: DaemonResponse = serde_json::from_str(&line)?;
    match (response.ok, response.report, response.error) {
        (true, Some(report), _) => Ok(report),
        (false, _, Some(error)) => Err(error.into()),
        _ => Err("daemon returned an invalid response".into()),
    }
}

fn print_report(report: &CheckReport, verbose: bool) {
    let colours = Colours {
        enabled: std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none(),
    };
    let input_counts = map_change_counts(&report.before.inputs, &report.after.inputs);
    let package_counts = map_change_counts(&report.before.packages, &report.after.packages);

    println!(
        "{}\n",
        colours.paint("2", format!("Checked {}", display_age(report.checked_at)))
    );
    println!("{}", colours.heading("Summary"));
    if report.before.kernel == report.after.kernel {
        println!("  Kernel   {} (unchanged)", report.before.kernel);
    } else {
        println!(
            "  Kernel   {} {} {}",
            report.before.kernel,
            colours.paint("2", "→"),
            report.after.kernel
        );
    }
    println!("  Inputs   {}", display_counts(&input_counts));
    println!("  Packages {}", display_counts(&package_counts));
    println!(
        "  System   {}",
        if report.before.system_drv == report.after.system_drv {
            "unchanged"
        } else {
            "changed"
        }
    );

    print_input_changes(report, verbose, &colours);
    print_package_changes(report, &colours);
}

fn print_input_changes(report: &CheckReport, verbose: bool, colours: &Colours) {
    let changes = map_changes(&report.before.inputs, &report.after.inputs);
    if changes.is_empty() {
        return;
    }
    println!("\n{}", colours.heading("Flake inputs"));
    for (name, old, new) in changes {
        let marker = change_marker(old, new);
        if verbose {
            println!(
                "  {} {}{}",
                colours.marker(marker),
                name,
                change_detail(old, new, "  ")
            );
        } else {
            println!("  {} {}", colours.marker(marker), name);
        }
    }
}

fn print_package_changes(report: &CheckReport, colours: &Colours) {
    let changes = map_changes(&report.before.packages, &report.after.packages);
    if changes.is_empty() {
        return;
    }
    println!("\n{}", colours.heading("Declared packages"));
    for (name, old, new) in changes {
        let marker = change_marker(old, new);
        let detail = match (old, new) {
            (None, Some(values)) | (Some(values), None) => display_set(values),
            (Some(old), Some(new)) => {
                format!("{} → {}", display_set(old), display_set(new))
            }
            (None, None) => unreachable!(),
        };
        println!("  {} {name}  {detail}", colours.marker(marker));
    }
}

fn map_changes<'a, T: PartialEq>(
    before: &'a BTreeMap<String, T>,
    after: &'a BTreeMap<String, T>,
) -> Vec<(String, Option<&'a T>, Option<&'a T>)> {
    let mut names = before.keys().cloned().collect::<BTreeSet<_>>();
    names.extend(after.keys().cloned());
    names
        .into_iter()
        .filter_map(|name| {
            let old = before.get(&name);
            let new = after.get(&name);
            (old != new).then_some((name, old, new))
        })
        .collect()
}

fn map_change_counts<T: PartialEq>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for (_, old, new) in map_changes(before, after) {
        match (old, new) {
            (None, Some(_)) => counts.added += 1,
            (Some(_), None) => counts.removed += 1,
            (Some(_), Some(_)) => counts.changed += 1,
            (None, None) => unreachable!(),
        }
    }
    counts
}

fn display_counts(counts: &ChangeCounts) -> String {
    if counts.changed + counts.added + counts.removed == 0 {
        return "no changes".to_owned();
    }
    let mut parts = Vec::new();
    if counts.changed > 0 {
        parts.push(format!("{} changed", counts.changed));
    }
    if counts.added > 0 {
        parts.push(format!("{} added", counts.added));
    }
    if counts.removed > 0 {
        parts.push(format!("{} removed", counts.removed));
    }
    parts.join(", ")
}

fn change_marker<T>(old: Option<&T>, new: Option<&T>) -> &'static str {
    match (old, new) {
        (None, Some(_)) => "+",
        (Some(_), None) => "-",
        (Some(_), Some(_)) => "~",
        (None, None) => unreachable!(),
    }
}

fn change_detail(old: Option<&String>, new: Option<&String>, separator: &str) -> String {
    match (old, new) {
        (None, Some(value)) | (Some(value), None) => format!("{separator}{value}"),
        (Some(old), Some(new)) => format!("{separator}{old} → {new}"),
        (None, None) => unreachable!(),
    }
}

fn display_set(values: &BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join(", ")
}

fn display_age(checked_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(checked_at);
    let elapsed = now.saturating_sub(checked_at);
    match elapsed {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}
