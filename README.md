# Fleet Simulation Engine

Fleet Simulation Engine is a discrete-time Rust simulator for managing vessels, docks, cargo, and historical world state. It is driven by newline-delimited JSON commands: the program reads one command per line from `stdin` and writes one JSON response per command to `stdout`.

The simulator is protocol-first, so it can be run from the terminal, tested with JSON fixtures, or inspected through the optional local dashboard.

See `DESIGN.md` for the design document.

## What It Supports

The base simulator tracks vessel movement across a numbered set of locations:

- `init` creates the world with travel times, dock throughput, starting vessels, and optional starting cargo.
- `tick` advances the simulation by one discrete time step.
- `set_destination` plans travel for an idle vessel.
- `get_state` returns the current planned world state.

Follow-On 1 adds dock and cargo operations:

- Each location has a dock with cargo inventory.
- Each vessel has cargo inventory.
- Cargo is grouped by destination and quantity.
- `dock` parks a vessel at its current location.
- `undock` returns a docked vessel to idle state.
- `load` moves cargo from a dock to a vessel.
- `unload` moves cargo from a vessel to a dock, or delivers it if the cargo has reached its destination.
- `swap` loads one cargo group while unloading another in a single dock operation.
- `dock_throughput` limits how many cargo operations each dock can perform between ticks.

Follow-On 2 adds historical state queries:

- The engine stores committed snapshots at tick boundaries.
- `get_state_at` returns the committed state for a previous tick.
- Historical queries do not mutate the current simulation state.

The project also includes an optional local dashboard:

- Run it with `cargo run -- --dashboard`.
- It serves a browser UI from the Rust binary.
- It sends the same JSON commands to the same simulation engine used by the CLI.

## Requirements

Install Rust and Cargo with `rustup`:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, restart your shell or follow the `rustup` prompt to update your `PATH`.

## Run The CLI

Run the base example command stream:

```sh
cargo run --quiet < examples/base.input.jsonl
```

Run the cargo/dock follow-on example:

```sh
cargo run --quiet < examples/follow_on_1.input.jsonl
```

Run the historical-query follow-on example:

```sh
cargo run --quiet < examples/follow_on_2.input.jsonl
```

Run the edge-case example:

```sh
cargo run --quiet < examples/edge_cases.input.jsonl
```

You can also type commands manually:

```sh
cargo run --quiet
```

Then enter one JSON command per line. Blank lines are ignored.

## Run The Dashboard

Start the optional local dashboard:

```sh
cargo run -- --dashboard
```

Open the URL printed to `stderr`, usually:

```text
http://127.0.0.1:7878
```

Use a different port if needed:

```sh
cargo run -- --dashboard --port 9000
```

Dashboard mode is local-only and does not change the normal stdin/stdout CLI behavior.

## Test

Run the full test suite:

```sh
cargo test
```

The tests cover:

- simulator library behavior and validation rules
- cargo loading, unloading, delivery, swap, and throughput behavior
- historical state queries
- CLI stdin/stdout fixture behavior
- embedded dashboard assets and dashboard command handling

## Command Format

Each input line is a JSON object with a `command` string and a `parameters` object:

```json
{"command":"get_state","parameters":{}}
```

Each response is either:

```json
{"status":"ok","payload":{}}
```

or:

```json
{"status":"error","payload":{"message":"reason"}}
```
