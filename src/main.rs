use fleet_sim::Engine;
use std::io::{self, BufRead};

fn main() {
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
