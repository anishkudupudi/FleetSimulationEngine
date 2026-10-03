use serde::{Deserialize, Serialize};
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VesselState {
    Idle {
        location: usize,
    },
    Transit {
        from: usize,
        to: usize,
        arrives_at_tick: u64,
    },
}

#[derive(Debug, Default)]
pub struct Engine {
    travel_times: Option<Vec<Vec<u64>>>,
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
            Command::Tick(_) => self.tick(),
            Command::SetDestination(params) => self.set_destination(params),
            Command::GetState(_) => self.get_state(),
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
                    },
                )
                .is_some()
            {
                return Err(format!("duplicate vessel id {}", vessel.id));
            }
        }

        let state = WorldState { tick: 0, vessels };
        self.travel_times = Some(params.travel_times);
        self.committed = Some(state.clone());
        self.planned = Some(state.clone());
        Ok(state)
    }

    fn tick(&mut self) -> Result<WorldState, String> {
        self.ensure_initialized()?;

        let mut next = self
            .planned
            .clone()
            .expect("planned state exists after initialization");
        next.tick = next
            .tick
            .checked_add(1)
            .ok_or_else(|| "tick overflow".to_string())?;

        for vessel in next.vessels.values_mut() {
            let arrival_location = match vessel {
                VesselState::Transit {
                    to,
                    arrives_at_tick,
                    ..
                } if *arrives_at_tick <= next.tick => Some(*to),
                _ => None,
            };

            if let Some(location) = arrival_location {
                *vessel = VesselState::Idle { location };
            }
        }

        self.committed = Some(next.clone());
        self.planned = Some(next.clone());
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
            Some(VesselState::Idle { location }) => *location,
            Some(VesselState::Transit { .. }) => {
                return Err(format!("vessel {} is not idle", params.vessel_id));
            }
            None => return Err(format!("unknown vessel {}", params.vessel_id)),
        };

        let planned_state = if params.destination == current_location {
            VesselState::Idle {
                location: current_location,
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
            }
        };

        let planned = self
            .planned
            .as_mut()
            .expect("planned state exists after initialization");
        planned.vessels.insert(params.vessel_id, planned_state);
        Ok(planned.clone())
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
    Tick(EmptyParams),
    SetDestination(SetDestinationParams),
    GetState(EmptyParams),
}

#[derive(Debug, Deserialize)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize)]
pub struct InitParams {
    pub travel_times: Vec<Vec<u64>>,
    pub vessels: Vec<InitVessel>,
}

#[derive(Debug, Deserialize)]
pub struct InitVessel {
    pub id: String,
    pub location: usize,
}

#[derive(Debug, Deserialize)]
pub struct SetDestinationParams {
    pub vessel_id: String,
    pub destination: usize,
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
            r#"{"command":"init","parameters":{"travel_times":[[0,4,7],[4,0,3],[7,3,0]],"vessels":[{"id":"v1","location":0},{"id":"v2","location":2}]}}"#,
        ))
    }

    #[test]
    fn init_creates_tick_zero_world() {
        let mut engine = Engine::new();
        let state = init(&mut engine);

        assert_eq!(state.tick, 0);
        assert_eq!(
            state.vessels.get("v1"),
            Some(&VesselState::Idle { location: 0 })
        );
        assert_eq!(
            state.vessels.get("v2"),
            Some(&VesselState::Idle { location: 2 })
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
            r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"vessels":[{"id":"other","location":0}]}}"#,
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
            engine
                .handle_line(r#"{"command":"init","parameters":{"travel_times":[],"vessels":[]}}"#),
        );
        let state = init(&mut engine);

        assert_eq!(message, "travel_times must not be empty");
        assert_eq!(state.tick, 0);
    }

    #[test]
    fn validates_init_inputs() {
        let cases = [
            (
                r#"{"command":"init","parameters":{"travel_times":[],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0,1],[1]],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times must be square",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[1,1],[1,0]],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times[0][0] must be 0",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0,0],[1,0]],"vessels":[{"id":"v1","location":0}]}}"#,
                "travel_times[0][1] must be positive",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"vessels":[]}}"#,
                "vessels must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"vessels":[{"id":"","location":0}]}}"#,
                "vessel id must not be empty",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"vessels":[{"id":"v1","location":0},{"id":"v1","location":0}]}}"#,
                "duplicate vessel id v1",
            ),
            (
                r#"{"command":"init","parameters":{"travel_times":[[0]],"vessels":[{"id":"v1","location":1}]}}"#,
                "vessel v1 location 1 is out of range",
            ),
        ];

        for (input, expected) in cases {
            let mut engine = Engine::new();
            assert_eq!(error_message(engine.handle_line(input)), expected);
        }
    }

    #[test]
    fn set_destination_overwrites_before_tick() {
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
                arrives_at_tick: 4
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
            Some(&VesselState::Idle { location: 0 })
        );
    }

    #[test]
    fn vessel_arrives_exactly_on_arrival_tick() {
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
            Some(&VesselState::Idle { location: 1 })
        );
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
