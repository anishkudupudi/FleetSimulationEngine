const state = {
  mode: "planned",
  livePlanned: null,
  committed: null,
  historical: null,
  selectedTick: 0,
  travelTimes: null,
  selectedVessel: null,
  selectedDock: null,
  sideView: "vessels",
  mapView: { x: 0, y: 0, width: 900, height: 520 },
  mapDrag: null,
  suppressMapClick: false,
  log: [],
};

const els = {
  initStatus: document.querySelector("#init-status"),
  tickStatus: document.querySelector("#tick-status"),
  committedStatus: document.querySelector("#committed-status"),
  map: document.querySelector("#world-map"),
  vesselsView: document.querySelector("#vessels-view"),
  docksView: document.querySelector("#docks-view"),
  responseLog: document.querySelector("#response-log"),
  rawCommand: document.querySelector("#raw-command"),
  generatedCommand: document.querySelector("#generated-command"),
  commandSelect: document.querySelector("#command-select"),
  vesselId: document.querySelector("#vessel-id-input"),
  destination: document.querySelector("#destination-input"),
  amount: document.querySelector("#amount-input"),
  loadDestination: document.querySelector("#load-destination-input"),
  unloadDestination: document.querySelector("#unload-destination-input"),
  initInput: document.querySelector("#init-input"),
  tickManifestLocation: document.querySelector("#tick-manifest-location-input"),
  tickManifestDestination: document.querySelector("#tick-manifest-destination-input"),
  tickManifestQuantity: document.querySelector("#tick-manifest-quantity-input"),
  tickSlider: document.querySelector("#tick-slider"),
  tickInput: document.querySelector("#tick-input"),
  builderTick: document.querySelector("#builder-tick-input"),
  commandFields: document.querySelectorAll(".command-field"),
  terminalResizer: document.querySelector("#terminal-resizer"),
};

document.querySelector("#run-command-button").addEventListener("click", () => {
  runRawCommand();
});

document.querySelector("#run-lines-button").addEventListener("click", () => {
  runRawLines();
});

document.querySelector("#send-generated-button").addEventListener("click", () => {
  const command = parseJson(els.generatedCommand.value);
  if (command) {
    sendCommand(command, { source: "builder" });
  }
});

document.querySelector("#live-button").addEventListener("click", () => {
  state.mode = "planned";
  state.historical = null;
  state.selectedTick = latestTick();
  setActiveMode();
  render();
});

for (const button of document.querySelectorAll(".mode-button")) {
  button.addEventListener("click", async () => {
    state.mode = button.dataset.mode;
    if (state.mode === "planned") {
      state.historical = null;
      state.selectedTick = latestTick();
    }
    setActiveMode();
    if (state.mode === "committed" || state.mode === "compare") {
      await refreshCommitted();
    }
    render();
  });
}

for (const button of document.querySelectorAll(".side-tab")) {
  button.addEventListener("click", () => {
    setSideView(button.dataset.sideView);
  });
}

for (const button of document.querySelectorAll(".io-tab")) {
  button.addEventListener("click", () => {
    const selected = button.dataset.ioTab;
    for (const current of document.querySelectorAll(".io-tab")) {
      current.classList.toggle("active", current === button);
    }
    document.querySelector("#generated-pane").classList.toggle("hidden", selected !== "generated");
    document.querySelector("#raw-pane").classList.toggle("hidden", selected !== "raw");
  });
}

document.querySelector("#cycle-prev-button").addEventListener("click", () => cycleSideSelection(-1));
document.querySelector("#cycle-next-button").addEventListener("click", () => cycleSideSelection(1));

for (const input of [
  els.commandSelect,
  els.vesselId,
  els.destination,
  els.amount,
  els.loadDestination,
  els.unloadDestination,
  els.initInput,
  els.tickManifestLocation,
  els.tickManifestDestination,
  els.tickManifestQuantity,
  els.builderTick,
]) {
  input.addEventListener("input", updateGeneratedCommand);
}

els.commandSelect.addEventListener("change", () => {
  updateCommandFields();
  updateGeneratedCommand();
});

els.tickSlider.addEventListener("input", () => {
  els.tickInput.value = els.tickSlider.value;
});

els.tickSlider.addEventListener("change", async () => {
  await loadHistorical(Number(els.tickSlider.value));
});

els.tickInput.addEventListener("change", async () => {
  await loadHistorical(Number(els.tickInput.value || 0));
});

els.terminalResizer.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  const startY = event.clientY;
  const startHeight = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--terminal-height"), 10) || 310;

  const move = (moveEvent) => {
    const nextHeight = Math.max(220, Math.min(window.innerHeight - 220, startHeight + startY - moveEvent.clientY));
    document.documentElement.style.setProperty("--terminal-height", `${nextHeight}px`);
  };
  const stop = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", stop);
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", stop);
});

els.map.addEventListener("click", (event) => {
  if (state.suppressMapClick) {
    state.suppressMapClick = false;
    return;
  }
  if (!event.target.closest(".vessel-ship-hit") && !event.target.classList.contains("location-node")) {
    clearSelection();
  }
});

els.map.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    zoomMap(event);
  },
  { passive: false },
);

els.map.addEventListener("pointerdown", (event) => {
  if (event.target.closest(".vessel-ship-hit") || event.target.classList.contains("location-node")) {
    return;
  }

  els.map.setPointerCapture(event.pointerId);
  els.map.classList.add("panning");
  state.mapDrag = {
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    startView: { ...state.mapView },
    moved: false,
  };
});

els.map.addEventListener("pointermove", (event) => {
  if (!state.mapDrag) {
    return;
  }

  const rect = els.map.getBoundingClientRect();
  const dx = ((event.clientX - state.mapDrag.startX) / rect.width) * state.mapDrag.startView.width;
  const dy = ((event.clientY - state.mapDrag.startY) / rect.height) * state.mapDrag.startView.height;

  if (Math.abs(event.clientX - state.mapDrag.startX) > 3 || Math.abs(event.clientY - state.mapDrag.startY) > 3) {
    state.mapDrag.moved = true;
  }

  state.mapView.x = state.mapDrag.startView.x - dx;
  state.mapView.y = state.mapDrag.startView.y - dy;
  applyMapViewBox();
});

els.map.addEventListener("pointerup", (event) => {
  if (!state.mapDrag) {
    return;
  }

  state.suppressMapClick = state.mapDrag.moved;
  state.mapDrag = null;
  els.map.classList.remove("panning");
  if (els.map.hasPointerCapture(event.pointerId)) {
    els.map.releasePointerCapture(event.pointerId);
  }
});

els.map.addEventListener("pointercancel", () => {
  state.mapDrag = null;
  els.map.classList.remove("panning");
});

document.querySelector(".side-section").addEventListener("click", (event) => {
  if (event.target.classList.contains("side-section") || event.target.id === "vessels-view" || event.target.id === "docks-view") {
    clearSelection();
  }
});

updateGeneratedCommand();
updateCommandFields();
render();

async function runRawCommand() {
  const command = parseJson(els.rawCommand.value);
  if (command) {
    await sendCommand(command, { source: "raw" });
  }
}

async function runRawLines() {
  const lines = els.rawCommand.value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);

  for (const line of lines) {
    const command = parseJson(line);
    if (!command) {
      break;
    }
    await sendCommand(command, { source: "raw" });
  }
}

function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch (error) {
    appendLog({ command: text, response: { status: "error", payload: { message: error.message } } });
    renderLog();
    return null;
  }
}

async function sendCommand(command, options = {}) {
  let response;
  try {
    response = await postCommand(command);
  } catch (error) {
    response = { status: "error", payload: { message: error.message } };
  }

  appendLog({ command, response });

  if (response.status === "ok") {
    applyOkResponse(command, response.payload, options);
  }

  if (state.mode === "planned" && response.status === "ok" && command.command !== "get_state_at") {
    await refreshCommitted();
  }

  render();
  return response;
}

async function postCommand(command) {
  const response = await fetch("/command", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(command),
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }

  return response.json();
}

function applyOkResponse(command, payload, options) {
  if (command.command === "init") {
    state.travelTimes = command.parameters.travel_times || null;
    state.livePlanned = payload;
    state.committed = payload;
    state.historical = null;
    state.selectedTick = payload.tick;
    state.mode = "planned";
    setActiveMode();
    return;
  }

  if (command.command === "tick") {
    state.livePlanned = payload;
    state.committed = payload;
    state.historical = null;
    state.selectedTick = payload.tick;
    return;
  }

  if (command.command === "get_state_at") {
    if (options.target === "committed") {
      state.committed = payload;
    } else {
      state.historical = payload;
      state.selectedTick = payload.tick;
      state.mode = "historical";
      setActiveMode();
    }
    return;
  }

  if (command.command === "get_state") {
    state.livePlanned = payload;
    return;
  }

  state.livePlanned = payload;
}

async function refreshCommitted() {
  if (!state.livePlanned) {
    return;
  }

  const command = { command: "get_state_at", parameters: { tick: state.livePlanned.tick } };
  let response;
  try {
    response = await postCommand(command);
  } catch (error) {
    appendLog({ command, response: { status: "error", payload: { message: error.message } } });
    return;
  }

  if (response.status === "ok") {
    state.committed = response.payload;
  } else {
    appendLog({ command, response });
  }
}

async function loadHistorical(tick) {
  if (!state.livePlanned) {
    return;
  }
  const command = { command: "get_state_at", parameters: { tick } };
  const response = await sendCommand(command, { target: "historical" });
  if (response.status === "ok") {
    state.selectedTick = response.payload.tick;
  }
}

function appendLog(entry) {
  state.log.unshift(entry);
  state.log = state.log.slice(0, 50);
}

function render() {
  const visible = visibleState();
  renderHeader();
  renderTimeline();
  renderMap(visible);
  renderVessels(visible);
  renderDocks(visible);
  renderLog();
  updateCommandFields();
  updateGeneratedCommand();
}

function visibleState() {
  if (state.mode === "committed") {
    return state.committed || state.livePlanned;
  }
  if (state.mode === "historical") {
    return state.historical || state.committed || state.livePlanned;
  }
  return state.livePlanned;
}

function renderHeader() {
  els.initStatus.textContent = state.livePlanned ? "initialized" : "not initialized";
  els.tickStatus.textContent = state.livePlanned ? `tick ${state.livePlanned.tick}` : "tick -";
  if (state.mode === "historical") {
    els.committedStatus.textContent = `historical tick ${state.historical?.tick ?? state.selectedTick}`;
  } else if (state.mode === "committed") {
    els.committedStatus.textContent = "committed view";
  } else {
    els.committedStatus.textContent = "live view";
  }
}

function renderTimeline() {
  const maxTick = state.committed?.tick ?? state.livePlanned?.tick ?? 0;
  els.tickSlider.max = String(maxTick);
  els.tickInput.max = String(maxTick);
  const value = Math.min(state.selectedTick || 0, maxTick);
  els.tickSlider.value = String(value);
  els.tickInput.value = String(value);
  if (!els.builderTick.matches(":focus")) {
    els.builderTick.value = String(value);
  }
}

function setActiveMode() {
  for (const button of document.querySelectorAll(".mode-button")) {
    button.classList.toggle("active", button.dataset.mode === state.mode);
  }
}

function renderMap(world) {
  els.map.textContent = "";
  applyMapViewBox();
  if (!world) {
    svgText(450, 260, "Run init to create the world", "map-subtle");
    return;
  }

  const dockIds = sortedKeys(world.docks);
  const positions = locationPositions(dockIds);
  const compare = comparisonBaseline();
  const stationaryCounts = countStationaryVessels(world);
  const routeCounts = countTransitVessels(world);

  for (let i = 0; i < dockIds.length; i += 1) {
    for (let j = i + 1; j < dockIds.length; j += 1) {
      const from = positions[dockIds[i]];
      const to = positions[dockIds[j]];
      svgLine(from.x, from.y, to.x, to.y, "route-line");
    }
  }

  for (const [id, vessel] of Object.entries(world.vessels)) {
    if (vessel.state === "transit") {
      const from = positions[vessel.from];
      const to = positions[vessel.to];
      if (from && to) {
        const planned = compare && JSON.stringify(compare.vessels[id]) !== JSON.stringify(vessel);
        svgLine(from.x, from.y, to.x, to.y, `route-line active${planned ? " planned" : ""}`);
      }
    }
  }

  for (const id of dockIds) {
    const point = positions[id];
    const dock = world.docks[id];
    const changed = compare && JSON.stringify(compare.docks[id]) !== JSON.stringify(dock);
    const selected = state.selectedDock === id;
    const dockNode = svgCircle(point.x, point.y, 34, `location-node${changed ? " changed" : ""}${selected ? " selected" : ""}`, id);
    dockNode.addEventListener("click", (event) => {
      event.stopPropagation();
      selectDock(id, { scroll: true, render: true });
    });
    svgText(point.x, point.y - 2, `Dock ${id}`, "map-label");
    svgText(point.x, point.y + 15, `${cargoTotal(dock.cargo)} cargo`, "map-subtle");
  }

  const locationOffsets = {};
  const routeOffsets = {};
  for (const [id, vessel] of Object.entries(world.vessels)) {
    const planned = compare && JSON.stringify(compare.vessels[id]) !== JSON.stringify(vessel);
    if (vessel.state === "transit") {
      const from = positions[vessel.from];
      const to = positions[vessel.to];
      if (!from || !to) {
        continue;
      }
      const key = laneRouteKey(vessel.from, vessel.to);
      const laneIndex = routeOffsets[key] || 0;
      routeOffsets[key] = laneIndex + 1;
      const point = transitPoint(world.tick, vessel, from, to, laneIndex, routeCounts[key] || 1);
      const x = point.x;
      const y = point.y;
      drawVessel(id, x, y, `vessel-marker transit${planned ? " planned" : ""}`, point.angle);
    } else {
      const point = positions[vessel.location];
      if (!point) {
        continue;
      }
      const offsetIndex = locationOffsets[vessel.location] || 0;
      locationOffsets[vessel.location] = offsetIndex + 1;
      const count = stationaryCounts[vessel.location] || 1;
      const angle = -Math.PI / 2 + (Math.PI * 2 * offsetIndex) / count;
      const radius = vessel.state === "docked" ? 64 : 86;
      const x = point.x + Math.cos(angle) * radius;
      const y = point.y + Math.sin(angle) * radius;
      drawVessel(id, x, y, `vessel-marker${planned ? " planned" : ""}`, 0);
    }
  }
}

function countStationaryVessels(world) {
  const counts = {};
  for (const vessel of Object.values(world.vessels)) {
    if (vessel.state !== "transit") {
      counts[vessel.location] = (counts[vessel.location] || 0) + 1;
    }
  }
  return counts;
}

function countTransitVessels(world) {
  const counts = {};
  for (const vessel of Object.values(world.vessels)) {
    if (vessel.state === "transit") {
      const key = laneRouteKey(vessel.from, vessel.to);
      counts[key] = (counts[key] || 0) + 1;
    }
  }
  return counts;
}

function transitPoint(tick, vessel, from, to, laneIndex, laneCount) {
  const progress = Math.max(0.16, Math.min(0.84, transitProgress(tick, vessel)));
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.hypot(dx, dy) || 1;
  const lane = (laneIndex - (laneCount - 1) / 2) * 24;
  const offsetX = (-dy / length) * lane;
  const offsetY = (dx / length) * lane;

  return {
    x: from.x + dx * progress + offsetX,
    y: from.y + dy * progress + offsetY,
    angle: angleForRoute(from, to),
  };
}

function angleForRoute(from, to) {
  return (Math.atan2(to.y - from.y, to.x - from.x) * 180) / Math.PI + 90;
}

function drawVessel(id, x, y, className, angle) {
  const selected = state.selectedVessel === id;
  const vesselNode = svgShip(x, y, `${className.replace("vessel-marker", "vessel-ship")}${selected ? " selected" : ""}`, id, angle);
  vesselNode.addEventListener("click", (event) => {
    event.stopPropagation();
    selectVessel(id, { scroll: true, render: true });
  });
  svgText(x, y + 39, id, "vessel-label");
}

function locationPositions(ids) {
  const positions = {};
  const centerX = 450;
  const centerY = 260;
  const radius = Math.min(310, 90 + ids.length * 28);
  ids.forEach((id, index) => {
    const angle = -Math.PI / 2 + (Math.PI * 2 * index) / Math.max(ids.length, 1);
    positions[id] = {
      x: centerX + Math.cos(angle) * radius,
      y: centerY + Math.sin(angle) * radius,
    };
  });
  return positions;
}

function transitProgress(tick, vessel) {
  if (!state.travelTimes || !state.travelTimes[vessel.from]) {
    return 0.5;
  }
  const travelTime = state.travelTimes[vessel.from][vessel.to];
  if (!travelTime) {
    return 1;
  }
  const departed = vessel.arrives_at_tick - travelTime;
  return Math.max(0, Math.min(1, (tick - departed) / travelTime));
}

function renderVessels(world) {
  if (!world) {
    els.vesselsView.innerHTML = `<p class="empty">No vessels yet.</p>`;
    return;
  }

  const cards = sortedEntries(world.vessels).map(([id, vessel]) => {
    const committed = comparisonBaseline()?.vessels?.[id] || null;
    const changed = committed && JSON.stringify(committed) !== JSON.stringify(vessel);
    const status = changed ? `${committed.state} -> ${vessel.state}` : vessel.state;
    const selected = state.selectedVessel === id;
    return `
      <article class="entity-card${selected ? " selected" : ""}" data-vessel="${escapeHtml(id)}">
        <div class="entity-title">
          <button type="button" data-select-vessel="${escapeHtml(id)}">${escapeHtml(id)}</button>
          ${chip(status, changed ? "planned" : "committed")}
        </div>
        ${selected ? vesselDetailsHtml(vessel, committed) : ""}
      </article>
    `;
  });

  els.vesselsView.innerHTML = `<div class="entity-list">${cards.join("")}</div>`;

  for (const button of els.vesselsView.querySelectorAll("[data-select-vessel]")) {
    button.addEventListener("click", () => selectVessel(button.dataset.selectVessel, { render: true }));
  }
  for (const card of els.vesselsView.querySelectorAll("[data-vessel]")) {
    card.addEventListener("click", (event) => {
      if (!event.target.closest("button")) {
        selectVessel(card.dataset.vessel, { render: true });
      }
    });
  }
}

function renderDocks(world) {
  if (!world) {
    els.docksView.innerHTML = `<p class="empty">No docks yet.</p>`;
    return;
  }

  const groups = dockGroups(world);
  const cards = sortedEntries(world.docks).map(([id, dock]) => {
    const committed = comparisonBaseline()?.docks?.[id] || null;
    const changed = committed && JSON.stringify(committed) !== JSON.stringify(dock);
    const group = groups[id] || { docked: [], idle: [], inbound: [], outbound: [] };
    const selected = state.selectedDock === id;
    return `
      <article class="entity-card${selected ? " selected" : ""}" data-dock="${escapeHtml(id)}">
        <div class="entity-title">
          <button type="button" data-select-dock="${escapeHtml(id)}">Dock ${escapeHtml(id)}</button>
          ${chip(`${cargoTotal(dock.cargo)} cargo`, changed ? "planned" : "committed")}
        </div>
        ${selected ? dockDetailsHtml(dock, committed, group) : ""}
      </article>
    `;
  });

  els.docksView.innerHTML = `<div class="entity-list">${cards.join("")}</div>`;

  for (const button of els.docksView.querySelectorAll("[data-select-dock]")) {
    button.addEventListener("click", () => selectDock(button.dataset.selectDock, { render: true }));
  }
  for (const card of els.docksView.querySelectorAll("[data-dock]")) {
    card.addEventListener("click", (event) => {
      if (!event.target.closest("button")) {
        selectDock(card.dataset.dock, { render: true });
      }
    });
  }
}

function dockGroups(world) {
  const groups = {};
  for (const id of sortedKeys(world.docks)) {
    groups[id] = { docked: [], idle: [], inbound: [], outbound: [] };
  }

  for (const [id, vessel] of sortedEntries(world.vessels)) {
    if (vessel.state === "docked") {
      groups[vessel.location]?.docked.push(id);
    } else if (vessel.state === "idle") {
      groups[vessel.location]?.idle.push(id);
    } else if (vessel.state === "transit") {
      groups[vessel.to]?.inbound.push(id);
      groups[vessel.from]?.outbound.push(id);
    }
  }

  return groups;
}

function vesselDetailsHtml(vessel, committed) {
  const arrival = vessel.state === "transit" ? escapeHtml(vessel.arrives_at_tick) : "-";
  return `
    <div class="entity-details">
      <div class="detail-row"><span class="detail-label">Location / route</span> ${escapeHtml(vesselLocation(vessel, committed))}</div>
      <div class="detail-row"><span class="detail-label">Arrival tick</span> ${arrival}</div>
      <div class="detail-row"><span class="detail-label">Cargo</span> ${cargoHtml(vessel.cargo, committed?.cargo)}</div>
    </div>
  `;
}

function dockDetailsHtml(dock, committed, group) {
  return `
    <div class="entity-details">
      <div class="detail-row"><span class="detail-label">Cargo</span> ${cargoHtml(dock.cargo, committed?.cargo)}</div>
      <div class="detail-row"><span class="detail-label">Docked</span> ${listHtml(group.docked)}</div>
      <div class="detail-row"><span class="detail-label">Idle</span> ${listHtml(group.idle)}</div>
      <div class="detail-row"><span class="detail-label">Incoming</span> ${listHtml(group.inbound)}</div>
      <div class="detail-row"><span class="detail-label">Outgoing</span> ${listHtml(group.outbound)}</div>
    </div>
  `;
}

function renderLog() {
  els.responseLog.innerHTML = state.log
    .map((entry) => {
      const status = entry.response.status;
      const message = status === "ok" ? `tick ${entry.response.payload?.tick ?? "-"}` : entry.response.payload?.message;
      return `
        <li>
          <span class="${status === "ok" ? "ok" : "error"}">${escapeHtml(status)}</span>
          <span>${escapeHtml(String(message || ""))}</span>
          <code>${escapeHtml(JSON.stringify(entry.command))}</code>
        </li>
      `;
    })
    .join("");
}

function updateGeneratedCommand() {
  let command;
  const selected = els.commandSelect.value;
  const vesselId = els.vesselId.value || state.selectedVessel || "v1";
  const destination = numberValue(els.destination);
  const amount = numberValue(els.amount) || 1;

  if (selected === "init") {
    command = initCommand();
  } else if (selected === "tick") {
    command = { command: "tick", parameters: tickParameters() };
  } else if (selected === "set_destination") {
    command = { command: selected, parameters: { vessel_id: vesselId, destination } };
  } else if (selected === "dock" || selected === "undock") {
    command = { command: selected, parameters: { vessel_id: vesselId } };
  } else if (selected === "load" || selected === "unload") {
    command = { command: selected, parameters: { vessel_id: vesselId, destination, amount } };
  } else if (selected === "swap") {
    command = {
      command: "swap",
      parameters: {
        vessel_id: vesselId,
        load_destination: numberValue(els.loadDestination),
        unload_destination: numberValue(els.unloadDestination),
        amount,
      },
    };
  } else if (selected === "get_state_at") {
    command = { command: "get_state_at", parameters: { tick: Number(els.builderTick.value || 0) } };
  } else {
    command = { command: "get_state", parameters: {} };
  }

  els.generatedCommand.value = JSON.stringify(command, null, 2);
}

function initCommand() {
  const parsed = parseJsonQuiet(els.initInput.value) || {};
  if (parsed.command === "init" && parsed.parameters) {
    return parsed;
  }
  return { command: "init", parameters: parsed };
}

function updateCommandFields() {
  const selected = els.commandSelect.value;
  for (const field of els.commandFields) {
    const allowed = field.dataset.fields.split(/\s+/);
    field.classList.toggle("hidden", !allowed.includes(selected));
  }
}

function tickParameters() {
  const quantity = numberValue(els.tickManifestQuantity);
  if (!quantity) {
    return {};
  }

  return {
    cargo_manifest: [
      {
        location: numberValue(els.tickManifestLocation),
        destination: numberValue(els.tickManifestDestination),
        quantity,
      },
    ],
  };
}

function comparisonBaseline() {
  if (state.mode !== "planned") {
    return null;
  }
  if (!state.committed || !state.livePlanned || state.committed.tick !== state.livePlanned.tick) {
    return null;
  }
  return state.committed;
}

function latestTick() {
  return state.committed?.tick ?? state.livePlanned?.tick ?? 0;
}

function cycleSideSelection(direction) {
  const world = visibleState();
  if (!world) {
    return;
  }

  if (state.sideView === "vessels") {
    const ids = sortedKeys(world.vessels);
    if (!ids.length) {
      return;
    }
    const index = ids.indexOf(state.selectedVessel);
    const current = index >= 0 ? index : direction > 0 ? -1 : 0;
    const next = ids[(current + direction + ids.length) % ids.length];
    selectVessel(next, { render: true });
    scrollSelectedIntoView(`[data-vessel="${cssEscape(next)}"]`);
  } else {
    const ids = sortedKeys(world.docks);
    if (!ids.length) {
      return;
    }
    const index = ids.indexOf(state.selectedDock);
    const current = index >= 0 ? index : direction > 0 ? -1 : 0;
    const next = ids[(current + direction + ids.length) % ids.length];
    selectDock(next, { render: true });
    scrollSelectedIntoView(`[data-dock="${cssEscape(next)}"]`);
  }
}

function scrollSelectedIntoView(selector) {
  document.querySelector(selector)?.scrollIntoView({ block: "nearest" });
}

function cssEscape(value) {
  if (window.CSS?.escape) {
    return CSS.escape(value);
  }
  return String(value).replaceAll('"', '\\"');
}

function applyMapViewBox() {
  els.map.setAttribute(
    "viewBox",
    `${state.mapView.x} ${state.mapView.y} ${state.mapView.width} ${state.mapView.height}`,
  );
}

function zoomMap(event) {
  const pointer = mapPointFromEvent(event);
  const factor = event.deltaY < 0 ? 0.88 : 1.14;
  const nextWidth = clamp(state.mapView.width * factor, 220, 1600);
  const nextHeight = nextWidth * (520 / 900);
  const ratioX = (pointer.x - state.mapView.x) / state.mapView.width;
  const ratioY = (pointer.y - state.mapView.y) / state.mapView.height;

  state.mapView = {
    x: pointer.x - ratioX * nextWidth,
    y: pointer.y - ratioY * nextHeight,
    width: nextWidth,
    height: nextHeight,
  };
  applyMapViewBox();
}

function mapPointFromEvent(event) {
  const rect = els.map.getBoundingClientRect();
  return {
    x: state.mapView.x + ((event.clientX - rect.left) / rect.width) * state.mapView.width,
    y: state.mapView.y + ((event.clientY - rect.top) / rect.height) * state.mapView.height,
  };
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function parseJsonQuiet(text) {
  try {
    return JSON.parse(text);
  } catch (_) {
    return null;
  }
}

function setSideView(view) {
  state.sideView = view;
  for (const current of document.querySelectorAll(".side-tab")) {
    current.classList.toggle("active", current.dataset.sideView === view);
  }
  document.querySelector("#vessels-view").classList.toggle("hidden", view !== "vessels");
  document.querySelector("#docks-view").classList.toggle("hidden", view !== "docks");
}

function selectVessel(id, options = {}) {
  state.selectedVessel = id;
  state.selectedDock = null;
  setSideView("vessels");
  els.vesselId.value = id;
  updateGeneratedCommand();
  if (options.render) {
    render();
  }
  if (options.scroll) {
    scrollSelectedIntoView(`[data-vessel="${cssEscape(id)}"]`);
  }
}

function selectDock(id, options = {}) {
  state.selectedDock = id;
  state.selectedVessel = null;
  setSideView("docks");
  els.destination.value = id;
  updateGeneratedCommand();
  if (options.render) {
    render();
  }
  if (options.scroll) {
    scrollSelectedIntoView(`[data-dock="${cssEscape(id)}"]`);
  }
}

function clearSelection() {
  if (!state.selectedVessel && !state.selectedDock) {
    return;
  }
  state.selectedVessel = null;
  state.selectedDock = null;
  render();
}

function vesselLocation(vessel, committed) {
  const current = vesselLocationSimple(vessel);
  if (!committed) {
    return current;
  }
  const before = vesselLocationSimple(committed);
  return before === current ? current : `${before} -> ${current}`;
}

function vesselLocationSimple(vessel) {
  if (vessel.state === "transit") {
    return `${vessel.from} -> ${vessel.to}`;
  }
  return String(vessel.location);
}

function cargoHtml(cargo, committedCargo) {
  const cargoMap = cargoToMap(cargo);
  const committedMap = committedCargo ? cargoToMap(committedCargo) : null;
  const destinations = new Set(Object.keys(cargoMap));
  if (committedMap) {
    for (const destination of Object.keys(committedMap)) {
      destinations.add(destination);
    }
  }

  if (destinations.size === 0) {
    return `<span class="empty">empty</span>`;
  }

  const items = [...destinations].sort(numberSort).map((destination) => {
    const value = cargoMap[destination] || 0;
    const committed = committedMap?.[destination];
    if (committedMap && committed !== value) {
      return `<span class="chip planned">to ${escapeHtml(destination)}: <span class="delta">${committed || 0} -> ${value}</span></span>`;
    }
    return `<span class="chip committed">to ${escapeHtml(destination)}: ${value}</span>`;
  });

  return `<div class="cargo-list">${items.join("")}</div>`;
}

function cargoToMap(cargo) {
  const map = {};
  for (const entry of cargo || []) {
    map[String(entry.destination)] = entry.quantity;
  }
  return map;
}

function cargoTotal(cargo) {
  return (cargo || []).reduce((total, entry) => total + entry.quantity, 0);
}

function listHtml(items) {
  if (!items.length) {
    return `<span class="empty">none</span>`;
  }
  return `<div class="mini-list">${items.map((item) => `<span class="chip committed">${escapeHtml(item)}</span>`).join("")}</div>`;
}

function chip(text, kind) {
  return `<span class="chip ${kind}">${escapeHtml(text)}</span>`;
}

function sortedEntries(object) {
  return Object.entries(object || {}).sort(([a], [b]) => numberSort(a, b));
}

function sortedKeys(object) {
  return Object.keys(object || {}).sort(numberSort);
}

function numberSort(a, b) {
  const left = Number(a);
  const right = Number(b);
  if (Number.isFinite(left) && Number.isFinite(right)) {
    return left - right;
  }
  return String(a).localeCompare(String(b));
}

function numberValue(input) {
  return Number(input.value || 0);
}

function routeKey(from, to) {
  return `${from}:${to}`;
}

function laneRouteKey(from, to) {
  const left = Number(from);
  const right = Number(to);
  return left <= right ? `${from}:${to}` : `${to}:${from}`;
}

function svgCircle(cx, cy, r, className, title) {
  const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  circle.setAttribute("cx", cx);
  circle.setAttribute("cy", cy);
  circle.setAttribute("r", r);
  circle.setAttribute("class", className);
  if (title) {
    const titleElement = document.createElementNS("http://www.w3.org/2000/svg", "title");
    titleElement.textContent = title;
    circle.appendChild(titleElement);
  }
  els.map.appendChild(circle);
  return circle;
}

function svgShip(x, y, className, title, angle = 0) {
  const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
  group.setAttribute("class", `${className} vessel-ship-hit`);
  group.setAttribute("transform", `translate(${x} ${y}) rotate(${angle})`);

  const hull = document.createElementNS("http://www.w3.org/2000/svg", "path");
  hull.setAttribute("d", "M 0 -23 L 9 -11 L 9 18 L 6 23 L -6 23 L -9 18 L -9 -11 Z");
  hull.setAttribute("class", "ship-hull");
  group.appendChild(hull);

  for (const y of [-8, 0, 8]) {
    const bay = document.createElementNS("http://www.w3.org/2000/svg", "rect");
    bay.setAttribute("x", "-6");
    bay.setAttribute("y", String(y));
    bay.setAttribute("width", "12");
    bay.setAttribute("height", "6");
    bay.setAttribute("rx", "1");
    bay.setAttribute("class", "ship-cargo-bay");
    group.appendChild(bay);
  }

  const bridge = document.createElementNS("http://www.w3.org/2000/svg", "rect");
  bridge.setAttribute("x", "-7");
  bridge.setAttribute("y", "16");
  bridge.setAttribute("width", "14");
  bridge.setAttribute("height", "6");
  bridge.setAttribute("rx", "2");
  bridge.setAttribute("class", "ship-bridge");
  group.appendChild(bridge);

  for (const xOffset of [-5, 5]) {
    const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
    line.setAttribute("x1", String(xOffset));
    line.setAttribute("y1", "-13");
    line.setAttribute("x2", String(xOffset));
    line.setAttribute("y2", "13");
    line.setAttribute("class", "ship-deck-line");
    group.appendChild(line);
  }

  if (title) {
    const titleElement = document.createElementNS("http://www.w3.org/2000/svg", "title");
    titleElement.textContent = title;
    group.appendChild(titleElement);
  }

  els.map.appendChild(group);
  return group;
}

function svgLine(x1, y1, x2, y2, className) {
  const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
  line.setAttribute("x1", x1);
  line.setAttribute("y1", y1);
  line.setAttribute("x2", x2);
  line.setAttribute("y2", y2);
  line.setAttribute("class", className);
  els.map.appendChild(line);
}

function svgText(x, y, text, className) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", "text");
  node.setAttribute("x", x);
  node.setAttribute("y", y);
  node.setAttribute("class", className);
  node.textContent = text;
  els.map.appendChild(node);
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
