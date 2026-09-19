use nx::{AppPaths, CheckReport, CheckRequest, Config, check};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(tag = "command")]
enum Request {
    #[serde(rename = "check")]
    Check(CheckRequest),
}

#[derive(Debug, Serialize)]
struct Response {
    ok: bool,
    report: Option<CheckReport>,
    error: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
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
    eprintln!("nxd listening on {}", socket.display());

    for stream in listener.incoming() {
        handle(stream?)?;
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
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Check(request)) => match check(&request) {
            Ok(report) => Response {
                ok: true,
                report: Some(report),
                error: None,
            },
            Err(error) => Response {
                ok: false,
                report: None,
                error: Some(error.to_string()),
            },
        },
        Err(error) => Response {
            ok: false,
            report: None,
            error: Some(error.to_string()),
        },
    };
    let mut stream = stream;
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    Ok(())
}
