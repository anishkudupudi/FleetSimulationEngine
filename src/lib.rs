use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

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
    pub vessels: BTreeMap<String, VesselState>,
    pub docks: BTreeMap<usize, DockState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockState {
    pub cargo: CargoInventory,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CargoInventory {
    by_destination: BTreeMap<usize, u64>,
}

impl CargoInventory {
    fn add(&mut self, destination: usize, quantity: u64) {
        if quantity == 0 {
            return;
        }

        *self.by_destination.entry(destination).or_insert(0) += quantity;
    }

    fn remove_one(&mut self, destination: usize) -> Result<(), String> {
        let quantity = self
            .by_destination
            .get_mut(&destination)
            .ok_or_else(|| format!("cargo with destination {destination} is not available"))?;

        *quantity -= 1;
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
        let mut seq = serializer.serialize_seq(Some(self.by_destination.len()))?;
        for (destination, quantity) in &self.by_destination {
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
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VesselState {
    Idle {
        location: usize,
        cargo: CargoInventory,
    },
    Docked {
        location: usize,
        cargo: CargoInventory,
    },
    Transit {
        from: usize,
        to: usize,
        arrives_at_tick: u64,
        cargo: CargoInventory,
    },
}

impl VesselState {
    fn cargo(&self) -> &CargoInventory {
        match self {
            Self::Idle { cargo, .. } | Self::Docked { cargo, .. } | Self::Transit { cargo, .. } => {
                cargo
            }
        }
    }

    fn cargo_mut(&mut self) -> &mut CargoInventory {
        match self {
            Self::Idle { cargo, .. } | Self::Docked { cargo, .. } | Self::Transit { cargo, .. } => {
                cargo
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Engine {
    travel_times: Option<Vec<Vec<u64>>>,
    dock_throughput: Option<Vec<u64>>,
    pending_dock_operations: Option<Vec<u64>>,
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
        let mut vessels = BTreeMap::new();

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
                    VesselState::Idle {
                        location: vessel.location,
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
            let arrival = match vessel {
                VesselState::Transit {
                    to,
                    arrives_at_tick,
                    cargo,
                    ..
                } if *arrives_at_tick <= next.tick => Some((*to, cargo.clone())),
                _ => None,
            };

            if let Some((location, cargo)) = arrival {
                *vessel = VesselState::Idle { location, cargo };
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
            Some(VesselState::Idle { location, .. }) => *location,
            Some(_) => return Err(format!("vessel {} is not idle", params.vessel_id)),
            None => return Err(format!("unknown vessel {}", params.vessel_id)),
        };

        let planned_cargo = self.planned_vessel_cargo(&params.vessel_id)?;
        let planned_state = if params.destination == current_location {
            VesselState::Idle {
                location: current_location,
                cargo: planned_cargo,
            }
        } else {
            let travel_time = travel_times[current_location][params.destination];
            let arrives_at_tick = committed
                .tick
                .checked_add(travel_time)
                .ok_or_else(|| format!("travel time overflow for vessel {}", params.vessel_id))?;

            VesselState::Transit {
                from: current_location,
                to: params.destination,
                arrives_at_tick,
                cargo: planned_cargo,
            }
        };

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned.vessels.insert(params.vessel_id, planned_state);
        Ok(planned.clone())
    }

    fn dock(&mut self, params: VesselParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let location = match self.committed_vessel(&params.vessel_id)? {
            VesselState::Idle { location, .. } | VesselState::Docked { location, .. } => *location,
            VesselState::Transit { .. } => {
                return Err(format!("vessel {} is not idle", params.vessel_id));
            }
        };

        let cargo = self.planned_vessel_cargo(&params.vessel_id)?;
        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .insert(params.vessel_id, VesselState::Docked { location, cargo });
        Ok(planned.clone())
    }

    fn undock(&mut self, params: VesselParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let location = match self.committed_vessel(&params.vessel_id)? {
            VesselState::Docked { location, .. } => *location,
            VesselState::Idle { .. } => match self
                .planned
                .as_ref()
                .expect("planned state exists after initialization")
                .vessels
                .get(&params.vessel_id)
            {
                Some(VesselState::Docked { location, .. }) => *location,
                _ => return Err(format!("vessel {} is not docked", params.vessel_id)),
            },
            VesselState::Transit { .. } => {
                return Err(format!("vessel {} is not docked", params.vessel_id));
            }
        };

        let cargo = self.planned_vessel_cargo(&params.vessel_id)?;
        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned
            .vessels
            .insert(params.vessel_id, VesselState::Idle { location, cargo });
        Ok(planned.clone())
    }

    fn load(&mut self, params: TransferParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location)?;

        let result = self.apply_load(&params.vessel_id, location, params.destination);
        if result.is_err() {
            self.release_dock_operation(location);
        }

        result
    }

    fn unload(&mut self, params: TransferParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location)?;

        let result = self.apply_unload(&params.vessel_id, location, params.destination);
        if result.is_err() {
            self.release_dock_operation(location);
        }

        result
    }

    fn swap(&mut self, params: SwapParams) -> Result<WorldState, String> {
        self.ensure_initialized()?;
        let location = self.validate_cargo_command(&params.vessel_id)?;
        self.reserve_dock_operation(location)?;

        let before = self.planned.clone();
        let load_result = self.apply_load(&params.vessel_id, location, params.load_destination);
        let result = match load_result {
            Ok(_) => self.apply_unload(&params.vessel_id, location, params.unload_destination),
            Err(err) => Err(err),
        };

        if result.is_err() {
            self.planned = before;
            self.release_dock_operation(location);
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

    fn planned_vessel_cargo(&self, vessel_id: &str) -> Result<CargoInventory, String> {
        self.planned
            .as_ref()
            .expect("planned state exists after initialization")
            .vessels
            .get(vessel_id)
            .map(|vessel| vessel.cargo().clone())
            .ok_or_else(|| format!("unknown vessel {vessel_id}"))
    }

    fn validate_cargo_command(&self, vessel_id: &str) -> Result<usize, String> {
        let committed_location = match self.committed_vessel(vessel_id)? {
            VesselState::Docked { location, .. } => *location,
            _ => return Err(format!("vessel {vessel_id} is not docked")),
        };

        match self
            .planned
            .as_ref()
            .expect("planned state exists after initialization")
            .vessels
            .get(vessel_id)
        {
            Some(VesselState::Docked { location, .. }) if *location == committed_location => {
                Ok(committed_location)
            }
            Some(_) => Err(format!("vessel {vessel_id} is not docked")),
            None => Err(format!("unknown vessel {vessel_id}")),
        }
    }

    fn reserve_dock_operation(&mut self, location: usize) -> Result<(), String> {
        let limit = self
            .dock_throughput
            .as_ref()
            .expect("dock throughput exists after initialization")[location];
        let operations = self
            .pending_dock_operations
            .as_mut()
            .expect("dock operation counters exist after initialization");
        let next_count = operations[location] + 1;

        if next_count > limit {
            return Err(format!(
                "transfer of {next_count} operations exceeds dock throughput of {limit} at location {location}"
            ));
        }

        operations[location] = next_count;
        Ok(())
    }

    fn release_dock_operation(&mut self, location: usize) {
        let operations = self
            .pending_dock_operations
            .as_mut()
            .expect("dock operation counters exist after initialization");
        operations[location] -= 1;
    }

    fn apply_load(
        &mut self,
        vessel_id: &str,
        location: usize,
        destination: usize,
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
            .remove_one(destination)?;
        planned
            .vessels
            .get_mut(vessel_id)
            .expect("planned vessel exists after validation")
            .cargo_mut()
            .add(destination, 1);

        Ok(planned.clone())
    }

    fn apply_unload(
        &mut self,
        vessel_id: &str,
        location: usize,
        destination: usize,
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
            .remove_one(destination)?;

        if destination != location {
            planned
                .docks
                .get_mut(&location)
                .expect("dock exists for valid location")
                .cargo
                .add(destination, 1);
        }

        Ok(planned.clone())
    }
}

fn empty_docks(location_count: usize) -> BTreeMap<usize, DockState> {
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
    docks: &mut BTreeMap<usize, DockState>,
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

#[derive(Debug)]
pub enum Command {
    Init(InitParams),
    Tick(TickParams),
    SetDestination(SetDestinationParams),
    GetState(EmptyParams),
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
pub struct VesselParams {
    pub vessel_id: String,
}

#[derive(Debug, Deserialize)]
pub struct TransferParams {
    pub vessel_id: String,
    pub destination: usize,
}

#[derive(Debug, Deserialize)]
pub struct SwapParams {
    pub vessel_id: String,
    pub load_destination: usize,
    pub unload_destination: usize,
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

    #[test]
    fn init_creates_tick_zero_world_with_docks_and_empty_cargo() {
        let mut engine = Engine::new();
        let state = init(&mut engine);

        assert_eq!(state.tick, 0);
        assert_eq!(
            state.vessels.get("v1"),
            Some(&VesselState::Idle {
                location: 0,
                cargo: CargoInventory::default()
            })
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
            Some(&VesselState::Transit {
                from: 0,
                to: 1,
                arrives_at_tick: 4,
                cargo: CargoInventory::default()
            })
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
            Some(&VesselState::Idle {
                location: 0,
                cargo: CargoInventory::default()
            })
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
                Some(VesselState::Transit { .. })
            ));
        }

        let state = ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));
        assert_eq!(
            state.vessels.get("v1"),
            Some(&VesselState::Idle {
                location: 1,
                cargo: CargoInventory::default()
            })
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
            Some(VesselState::Docked { .. })
        ));

        let transit = ok_payload(engine.handle_line(
            r#"{"command":"set_destination","parameters":{"vessel_id":"v1","destination":1}}"#,
        ));
        assert!(matches!(
            transit.vessels.get("v1"),
            Some(VesselState::Transit { .. })
        ));

        let docked =
            ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        assert!(matches!(
            docked.vessels.get("v1"),
            Some(VesselState::Docked { .. })
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
            Some(VesselState::Idle { .. })
        ));

        ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        ok_payload(engine.handle_line(r#"{"command":"tick","parameters":{}}"#));

        ok_payload(engine.handle_line(r#"{"command":"undock","parameters":{"vessel_id":"v1"}}"#));
        let state =
            ok_payload(engine.handle_line(r#"{"command":"dock","parameters":{"vessel_id":"v1"}}"#));
        assert!(matches!(
            state.vessels.get("v1"),
            Some(VesselState::Docked { .. })
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
            Some(VesselState::Idle { .. })
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
        assert_eq!(state.vessels.get("v1").unwrap().cargo(), &cargo(1, 1));
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
            state.vessels.get("v1").unwrap().cargo(),
            &CargoInventory::default()
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
        assert_eq!(state.vessels.get("v1").unwrap().cargo(), &cargo(2, 1));
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
