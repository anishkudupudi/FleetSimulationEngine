# Fleet Simulation Engine Design Document

## 1. Overview

The Fleet Simulation Engine is a command-driven Rust simulation for tracking the movement of vessels and cargo around dock locations. The user is able to send commands as JSON input (through stdin) to set vessels in transit, dock/undock at locations, and transfer cargo between vessels and docks. On success, the simulation outputs a response JSON line (stdout) with the updated world state. On failure, the JSON instead contains a descriptive error message.

This project also supports a UI dashboard for human-readable outputs. The dashboard allows users to send commands and visualize vessel/dock states without interacting with JSON formatted data, while still using the same simulation engine.

## 2. Implemented Features

### Initialization

The simulation is initialized through the `init` command. This defines the constant shape of the world: travel-time matrix, dock throughput values, starting vessels and their locations, and optional starting cargo. The travel-time matrix should always be a n x n matrix, determining that there are n possible locations. This value is used to determine if other parameters are also valid including vessel locations, cargo destinations, dock throughput entries, etc.

Main invariants:

- The engine has no world state before a successful `init`. Any previous command must fail.
- `init` can only succeed once.
- The travel-time matrix must be square.
- Diagonal travel times must be `0`.
- Non-diagonal travel times must be positive.
- Every location must have one positive dock throughput value.
- Vessel ids must be non-empty and unique.
- Vessel and cargo locations must be valid location ids.
- Cargo quantities must be positive.

### Discrete-time Tick

The simulator is run by discrete-time ticks, instead of a continous wall-clock time. When a tick is successful, it advances the simulation by one time step. The engine is responsible for resolving vessel arrivals and applying new cargo entered into the simulation at docks.

Main invariants:

- A successful `tick` advances time by exactly one step.
- Vessel arrivals are resolved during `tick`.
- Tick manifests add cargo to the newly committed tick state.
- Dock throughput counters reset after a successful `tick`.
- Failed commands do not advance time.

### Planned Versus Committed State

One key important design choice is the distinction between planned and committed states. Committed states are the most recently saved world state following a tick boundary. Whenever a new valid command (other than tick) is processed, the change is applied to the planned state. This represents the expected state that is intended to occur once the next tick boundary is called. This distinction allows the simulation to only store high-level changes between ticks. For example, if a vessel repeatedly undocks and docks at a location within the same tick, the simulation only stores the change between ticks rather than each command as historical fact. This also allows idle vessels to change their travel destination: set_desination can repeatedly be called before the next tick to any other location, or itself to remain in idle again. 

Main invariants:

- Committed state is the source of truth for tick-boundary history.
- Planned state represents the world state after successful intra-tick commands.
- Successful non-tick commands update planned state but not committed state.
- `tick` is the only command that promotes planned state to committed state.
- `get_state` returns planned state.
- `get_state_at` returns committed historical state.
- Historical snapshots are recorded from committed state, not planned state.
- Failed commands leave both committed and planned state unchanged.

### Vessel Movement

Vessel movement is handled by `set_destination`. This command is only valid if the vessel is idle in the committed state. After validating that the vessel exists and the destination is within range of locations, the vessel enters transit in the planned state. The arrival time is calculated using the travel_time matrix provided during initialization. Whenever tick runs, each transit vessel is checked to see if its arrival tick time has been reached. Once the vessel reaches its destination (arrival tick is the current commited tick), the vessel becomes idle once again.

Main invariants:

- Only idle committed vessels can be sent to a destination.
- Unknown vessel ids are rejected.
- Out-of-range destinations are rejected.
- Arrival tick is calculated from committed tick plus travel time.
- A vessel in transit becomes idle when the simulation reaches its arrival tick.
- Cargo remains attached to a vessel while it travels.
- Setting a destination to the vessel's current location keeps the vessel idle.

### Docking

Docking is represented as a vessel state, in addition to idle and transit. Calling `dock` changes an idle vessel's planned state to docked, whereas calling `undock` changes a docked vessel's planned state back to idle.


Docking and movement both operate through planned state. This means pending docking and movement decisions can overwrite each other before the next tick when committed state still allows the command. The purpose of this is to be able to revert a command within the same tick. Once a tick commits a vessel into transit, docking commands are rejected until that vessel arrives and becomes idle again.

Main invariants:

- A vessel in transit cannot dock or undock.
- Docking and undocking do not change cargo inventory.
- A vessel must be docked to run cargo operations.
- A pending undock prevents later cargo operations in that same tick from treating the vessel as still docked.

### Cargo And Dock Operations

Each location is represented as a dock with its own cargo inventory. Cargo is represented by destination, so any unit with the same destination can be considered equivalent. Cargo can enter the simulation at any point through optional manifest parameters on `init` and `tick` commands.

Vessels also have cargo inventory to transport units between docks. `load` moves cargo from a dock to a docked vessel at the same location. `unload` moves cargo from a docked vessel to a dock at the same location. If the destination cargo is reached, then it is removed from the simulation. Cargo can be dropped off at other docks. `swap`combines loading and unloading into a single command.

Main invariants:

- Cargo quantities are positive.
- Empty cargo entries are removed from inventories.
- Cargo with the same destination is aggregated.
- A vessel must be docked in committed state and still docked in planned state to transfer cargo.
- A dock must have enough matching cargo for `load`.
- A vessel must have enough matching cargo for `unload`.
- Delivered cargo leaves the simulation instead of being added back to a dock.
- Failed cargo commands leave state unchanged.

### Dock Throughput

Dock throughput is initialized during creation of the simulation. This limits how many cargo operations a dock can handle between each tick. Each dock has a different throughput limit based on its index in the dock throughput list. The counter for each dock's throughput limit is reset after every successful tick. `swap` is still considered one operation, even though it involved both an unload and load.

Main invariants:

- Each dock has a positive throughput limit.
- Throughput is counted per dock, not globally.
- Pending operations at a dock cannot exceed that dock's throughput.
- Throughput reservations are released if the cargo command fails.
- Throughput resets only after a successful `tick`.


## 3. Architecture

The code is organized by separating the simulation logic from the CLI parsing.

- `src/lib.rs` contains the core engine, data model, command parsing, validation, state transitions, command implementations, and most unit tests.
- `src/main.rs` either runs in normal CLI mode or with the dashboard. With the flag set, the project simply reads JSON inputs by stdin line by line. Each line is sent to the engine, which prints serialized responses back to stdout. In dashboard mode, it starts a local HTTP server to receive command JSONs from the UI instead of stdin.
- `src/dashboard.rs` hosts the local dashboard server. The service owns an engine instance and handles requests.
- `examples/` directory contains command streams and expected streams from the project specs.
- `tests/` directory contains integration tests for CLI and dashboard behavior


## 4. Input And Output Protocol

The simulator uses newline-delimited JSON as input. Each input is parsed as a command object with a `command` field and a `parameters` object. The command string determines which typed parameter structure the engine will parse, since each command requires a different set of parameters. This allows for a simple protocol while also checking for typed structs internally.

Each successful command returns a JSON response with a status: either `ok` or `error`. Valid commands contain the current world state and invalid commands return the error with a message to explain the failure:


```json
{"status":"ok","payload":{}}
```

```json
{"status":"error","payload":{"message":"reason"}}
```

## 5. Core State Model

The simulation engine stores both configuration and runtime state. The configuration includes the travel-time matrix and the dock throughput values created during `init`. Runtime state includes the planned state, committed state, and historical snapshots.

The following structs are used to represent runtime states:
- `WorldState`: contains the current tick, map of all vessels (`VesselState`), and map of all docks (`DockState`). This makes this struct a complete snapshot of the simulation at a specific tick, allowing for easy access for historical snapshorts with `get_state_at`.
- `VesselState`: contains the vessel's movement status (`VesselStatus`) and current `CargoInventory`. The status describes what the vessel is doing and the cargo inventory describes what the vessel is carrying.
- `VesselStatus`: represented as either Idle, Docked, or Transit. If it is idle or docked, it only contains the vessels current location. If the vessel is in transit, it stores the `arrives_at_tick` value and the two locations it is travelling to and from. This is stored as an enum to guarantee that a vessel is only in one of these states.
- `DockState`: contains `CargoInventory`. This represents the cargo that each dock holds.
- `CargoInventory`: contains a map of cargo units with (destination, quantity) pairs. This allows either a vessel or dock to maintain its inventory and keep track of how many units of each type of cargo it currently holds. When cargo quantity for any destination reaches 0, the entry is removed.

Main invariants:

- `WorldState` contains all vessels and all docks for the current snapshot.
- Each vessel has exactly one status at a time.
- A vessel's cargo is preserved when its status changes.
- Every location has a corresponding dock state.
- Cargo entries should not remain in inventory with zero quantity.
- Vessel cargo and dock cargo use the same inventory representation.
