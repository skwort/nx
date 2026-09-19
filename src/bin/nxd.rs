use nx::{AppPaths, Config, DaemonRequest, DaemonResponse, LogMode, check, init_logging};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use tracing::{error, info, warn};

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
    let socket = socket_override
        .or(config.daemon.socket)
        .unwrap_or(paths.socket);

    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    info!(socket = %socket.display(), "daemon listening");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle(stream) {
                    error!(%error, "request failed");
                }
            }
            Err(error) => error!(%error, "failed to accept connection"),
        }
    }
    Ok(())
}

fn parse_socket() -> nx::Result<Option<PathBuf>> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                return Ok(Some(PathBuf::from(
                    args.next().ok_or("--socket needs a path")?,
                )));
            }
            "-h" | "--help" => {
                println!("Usage: nxd [--socket PATH]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    Ok(None)
}

fn handle(stream: UnixStream) -> nx::Result<()> {
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
            match check(&request) {
                Ok(report) => DaemonResponse::success(report),
                Err(error) => {
                    warn!(%error, "check failed");
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
