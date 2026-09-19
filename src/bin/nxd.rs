use nx::{
    AppPaths, CheckRequest, Config, DaemonRequest, DaemonResponse, LogMode, ReportTarget, check,
    init_logging, load_latest_report,
};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

const STARTUP_DELAY: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Default)]
struct CheckWorker {
    gate: Arc<Mutex<()>>,
}

impl CheckWorker {
    fn run(&self, request: &CheckRequest) -> nx::Result<nx::CheckReport> {
        debug!("waiting for check worker");
        let _guard = self
            .gate
            .lock()
            .map_err(|_| "check worker lock is poisoned")?;
        check(request)
    }
}

fn main() {
    if let Err(error) = init_logging(LogMode::Daemon) {
        eprintln!("error: failed to initialize logging: {error}");
        std::process::exit(1);
    }
    if let Err(error) = run() {
        error!(%error, "daemon failed");
        std::process::exit(1);
    }
}

fn run() -> nx::Result<()> {
    let socket_override = parse_socket()?;
    let paths = AppPaths::discover()?;
    let config = Config::load(&paths.config_file)?;
    let check_interval = config.daemon.interval()?;
    let socket = socket_override
        .or(config.daemon.socket.clone())
        .unwrap_or(paths.socket);

    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    info!(socket = %socket.display(), "daemon listening");

    let worker = CheckWorker::default();
    if let Some(interval) = check_interval {
        let request = CheckRequest {
            flake: config
                .system
                .flake
                .ok_or("system.flake is required when daemon.check_interval is configured")?,
            host: config.system.configuration.ok_or(
                "system.configuration is required when daemon.check_interval is configured",
            )?,
            offline: false,
        };
        start_scheduler(worker.clone(), request, interval);
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let worker = worker.clone();
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &worker) {
                        error!(%error, "request failed");
                    }
                });
            }
            Err(error) => error!(%error, "failed to accept connection"),
        }
    }
    Ok(())
}

fn parse_socket() -> nx::Result<Option<PathBuf>> {
    let mut args = env::args().skip(1);
    let mut socket = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                socket = Some(PathBuf::from(args.next().ok_or("--socket needs a path")?));
            }
            "-h" | "--help" => {
                println!("Usage: nxd [--socket PATH]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    Ok(socket)
}

fn handle(stream: UnixStream, worker: &CheckWorker) -> nx::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let response = match serde_json::from_str::<DaemonRequest>(&line) {
        Ok(DaemonRequest::Check(request)) => {
            info!(
                flake = %request.flake.display(),
                configuration = %request.host,
                "received check request"
            );
            match worker.run(&request) {
                Ok(report) => DaemonResponse::success(report),
                Err(error) => {
                    warn!(%error, "check failed");
                    DaemonResponse::failure(error.to_string())
                }
            }
        }
        Ok(DaemonRequest::List(target)) => {
            info!(
                flake = %target.flake.display(),
                configuration = %target.configuration,
                "received list request"
            );
            match load_latest_report(&target) {
                Ok(Some(report)) => DaemonResponse::success(report),
                Ok(None) => DaemonResponse::failure(format!(
                    "no update report found for {}#{}; run `nx update check` first",
                    target.flake.display(),
                    target.configuration
                )),
                Err(error) => {
                    warn!(%error, "failed to load report");
                    DaemonResponse::failure(error.to_string())
                }
            }
        }
        Err(error) => {
            warn!(%error, "invalid request");
            DaemonResponse::failure(error.to_string())
        }
    };
    let mut stream = stream;
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn start_scheduler(worker: CheckWorker, request: CheckRequest, interval: Duration) {
    thread::spawn(move || {
        let target = ReportTarget {
            flake: request.flake.clone(),
            configuration: request.host.clone(),
        };
        info!(
            interval_seconds = interval.as_secs(),
            startup_delay_seconds = STARTUP_DELAY.as_secs(),
            flake = %target.flake.display(),
            configuration = %target.configuration,
            "update scheduler started"
        );
        info!(
            seconds = STARTUP_DELAY.as_secs(),
            "first update check scheduled"
        );
        thread::sleep(STARTUP_DELAY);

        loop {
            info!("starting scheduled update check");
            match worker.run(&request) {
                Ok(_) => info!("scheduled update check completed"),
                Err(error) => warn!(%error, "scheduled update check failed"),
            }
            info!(seconds = interval.as_secs(), "next update check scheduled");
            thread::sleep(interval);
        }
    });
}
