# Fleet Simulation Engine

Discrete-time vessel simulator for the interview project. The program reads newline-delimited JSON commands from `stdin` and writes one JSON response per command to `stdout`.

This implementation currently covers the base project and Follow-On 1 cargo/dock operations:

- `init`
- `tick`
- `set_destination`
- `get_state`
- `dock`
- `undock`
- `load`
- `unload`
- `swap`

Follow-On 2 historical queries are not implemented yet.

## Setup

Install Rust and Cargo with rustup:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, restart your shell or follow the rustup prompt to update your `PATH`.

## Run

Run the simulator with an example command stream:

```sh
cargo run --quiet < examples/base.input.jsonl
```

The output should match:

```sh
cat examples/base.expected.jsonl
```

Run the cargo/dock flow:

```sh
cargo run --quiet < examples/follow_on_1.input.jsonl
```

You can also type commands manually:

```sh
cargo run --quiet
```

## Test

```sh
cargo test
```

Tests cover the simulator library and the CLI stdin/stdout behavior.
