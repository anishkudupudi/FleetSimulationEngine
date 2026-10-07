use fleet_sim::Engine;
use std::env;
use std::io::{self, BufRead};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--dashboard") {
        let port = match dashboard_port(&args) {
            Ok(port) => port,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(2);
            }
        };

        if let Err(err) = fleet_sim::dashboard::run(port) {
            eprintln!("failed to run dashboard: {err}");
            std::process::exit(1);
        }
        return;
    }

    let stdin = io::stdin();
    let mut engine = Engine::new();

    for line in stdin.lock().lines() {
        match line {
            Ok(line) => {
                if let Some(response) = engine.handle_line(&line) {
                    println!(
                        "{}",
                        serde_json::to_string(&response)
                            .expect("serializing a response should not fail")
                    );
                }
            }
            Err(err) => {
                eprintln!("failed to read stdin: {err}");
                std::process::exit(1);
            }
        }
    }
}

fn dashboard_port(args: &[String]) -> Result<u16, String> {
    let mut port = 7878;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--dashboard" => {
                index += 1;
            }
            "--port" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--port requires a value".to_string())?;
                port = value
                    .parse::<u16>()
                    .map_err(|_| format!("invalid port {value}"))?;
                index += 2;
            }
            arg => return Err(format!("unknown dashboard argument {arg}")),
        }
    }

    Ok(port)
}
