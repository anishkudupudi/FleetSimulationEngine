use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

pub mod dashboard;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "payload", rename_all = "lowercase")]
pub enum Response {
    Ok(WorldState),
    Error(ErrorPayload),
}

impl Response {
    pub fn ok(payload: WorldState) -> Self {
        Self::Ok(payload)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::Error(ErrorPayload {
            message: message.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldState {
    pub tick: u64,
    pub vessels: HashMap<String, VesselState>,
    pub docks: HashMap<usize, DockState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockState {
    pub cargo: CargoInventory,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CargoInventory {
    by_destination: HashMap<usize, u64>,
}

impl CargoInventory {
    fn add(&mut self, destination: usize, quantity: u64) {
        if quantity == 0 {
            return;
        }

        *self.by_destination.entry(destination).or_insert(0) += quantity;
    }

    fn remove(&mut self, destination: usize, amount: u64) -> Result<(), String> {
        if amount == 0 {
            return Ok(());
        }

        let quantity = self
            .by_destination
            .get_mut(&destination)
            .ok_or_else(|| format!("cargo with destination {destination} is not available"))?;

        if *quantity < amount {
            return Err(format!(
                "cargo with destination {destination} is not available"
            ));
        }

        *quantity -= amount;
        if *quantity == 0 {
            self.by_destination.remove(&destination);
        }

        Ok(())
    }
}

impl Serialize for CargoInventory {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries: Vec<_> = self.by_destination.iter().collect();
        entries.sort_by_key(|(destination, _)| **destination);

        let mut seq = serializer.serialize_seq(Some(entries.len()))?;
        for (destination, quantity) in entries {
            seq.serialize_element(&CargoEntry {
                destination: *destination,
                quantity: *quantity,
            })?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for CargoInventory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<CargoEntry>::deserialize(deserializer)?;
        let mut inventory = Self::default();
        for entry in entries {
            inventory.add(entry.destination, entry.quantity);
        }
        Ok(inventory)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoEntry {
    pub destination: usize,
    pub quantity: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VesselState {
    #[serde(flatten)]
    pub status: VesselStatus,
    pub cargo: CargoInventory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VesselStatus {
    Idle {
        location: usize,
    },
    Docked {
        location: usize,
    },
    Transit {
        from: usize,
        to: usize,
        arrives_at_tick: u64,
    },
}

impl VesselState {
    fn cargo_mut(&mut self) -> &mut CargoInventory {
        &mut self.cargo
    }
}

#[derive(Debug, Default)]
pub struct Engine {
    travel_times: Option<Vec<Vec<u64>>>,
    dock_throughput: Option<Vec<u64>>,
    pending_dock_operations: Option<Vec<u64>>,
    history: Vec<WorldState>,
    // Committed state is the last tick-boundary snapshot; planned state includes
    // valid commands issued since that tick.
    committed: Option<WorldState>,
    planned: Option<WorldState>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_line(&mut self, line: &str) -> Option<Response> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        Some(match self.handle_command_line(trimmed) {
            Ok(state) => Response::ok(state),
            Err(message) => Response::error(message),
        })
    }

    pub fn handle_command(&mut self, command: Command) -> Result<WorldState, String> {
        match command {
            Command::Init(params) => self.init(params),
            Command::Tick(params) => self.tick(params),
            Command::SetDestination(params) => self.set_destination(params),
            Command::GetState(_) => self.get_state(),
            Command::GetStateAt(params) => self.get_state_at(params),
            Command::Dock(params) => self.dock(params),
            Command::Undock(params) => self.undock(params),
            Command::Load(params) => self.load(params),
            Command::Unload(params) => self.unload(params),
            Command::Swap(params) => self.swap(params),
        }
    }

    fn handle_command_line(&mut self, line: &str) -> Result<WorldState, String> {
        let raw: RawCommand =
            serde_json::from_str(line).map_err(|err| format!("invalid command: {err}"))?;
        let command = raw.into_command()?;
        self.handle_command(command)
    }

    fn init(&mut self, params: InitParams) -> Result<WorldState, String> {
        if self.committed.is_some() {
            return Err("engine is already initialized".to_string());
        }

        validate_travel_times(&params.travel_times)?;
        validate_dock_throughput(&params.dock_throughput, params.travel_times.len())?;

        if params.vessels.is_empty() {
            return Err("vessels must not be empty".to_string());
        }

        let location_count = params.travel_times.len();
        let mut vessels = HashMap::new();

        for vessel in params.vessels {
            if vessel.id.is_empty() {
                return Err("vessel id must not be empty".to_string());
            }

            if vessel.location >= location_count {
                return Err(format!(
                    "vessel {} location {} is out of range",
                    vessel.id, vessel.location
                ));
            }

            if vessels
                .insert(
                    vessel.id.clone(),
                    VesselState {
                        status: VesselStatus::Idle {
                            location: vessel.location,
                        },
                        cargo: CargoInventory::default(),
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate vessel id {}", vessel.id));
            }
        }

        let mut docks = empty_docks(location_count);
        apply_manifest(&mut docks, params.cargo_manifest.as_deref(), location_count)?;

        let state = WorldState {
            tick: 0,
            vessels,
            docks,
        };

        self.travel_times = Some(params.travel_times);
        self.dock_throughput = Some(params.dock_throughput);
        self.pending_dock_operations = Some(vec![0; location_count]);
        self.committed = Some(state.clone());
        self.planned = Some(state.clone());
        self.history = vec![state.clone()];
        Ok(state)
    }

    fn tick(&mut self, params: TickParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let location_count = self.location_count();
        if let Some(manifest) = params.cargo_manifest.as_deref() {
            validate_manifest(manifest, location_count)?;
        }

        let mut next = self
            .planned
            .clone()
            .expect("planned state exists after initialization");

        next.tick = next
            .tick
            .checked_add(1)
            .ok_or_else(|| "tick overflow".to_string())?;

        for vessel in next.vessels.values_mut() {
            let arrival = match &vessel.status {
                VesselStatus::Transit {
                    to,
                    arrives_at_tick,
                    ..
                } if *arrives_at_tick <= next.tick => Some(*to),
                _ => None,
            };

            if let Some(location) = arrival {
                vessel.status = VesselStatus::Idle { location };
            }
        }

        apply_manifest(
            &mut next.docks,
            params.cargo_manifest.as_deref(),
            location_count,
        )?;

        self.committed = Some(next.clone());
        self.planned = Some(next.clone());
        self.pending_dock_operations = Some(vec![0; location_count]);
        self.history.push(next.clone());
        Ok(next)
    }

    fn set_destination(&mut self, params: SetDestinationParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let travel_times = self
            .travel_times
            .as_ref()
            .expect("travel times exist after initialization");

        if params.destination >= travel_times.len() {
            return Err(format!(
                "destination {} is out of range",
                params.destination
            ));
        }

        let committed = self
            .committed
            .as_ref()
            .expect("committed state exists after initialization");

        let current_location = match committed.vessels.get(&params.vessel_id) {
            Some(VesselState {
                status: VesselStatus::Idle { location },
                ..
            }) => *location,
            Some(_) => return Err(format!("vessel {} is not idle", params.vessel_id)),
            None => return Err(format!("unknown vessel {}", params.vessel_id)),
        };

        let planned_status = if params.destination == current_location {
            VesselStatus::Idle {
                location: current_location,
            }
        } else {
            let travel_time = travel_times[current_location][params.destination];
            let arrives_at_tick = committed
                .tick
                .checked_add(travel_time)
                .ok_or_else(|| format!("travel time overflow for vessel {}", params.vessel_id))?;

            VesselStatus::Transit {
                from: current_location,
                to: params.destination,
                arrives_at_tick,
            }
        };

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .get_mut(&params.vessel_id)
            .expect("planned vessel exists after validation")
            .status = planned_status;
        Ok(planned.clone())
    }

    fn dock(&mut self, params: VesselParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let location = match self.committed_vessel(&params.vessel_id)? {
            VesselState {
                status: VesselStatus::Idle { location } | VesselStatus::Docked { location },
                ..
            } => *location,
            VesselState {
                status: VesselStatus::Transit { .. },
                ..
            } => {
                return Err(format!("vessel {} is not idle", params.vessel_id));
            }
        };

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .get_mut(&params.vessel_id)
            .expect("planned vessel exists after validation")
            .status = VesselStatus::Docked { location };
        Ok(planned.clone())
    }

    fn undock(&mut self, params: VesselParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let location = match self.committed_vessel(&params.vessel_id)? {
            VesselState {
                status: VesselStatus::Docked { location },
                ..
            } => *location,
            VesselState {
                status: VesselStatus::Idle { .. },
                ..
            } => match self
                .planned
                .as_ref()
                .expect("planned state exists after initialization")
                .vessels
                .get(&params.vessel_id)
            {
                Some(VesselState {
                    status: VesselStatus::Docked { location },
                    ..
                }) => *location,
                _ => return Err(format!("vessel {} is not docked", params.vessel_id)),
            },
            VesselState {
                status: VesselStatus::Transit { .. },
                ..
            } => {
                return Err(format!("vessel {} is not docked", params.vessel_id));
            }
        };

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .get_mut(&params.vessel_id)
            .expect("planned vessel exists after validation")
            .status = VesselStatus::Idle { location };
        Ok(planned.clone())
    }

    fn load(&mut self, params: TransferParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        validate_transfer_amount(params.amount)?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location, params.amount)?;

        let result = self.apply_load(
            &params.vessel_id,
            location,
            params.destination,
            params.amount,
        );
        if result.is_err() {
            self.release_dock_operation(location, params.amount);
        }

        result
    }

    fn unload(&mut self, params: TransferParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        validate_transfer_amount(params.amount)?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location, params.amount)?;

        let result = self.apply_unload(
            &params.vessel_id,
            location,
            params.destination,
            params.amount,
        );
        if result.is_err() {
            self.release_dock_operation(location, params.amount);
        }

        result
    }

    fn swap(&mut self, params: SwapParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        validate_transfer_amount(params.amount)?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location, params.amount)?;

        let before = self.planned.clone();
        let load_result = self.apply_load(
            &params.vessel_id,
            location,
            params.load_destination,
            params.amount,
        );
        let result = match load_result {
            Ok(_) => self.apply_unload(
                &params.vessel_id,
                location,
                params.unload_destination,
                params.amount,
            ),
            Err(err) => Err(err),
        };

        if result.is_err() {
            self.planned = before;
            self.release_dock_operation(location, params.amount);
        }

        result
    }

    fn get_state(&self) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        Ok(self
            .planned
            .clone()
            .expect("planned state exists after initialization"))
    }

    fn get_state_at(&self, params: GetStateAtParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let current_tick = self
            .committed
            .as_ref()
            .expect("committed state exists after initialization")
            .tick;

        if params.tick > current_tick {
            return Err(format!(
                "tick {} is in the future (current tick: {})",
                params.tick, current_tick
            ));
        }

        self.history
            .get(params.tick as usize)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "tick {} is in the future (current tick: {})",
                    params.tick, current_tick
                )
            })
    }

    fn ensure_initialized(&self) -> Result<(), String> {
        if self.committed.is_none() {
            Err("engine is not initialized".to_string())
        } else {
            Ok(())
        }
    }

    fn location_count(&self) -> usize {
        self.travel_times
            .as_ref()
            .expect("travel times exist after initialization")
            .len()
    }

    fn committed_vessel(&self, vessel_id: &str) -> Result<&VesselState, String> {
        self.committed
            .as_ref()
            .expect("committed state exists after initialization")
            .vessels
            .get(vessel_id)
            .ok_or_else(|| format!("unknown vessel {vessel_id}"))
    }

    fn validate_cargo_command(&self, vessel_id: &str) -> Result<usize, String> {
        let committed_location = match self.committed_vessel(vessel_id)? {
            VesselState {
                status: VesselStatus::Docked { location },
                ..
            } => *location,
            _ => return Err(format!("vessel {vessel_id} is not docked")),
        };

        match self
            .planned
            .as_ref()
            .expect("planned state exists after initialization")
            .vessels
            .get(vessel_id)
        {
            Some(VesselState {
                status: VesselStatus::Docked { location },
                ..
            }) if *location == committed_location => Ok(committed_location),
            Some(_) => Err(format!("vessel {vessel_id} is not docked")),
            None => Err(format!("unknown vessel {vessel_id}")),
        }
    }

    fn reserve_dock_operation(&mut self, location: usize, amount: u64) -> Result<(), String> {
        let limit = self
            .dock_throughput
            .as_ref()
            .expect("dock throughput exists after initialization")[location];
        let operations = self
            .pending_dock_operations
            .as_mut()
            .expect("dock operation counters exist after initialization");
        let next_count = operations[location]
            .checked_add(amount)
            .ok_or_else(|| format!("transfer operation count overflow at location {location}"))?;

        if next_count > limit {
            return Err(format!(
                "transfer of {next_count} operations exceeds dock throughput of {limit} at location {location}"
            ));
        }

        operations[location] = next_count;
        Ok(())
    }

    fn release_dock_operation(&mut self, location: usize, amount: u64) {
        let operations = self
            .pending_dock_operations
            .as_mut()
            .expect("dock operation counters exist after initialization");
        operations[location] -= amount;
    }

    fn apply_load(
        &mut self,
        vessel_id: &str,
        location: usize,
        destination: usize,
        amount: u64,
    ) -> Result<WorldState, String> {
        if destination >= self.location_count() {
            return Err(format!("destination {destination} is out of range"));
        }

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .docks
            .get_mut(&location)
            .expect("dock exists for valid location")
            .cargo
            .remove(destination, amount)?;
        planned
            .vessels
            .get_mut(vessel_id)
            .expect("planned vessel exists after validation")
            .cargo_mut()
            .add(destination, amount);

        Ok(planned.clone())
    }

    fn apply_unload(
        &mut self,
        vessel_id: &str,
        location: usize,
        destination: usize,
        amount: u64,
    ) -> Result<WorldState, String> {
        if destination >= self.location_count() {
            return Err(format!("destination {destination} is out of range"));
        }

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .get_mut(vessel_id)
            .expect("planned vessel exists after validation")
            .cargo_mut()
            .remove(destination, amount)?;

        if destination != location {
            planned
                .docks
                .get_mut(&location)
                .expect("dock exists for valid location")
                .cargo
                .add(destination, amount);
        }

        Ok(planned.clone())
    }
}

fn empty_docks(location_count: usize) -> HashMap<usize, DockState> {
    (0..location_count)
        .map(|location| {
            (
                location,
                DockState {
                    cargo: CargoInventory::default(),
                },
            )
        })
        .collect()
}

fn apply_manifest(
    docks: &mut HashMap<usize, DockState>,
    manifest: Option<&[CargoManifestEntry]>,
    location_count: usize,
) -> Result<(), String> {
    if let Some(manifest) = manifest {
        validate_manifest(manifest, location_count)?;
        for entry in manifest {
            docks
                .get_mut(&entry.location)
                .expect("dock exists for valid manifest location")
                .cargo
                .add(entry.destination, entry.quantity);
        }
    }

    Ok(())
}

fn validate_manifest(manifest: &[CargoManifestEntry], location_count: usize) -> Result<(), String> {
    for entry in manifest {
        if entry.location >= location_count {
            return Err(format!("cargo location {} is out of range", entry.location));
        }
        if entry.destination >= location_count {
            return Err(format!(
                "cargo destination {} is out of range",
                entry.destination
            ));
        }
        if entry.quantity == 0 {
            return Err("cargo quantity must be positive".to_string());
        }
    }

    Ok(())
}

fn validate_dock_throughput(dock_throughput: &[u64], location_count: usize) -> Result<(), String> {
    if dock_throughput.len() != location_count {
        return Err(format!(
            "dock_throughput length {} does not match location count {}",
            dock_throughput.len(),
            location_count
        ));
    }

    for (location, throughput) in dock_throughput.iter().enumerate() {
        if *throughput == 0 {
            return Err(format!("dock_throughput[{location}] must be positive"));
        }
    }

    Ok(())
}

fn validate_transfer_amount(amount: u64) -> Result<(), String> {
    if amount == 0 {
        Err("transfer amount must be positive".to_string())
    } else {
        Ok(())
    }
}

fn validate_travel_times(travel_times: &[Vec<u64>]) -> Result<(), String> {
    if travel_times.is_empty() {
        return Err("travel_times must not be empty".to_string());
    }

    let size = travel_times.len();
    for (row_index, row) in travel_times.iter().enumerate() {
        if row.len() != size {
            return Err("travel_times must be square".to_string());
        }

        for (column_index, travel_time) in row.iter().enumerate() {
            if row_index == column_index {
                if *travel_time != 0 {
                    return Err(format!(
                        "travel_times[{row_index}][{column_index}] must be 0"
                    ));
                }
            } else if *travel_time == 0 {
                return Err(format!(
                    "travel_times[{row_index}][{column_index}] must be positive"
                ));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct RawCommand {
    command: String,
    parameters: serde_json::Value,
}

impl RawCommand {
    fn into_command(self) -> Result<Command, String> {
        match self.command.as_str() {
            "init" => parse_params(self.parameters).map(Command::Init),
            "tick" => parse_params(self.parameters).map(Command::Tick),
            "set_destination" => parse_params(self.parameters).map(Command::SetDestination),
            "get_state" => parse_params(self.parameters).map(Command::GetState),
            "get_state_at" => parse_params(self.parameters).map(Command::GetStateAt),
            "dock" => parse_params(self.parameters).map(Command::Dock),
            "undock" => parse_params(self.parameters).map(Command::Undock),
            "load" => parse_params(self.parameters).map(Command::Load),
            "unload" => parse_params(self.parameters).map(Command::Unload),
            "swap" => parse_params(self.parameters).map(Command::Swap),
            _ => Err(format!("unknown command {}", self.command)),
        }
    }
}

fn parse_params<T>(value: serde_json::Value) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(value).map_err(|err| format!("invalid parameters: {err}"))
}

fn default_transfer_amount() -> u64 {
    1
}

#[derive(Debug)]
pub enum Command {
    Init(InitParams),
    Tick(TickParams),
    SetDestination(SetDestinationParams),
    GetState(EmptyParams),
    GetStateAt(GetStateAtParams),
    Dock(VesselParams),
    Undock(VesselParams),
    Load(TransferParams),
    Unload(TransferParams),
    Swap(SwapParams),
}

#[derive(Debug, Deserialize)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize)]
pub struct InitParams {
    pub travel_times: Vec<Vec<u64>>,
    pub dock_throughput: Vec<u64>,
    pub vessels: Vec<InitVessel>,
    #[serde(default)]
    pub cargo_manifest: Option<Vec<CargoManifestEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct InitVessel {
    pub id: String,
    pub location: usize,
}

#[derive(Debug, Deserialize)]
pub struct TickParams {
    #[serde(default)]
    pub cargo_manifest: Option<Vec<CargoManifestEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct CargoManifestEntry {
    pub location: usize,
    pub destination: usize,
    pub quantity: u64,
}

#[derive(Debug, Deserialize)]
pub struct SetDestinationParams {
    pub vessel_id: String,
    pub destination: usize,
}

#[derive(Debug, Deserialize)]
pub struct GetStateAtParams {
    pub tick: u64,
}

#[derive(Debug, Deserialize)]
pub struct VesselParams {
    pub vessel_id: String,
}

#[derive(Debug, Deserialize)]
pub struct TransferParams {
    pub vessel_id: String,
    pub destination: usize,
    #[serde(default = "default_transfer_amount")]
    pub amount: u64,
}

#[derive(Debug, Deserialize)]
pub struct SwapParams {
    pub vessel_id: String,
    pub load_destination: usize,
    pub unload_destination: usize,
    #[serde(default = "default_transfer_amount")]
    pub amount: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_payload(response: Option<Response>) -> WorldState {
        match response {
            Some(Response::Ok(payload)) => payload,
            other => panic!("expected ok response, got {other:?}"),
        }
    }

    fn error_message(response: Option<Response>) -> String {
        match response {
            Some(Response::Error(payload)) => payload.message,
            other => panic!("expected error response, got {other:?}"),
        }
    }

    fn init(engine: &mut Engine) -> WorldState {
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,4,7],[4,0,3],[7,3,0]],"dock_throughput":[2,4,1],"vessels":[{"id":"v1","location":0},{"id":"v2","location":2}]}}"#,
        ))
    }

    fn cargo(quantity: u64, destination: usize) -> CargoInventory {
        let mut inventory = CargoInventory::default();
        inventory.add(destination, quantity);
        inventory
    }

    fn vessel(status: VesselStatus, cargo: CargoInventory) -> VesselState {
        VesselState { status, cargo }
    }

    #[test]
    fn init_creates_tick_zero_world_with_docks_and_empty_cargo() {
        let mut engine = Engine::new();
        let state = init(&mut engine);

        assert_eq!(state.tick, 0);
        assert_eq!(
            state.vessels.get("v1"),
            Some(&vessel(
                VesselStatus::Idle { location: 0 },
                CargoInventory::default()
            ))
        );
        assert_eq!(state.docks.len(), 3);
        assert_eq!(
            state.docks.get(&0),
            Some(&DockState {
                cargo: CargoInventory::default()
            })
        );
    }

    #[test]
    fn command_before_init_returns_error() {
        let mut engine = Engine::new();

        let message =
            error_message(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert_eq!(message, "engine is not initialized");
    }

    #[test]
    fn second_init_returns_error_and_keeps_state() {
        let mut engine = Engine::new();
        init(&mut engine);

        let message = error_message(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[1,1],"vessels":[{"id":"other","location":0}]}}"#,
        ));
        let state = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert_eq!(message, "engine is already initialized");
        assert!(state.vessels.contains_key("v1"));
        assert!(!state.vessels.contains_key("other"));
    }

    #[test]
    fn invalid_init_can_be_retried() {
        let mut engine = Engine::new();

        let message = error_message(
            engine.handle_line(
                r#"{"command":"init","parameters":{"travel_times":[],"dock_throughput":[],"vessels":[]}}"#,
            ),
        );
        let state = init(&mut engine);

        assert_eq!(message, "travel_times must not be empty");
        assert_eq!(state.tick, 0);
    }

    #[test]
    fn validates_init_inputs() {
        let cases = [
            (
                r#"{"command":"init","parameters":{"travel_times":[],"dock_throughput":[],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0,1],[1]],"dock_throughput":[1,1],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times must be square",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[1,1],[1,0]],"dock_throughput":[1,1],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times[0][0] must be 0",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0,0],[1,0]],"dock_throughput":[1,1],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times[0][1] must be positive",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[],"vessels":[{"id":"v1","location":0}]}}"#,
                "dock_throughput length 0 does not match location count 1",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[0],"vessels":[{"id":"v1","location":0}]}}"#,
                "dock_throughput[0] must be positive",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[]}}"#,
                "vessels must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"","location":0}]}}"#,
                "vessel id must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"v1","location":0},{"id":"v1","location":0}]}}"#,
                "duplicate vessel id v1",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"v1","location":1}]}}"#,
                "vessel v1 location 1 is out of range",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":1,"destination":0,"quantity":1}]}}"#,
                "cargo location 1 is out of range",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":1}]}}"#,
                "cargo destination 1 is out of range",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"dock_throughput":[1],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":0,"quantity":0}]}}"#,
                "cargo quantity must be positive",
            ),
        ];

        for (input, expected) in cases {
            let mut engine = Engine::new();
            assert_eq!(error_message(engine.handle_line(input)), expected);
        }
    }

    #[test]
    fn init_requires_dock_throughput() {
        let mut engine = Engine::new();

        let message = error_message(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0]],"vessels":[{"id":"v1","location":0}]}}"#,
        ));

        assert!(message.contains("missing field `dock_throughput`"));
    }

    #[test]
    fn cargo_manifest_initializes_docks_sorted_and_aggregated() {
        let mut engine = Engine::new();
        let state = ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[3,3],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":1},{"location":0,"destination":1,"quantity":2}]}}"#,
        ));

        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(3, 1));
    }

    #[test]
    fn set_destination_overwrites_before_tick_and_preserves_cargo() {
        let mut engine = Engine::new();
        init(&mut engine);

        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":2}}"#,
        ));
        let state = ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));

        assert_eq!(
            state.vessels.get("v1"),
            Some(&vessel(
                VesselStatus::Transit {
                    from: 0,
                    to: 1,
                    arrives_at_tick: 4
                },
                CargoInventory::default()
            ))
        );
    }

    #[test]
    fn set_destination_rejects_after_transit_is_committed() {
        let mut engine = Engine::new();
        init(&mut engine);

        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let message = error_message(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":2}}"#,
        ));

        assert_eq!(message, "vessel v1 is not idle");
    }

    #[test]
    fn same_location_destination_cancels_pending_trip() {
        let mut engine = Engine::new();
        init(&mut engine);

        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        let state = ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":0}}"#,
        ));

        assert_eq!(
            state.vessels.get("v1"),
            Some(&vessel(
                VesselStatus::Idle { location: 0 },
                CargoInventory::default()
            ))
        );
    }

    #[test]
    fn vessel_arrives_exactly_on_arrival_tick_with_cargo_still_onboard() {
        let mut engine = Engine::new();
        init(&mut engine);

        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));

        for _ in 0..3 {
            let state = ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
            assert!(matches!(
                state.vessels.get("v1"),
                Some(VesselState {
                    status: VesselStatus::Transit { .. },
                    ..
                })
            ));
        }

        let state = ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        assert_eq!(
            state.vessels.get("v1"),
            Some(&vessel(
                VesselStatus::Idle { location: 1 },
                CargoInventory::default()
            ))
        );
    }

    #[test]
    fn dock_and_destination_overwrite_each_other_before_tick() {
        let mut engine = Engine::new();
        init(&mut engine);

        let docked =
            ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        assert!(matches!(
            docked.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Docked { .. },
                ..
            })
        ));

        let transit = ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        assert!(matches!(
            transit.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Transit { .. },
                ..
            })
        ));

        let docked =
            ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        assert!(matches!(
            docked.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Docked { .. },
                ..
            })
        ));
    }

    #[test]
    fn dock_and_undock_are_last_intent_wins_before_tick() {
        let mut engine = Engine::new();
        init(&mut engine);

        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        let state = ok_payload(
            engine.handle_line(r#"{"command":"undock","parameters":{"vessel_id":"v1"}}"#),
        );
        assert!(matches!(
            state.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Idle { .. },
                ..
            })
        ));

        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        ok_payload(engine.handle_line(r#"{"command":"undock","parameters":{"vessel_id":"v1"}}"#));
        let state =
            ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        assert!(matches!(
            state.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Docked { .. },
                ..
            })
        ));
    }

    #[test]
    fn docked_vessel_can_load_and_then_undock_but_not_load_after_pending_undock() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[3,3],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":1}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        ok_payload(
            engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ),
        );
        let state = ok_payload(
            engine.handle_line(r#"{"command":"undock","parameters":{"vessel_id":"v1"}}"#),
        );
        assert!(matches!(
            state.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Idle { .. },
                ..
            })
        ));

        let message =
            error_message(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(message, "vessel v1 is not docked");
    }

    #[test]
    fn load_unload_delivery_and_swap_update_inventories() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[5,5],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":2}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let state =
            ok_payload(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(1, 1));

        let state = ok_payload(engine.handle_line(
            r#"{"command":"unload","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(2, 1));

        ok_payload(
            engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ),
        );
        let state = ok_payload(engine.handle_line(
            r#"{"command":"swap","parameters":{"vessel_id":"v1","load_destination":1,"unload_destination":1}}"#,
        ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(1, 1));
        assert_eq!(state.vessels.get("v1").unwrap().cargo, cargo(1, 1));
    }

    #[test]
    fn load_amount_moves_multiple_units_and_counts_throughput() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[2,2],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":3}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let state = ok_payload(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1,"amount":2}}"#,
        ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(1, 1));
        assert_eq!(state.vessels.get("v1").unwrap().cargo, cargo(2, 1));

        let message =
            error_message(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(
            message,
            "transfer of 3 operations exceeds dock throughput of 2 at location 0"
        );
    }

    #[test]
    fn unload_amount_moves_multiple_units_back_to_dock() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[4,4],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":2}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1,"amount":2}}"#,
        ));

        let state = ok_payload(engine.handle_line(
            r#"{"command":"unload","parameters":{"vessel_id":"v1","destination":1,"amount":2}}"#,
        ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(2, 1));
        assert_eq!(
            state.vessels.get("v1").unwrap().cargo,
            CargoInventory::default()
        );
    }

    #[test]
    fn unload_amount_delivers_matching_destination_at_current_location() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[4,4],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":0,"quantity":2}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":0,"amount":2}}"#,
        ));

        let state = ok_payload(engine.handle_line(
            r#"{"command":"unload","parameters":{"vessel_id":"v1","destination":0,"amount":2}}"#,
        ));
        assert_eq!(
            state.docks.get(&0).unwrap().cargo,
            CargoInventory::default()
        );
        assert_eq!(
            state.vessels.get("v1").unwrap().cargo,
            CargoInventory::default()
        );
    }

    #[test]
    fn swap_amount_loads_and_unloads_multiple_units() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1,1],[1,0,1],[1,1,0]],"dock_throughput":[2,2,2],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":2},{"location":0,"destination":2,"quantity":2}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":2,"amount":2}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let state = ok_payload(engine.handle_line(
            r#"{"command":"swap","parameters":{"vessel_id":"v1","load_destination":1,"unload_destination":2,"amount":2}}"#,
        ));
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(2, 2));
        assert_eq!(state.vessels.get("v1").unwrap().cargo, cargo(2, 1));
    }

    #[test]
    fn amount_exceeding_available_cargo_leaves_state_and_throughput_unchanged() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[2,2],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":1}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let before = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));
        let message = error_message(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1,"amount":2}}"#,
        ));
        let after = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));
        assert_eq!(message, "cargo with destination 1 is not available");
        assert_eq!(before, after);

        let state =
            ok_payload(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(state.vessels.get("v1").unwrap().cargo, cargo(1, 1));
    }

    #[test]
    fn zero_transfer_amount_is_rejected() {
        let mut engine = Engine::new();
        init(&mut engine);

        let message = error_message(engine.handle_line(
            r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1,"amount":0}}"#,
        ));
        assert_eq!(message, "transfer amount must be positive");
    }

    #[test]
    fn unload_delivers_matching_destination_at_current_location() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[5,5],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":0,"quantity":1}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(
            engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":0}}"#,
            ),
        );

        let state = ok_payload(engine.handle_line(
            r#"{"command":"unload","parameters":{"vessel_id":"v1","destination":0}}"#,
        ));

        assert_eq!(
            state.docks.get(&0).unwrap().cargo,
            CargoInventory::default()
        );
        assert_eq!(
            state.vessels.get("v1").unwrap().cargo,
            CargoInventory::default()
        );
    }

    #[test]
    fn throughput_rejects_excess_operations_and_resets_on_tick() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[1,1],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":2}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(
            engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ),
        );

        let message =
            error_message(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(
            message,
            "transfer of 2 operations exceeds dock throughput of 1 at location 0"
        );

        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        let state =
            ok_payload(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(state.vessels.get("v1").unwrap().cargo, cargo(2, 1));
    }

    #[test]
    fn failed_cargo_command_leaves_state_and_throughput_unchanged() {
        let mut engine = Engine::new();
        ok_payload(engine.handle_line(
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[1,1],"vessels":[{"id":"v1","location":0}]}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let before = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));
        let message =
            error_message(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        let after = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));
        assert_eq!(message, "cargo with destination 1 is not available");
        assert_eq!(before, after);

        let message =
            error_message(engine.handle_line(
                r#"{"command":"load","parameters":{"vessel_id":"v1","destination":1}}"#,
            ));
        assert_eq!(message, "cargo with destination 1 is not available");
    }

    #[test]
    fn tick_manifest_adds_cargo_after_commit() {
        let mut engine = Engine::new();
        init(&mut engine);

        let state = ok_payload(engine.handle_line(
            r#"{"command":"tick","parameters":{"cargo_manifest":[{"location":0,"destination":2,"quantity":3}]}}"#,
        ));

        assert_eq!(state.tick, 1);
        assert_eq!(state.docks.get(&0).unwrap().cargo, cargo(3, 2));
    }

    #[test]
    fn get_state_at_returns_initial_committed_state_after_later_plans() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));

        let historical =
            ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":0}}"#));
        let planned = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert_eq!(
            historical.vessels.get("v1"),
            Some(&vessel(
                VesselStatus::Idle { location: 0 },
                CargoInventory::default()
            ))
        );
        assert!(matches!(
            planned.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Transit { .. },
                ..
            })
        ));
    }

    #[test]
    fn get_state_at_records_each_successful_tick() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let tick_one =
            ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":1}}"#));
        let tick_two =
            ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":2}}"#));

        assert_eq!(tick_one.tick, 1);
        assert_eq!(tick_two.tick, 2);
        assert!(matches!(
            tick_one.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Transit {
                    arrives_at_tick: 4,
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn get_state_at_ignores_pending_non_tick_changes() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));

        let historical =
            ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":1}}"#));
        let planned = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert!(matches!(
            historical.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Idle { .. },
                ..
            })
        ));
        assert!(matches!(
            planned.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Docked { .. },
                ..
            })
        ));
    }

    #[test]
    fn get_state_at_current_tick_returns_latest_committed_state() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(
            r#"{"command":"tick","parameters":{"cargo_manifest":[{"location":0,"destination":2,"quantity":1}]}}"#,
        ));

        let historical =
            ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":1}}"#));

        assert_eq!(historical.tick, 1);
        assert_eq!(historical.docks.get(&0).unwrap().cargo, cargo(1, 2));
    }

    #[test]
    fn get_state_at_future_tick_returns_error() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        let message = error_message(
            engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":99}}"#),
        );

        assert_eq!(message, "tick 99 is in the future (current tick: 1)");
    }

    #[test]
    fn get_state_at_negative_tick_is_parameter_error() {
        let mut engine = Engine::new();
        init(&mut engine);

        let message = error_message(
            engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":-1}}"#),
        );

        assert!(message.contains("invalid parameters"));
    }

    #[test]
    fn get_state_at_does_not_mutate_planned_state() {
        let mut engine = Engine::new();
        init(&mut engine);
        ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));

        ok_payload(engine.handle_line(r#"{"command":"get_state_at","parameters":{"tick":0}}"#));
        let planned = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert!(matches!(
            planned.vessels.get("v1"),
            Some(VesselState {
                status: VesselStatus::Transit { .. },
                ..
            })
        ));
    }

    #[test]
    fn invalid_destination_leaves_state_unchanged() {
        let mut engine = Engine::new();
        init(&mut engine);

        let before = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));
        let message = error_message(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":99}}"#,
        ));
        let after = ok_payload(engine.handle_line(r#"{"command":"get_state","parameters":{}}"#));

        assert_eq!(message, "destination 99 is out of range");
        assert_eq!(before, after);
    }

    #[test]
    fn blank_lines_produce_no_response() {
        let mut engine = Engine::new();

        assert_eq!(engine.handle_line("   "), None);
    }
}
