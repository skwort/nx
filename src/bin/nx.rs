use nx::{check, CheckReport, CheckRequest};
use std::env;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> nx::Result<()> {
    let (request, verbose) = parse_args()?;
    let report = check(&request)?;
    print_report(&report, verbose);
    Ok(())
}

fn parse_args() -> nx::Result<(CheckRequest, bool)> {
    let mut args = env::args().skip(1);
    let mut flake = None;
    let mut host = None;
    let mut offline = false;
    let mut verbose = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "update" | "check" => {}
            "--flake" => flake = Some(PathBuf::from(args.next().ok_or("--flake needs a path")?)),
            "--host" => host = Some(args.next().ok_or("--host needs a configuration name")?),
            "--offline" => offline = true,
            "--verbose" => verbose = true,
            "-h" | "--help" => {
                println!("Usage: nx update check --flake PATH --host NAME [--offline] [--verbose]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    Ok((
        CheckRequest {
            flake: flake.ok_or("--flake is required")?,
            host: host.ok_or("--host is required")?,
            offline,
        },
        verbose,
    ))
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
