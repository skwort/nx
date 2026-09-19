use nx::{
    AppPaths, CheckReport, CheckRequest, Config, DaemonRequest, DaemonResponse, LogMode, check,
    init_logging,
};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use tracing::{debug, error, info};

struct CliOptions {
    request: CheckRequest,
    report_verbose: bool,
    log_verbosity: u8,
    direct: bool,
    socket: PathBuf,
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
    let report = if options.direct {
        info!("running check directly");
        check(&options.request)?
    } else {
        request_check(&options.socket, options.request)?
    };
    print_report(&report, options.report_verbose);
    Ok(())
}

fn parse_args() -> nx::Result<CliOptions> {
    let mut args = env::args().skip(1);
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
                    "Usage: nx update check [--flake PATH] [--configuration NAME] [--offline] [--verbose] [-v|-vv] [--direct] [--socket PATH]"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    let paths = AppPaths::discover()?;
    let config = Config::load(&paths.config_file)?;

    Ok(CliOptions {
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

fn request_check(socket: &PathBuf, request: CheckRequest) -> nx::Result<CheckReport> {
    info!(socket = %socket.display(), "requesting check from daemon");
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        format!(
            "cannot connect to daemon at {}: {error}; start nxd or use --direct",
            socket.display()
        )
    })?;
    serde_json::to_writer(&mut stream, &DaemonRequest::Check(request))?;
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
    let before = &report.before;
    let after = &report.after;
    println!("\nNixOS update report\n");
    print_scalar_change("Kernel", &before.kernel, &after.kernel);
    print_map_changes("Flake inputs", &before.inputs, &after.inputs, verbose);

    let mut names = before
        .packages
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    names.extend(after.packages.keys().cloned());
    let mut changes = String::new();
    for name in names {
        let old = before.packages.get(&name).cloned().unwrap_or_default();
        let new = after.packages.get(&name).cloned().unwrap_or_default();
        if old == new {
            continue;
        }
        let marker = if old.is_empty() {
            "+"
        } else if new.is_empty() {
            "-"
        } else {
            "~"
        };
        let detail = if old.is_empty() {
            display_set(&new)
        } else if new.is_empty() {
            display_set(&old)
        } else {
            format!("{} -> {}", display_set(&old), display_set(&new))
        };
        changes.push_str(&format!("  {marker} {name}: {detail}\n"));
    }
    if changes.is_empty() {
        println!("Declared packages: unchanged");
    } else {
        println!("Declared package changes:\n{changes}");
    }
    println!(
        "System evaluation: {}",
        if before.system_drv == after.system_drv {
            "unchanged"
        } else {
            "changed"
        }
    );
}

fn print_scalar_change(label: &str, before: &str, after: &str) {
    if before == after {
        println!("{label}: unchanged ({before})");
    } else {
        println!("{label}: {before} -> {after}");
    }
}

fn print_map_changes(
    label: &str,
    before: &std::collections::BTreeMap<String, String>,
    after: &std::collections::BTreeMap<String, String>,
    verbose: bool,
) {
    let mut names = before
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    names.extend(after.keys().cloned());
    let changes = names
        .into_iter()
        .filter_map(|name| {
            let old = before.get(&name).map(String::as_str);
            let new = after.get(&name).map(String::as_str);
            if old == new {
                return None;
            }
            let (marker, details) = match (old, new) {
                (None, Some(value)) => ("+", verbose.then(|| value.to_owned())),
                (Some(value), None) => ("-", verbose.then(|| value.to_owned())),
                (Some(old), Some(new)) => ("~", verbose.then(|| format!("{old} -> {new}"))),
                (None, None) => unreachable!(),
            };
            Some(match details {
                Some(details) => format!("  {marker} {name}: {details}"),
                None => format!("  {marker} {name}"),
            })
        })
        .collect::<Vec<_>>();
    if changes.is_empty() {
        println!("{label}: no changes");
    } else {
        println!("{label}:\n{}", changes.join("\n"));
    }
}

fn display_set(values: &std::collections::BTreeSet<String>) -> String {
    values.iter().cloned().collect::<Vec<_>>().join(", ")
}
