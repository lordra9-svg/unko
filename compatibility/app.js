const REPO = "https://github.com/lordra9-svg/unko";
const CATALOG_URL = "games.json";
const ACCURACY_URLS = ["accuracy.jsonl", "accuracy-ares.jsonl", "accuracy-snes.jsonl"];
const PAGE_SIZE = 200;

const state = {
  query: "",
  status: "all",
  open: true,
  visible: {},
  data: null,
};

const el = {
  summary: document.querySelector("#summary"),
  systems: document.querySelector("#systems"),
  search: document.querySelector("#search"),
  status: document.querySelector("#status-filter"),
  expand: document.querySelector("#expand"),
  count: document.querySelector("#count"),
  legend: document.querySelector("#status-legend"),
};

function statusById(id) {
  return state.data.statuses.find((status) => status.id === id) ?? {
    id,
    label: id,
    description: "",
  };
}

function reportUrl(system, game) {
  const region = game.region?.split(" / ")[0] ?? "";
  const suffix = region && !game.title.includes(region) ? ` (${game.region})` : "";
  const title = `[${system.short}] ${game.title}${suffix}`;
  const labels = ["compatibility", `system:${system.id}`];
  return (
    `${REPO}/issues/new?template=compatibility-report.yml` +
    `&title=${encodeURIComponent(title)}` +
    `&labels=${encodeURIComponent(labels.join(","))}`
  );
}

function matches(game) {
  if (state.status !== "all" && game.status !== state.status) {
    return false;
  }
  if (!state.query) {
    return true;
  }
  const haystack = [
    game.title,
    game.format,
    game.region,
    game.revision,
    game.mapping,
    game.sha256,
    game.evidence,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(state.query);
}

function addCell(row, text, className) {
  const cell = document.createElement("td");
  if (className) {
    cell.className = className;
  }
  cell.textContent = text;
  row.append(cell);
  return cell;
}

function renderSummary(games) {
  const verified = games.filter((game) => game.status === "verified").length;
  const statuses = new Set(games.map((game) => game.status));
  el.summary.replaceChildren();
  for (const [value, label] of [
    [String(games.length), "games tracked"],
    [String(state.data.systems.length), "systems"],
    [String(verified), "verified"],
    [String(statuses.size), "states in use"],
    [state.data.updated || "—", "last updated"],
  ]) {
    const item = document.createElement("span");
    const strong = document.createElement("strong");
    strong.textContent = value;
    item.append(strong, document.createTextNode(label));
    el.summary.append(item);
  }
}

function renderLegend() {
  el.legend.replaceChildren();
  for (const status of state.data.statuses) {
    const term = document.createElement("dt");
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.dataset.status = status.id;
    badge.textContent = status.label;
    term.append(badge);
    const description = document.createElement("dd");
    description.textContent = status.description;
    el.legend.append(term, description);
  }
}

function renderTable(system, games) {
  const wrap = document.createElement("div");
  wrap.className = "table-wrap";
  const table = document.createElement("table");
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  for (const label of ["Game", "Format", "Region", "Mapping", "Status", "Accuracy", "Evidence", ""]) {
    const th = document.createElement("th");
    th.scope = "col";
    th.textContent = label;
    headRow.append(th);
  }
  thead.append(headRow);
  table.append(thead);

  const tbody = document.createElement("tbody");
  for (const game of games) {
    const row = document.createElement("tr");

    const gameCell = document.createElement("td");
    gameCell.className = "game";
    const title = document.createElement("strong");
    title.textContent = game.title;
    gameCell.append(title);
    const detail = document.createElement("small");
    detail.textContent = [game.revision, game.sha256 ? `sha256:${game.sha256.slice(0, 16)}…` : ""]
      .filter(Boolean)
      .join(" · ");
    if (detail.textContent) {
      gameCell.append(detail);
    }
    row.append(gameCell);

    addCell(row, game.format || "—");
    addCell(row, game.region || "—");
    addCell(row, game.mapping || "—");

    const statusCell = document.createElement("td");
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.dataset.status = game.status;
    badge.textContent = statusById(game.status).label;
    statusCell.append(badge);
    row.append(statusCell);

    const accuracyCell = document.createElement("td");
    const records = game.accuracy ? Object.entries(game.accuracy) : [];
    if (records.length === 0) {
      accuracyCell.className = "muted";
      accuracyCell.textContent = "—";
    } else {
      const labels = { rustynes: "R", ares: "A", unknown: "?" };
      let shown = false;
      for (const [oracle, record] of records) {
        if (record.status !== "ok") continue;
        const line = document.createElement("div");
        const value = document.createElement("strong");
        const detail = document.createElement("small");
        if (record.geometry_pct != null) {
          const match = record.best_geometry_pct ?? record.geometry_pct;
          value.textContent = `${labels[oracle] ?? oracle} ${match.toFixed(2)}%`;
          value.title = `${oracle}: palette-independent geometry ${match.toFixed(2)}% (shift ${record.best_shift ?? 0})`;
          detail.textContent = record.steady_geometry_pct == null
            ? "video"
            : `steady ${record.steady_geometry_pct.toFixed(2)}%`;
        } else if (record.video_pct != null) {
          value.textContent = `${labels[oracle] ?? oracle} ${record.video_pct.toFixed(2)}%`;
          value.title = `${oracle}: final-frame pixel match`;
          detail.textContent = record.audio_pct == null
            ? "video"
            : `audio ${record.audio_pct.toFixed(2)}%`;
        } else {
          continue;
        }
        line.append(value, detail);
        accuracyCell.append(line);
        shown = true;
      }
      if (!shown) {
        accuracyCell.className = "muted";
        accuracyCell.textContent = "error";
      } else {
        accuracyCell.className = "accuracy";
      }
    }
    row.append(accuracyCell);

    addCell(row, game.evidence || "—", "muted");

    const actions = document.createElement("td");
    const test = document.createElement("a");
    test.href = reportUrl(system, game);
    test.textContent = game.status === "untested" ? "Test" : "Report";
    actions.append(test);
    if (game.issue) {
      const issue = document.createElement("a");
      issue.href = `${REPO}/issues/${game.issue}`;
      issue.textContent = `Issue #${game.issue}`;
      issue.style.marginLeft = "10px";
      actions.append(issue);
    }
    row.append(actions);

    tbody.append(row);
  }
  table.append(tbody);
  wrap.append(table);
  return wrap;
}

function renderSystems() {
  const games = state.data.games.filter(matches);
  el.systems.replaceChildren();
  el.count.textContent = `${games.length} of ${state.data.games.length} games`;

  for (const system of state.data.systems) {
    const systemGames = games.filter((game) => game.system === system.id);
    if ((state.query || state.status !== "all") && systemGames.length === 0) {
      continue;
    }
    const details = document.createElement("details");
    details.className = "system";
    details.open = state.open || Boolean(state.query) || state.status !== "all";

    const summary = document.createElement("summary");
    const title = document.createElement("h2");
    title.textContent = system.name;
    const count = document.createElement("span");
    count.className = "system-count";
    count.textContent = `${systemGames.length} ${systemGames.length === 1 ? "game" : "games"}`;
    const chevron = document.createElement("span");
    chevron.className = "chevron";
    chevron.textContent = "▶";
    summary.append(title, count, chevron);
    details.append(summary);

    const body = document.createElement("div");
    body.className = "system-body";
    if (system.note) {
      const note = document.createElement("p");
      note.className = "system-note";
      note.textContent = system.note;
      body.append(note);
    }
    if (system.test) {
      const command = document.createElement("div");
      command.className = "test-command";
      const code = document.createElement("code");
      code.textContent = system.test;
      const copy = document.createElement("button");
      copy.type = "button";
      copy.textContent = "Copy";
      copy.addEventListener("click", async () => {
        try {
          await navigator.clipboard.writeText(system.test);
          copy.textContent = "Copied";
          setTimeout(() => (copy.textContent = "Copy"), 1200);
        } catch {
          copy.textContent = "Select command";
        }
      });
      command.append(code, copy);
      body.append(command);
    }

    if (systemGames.length === 0) {
      const empty = document.createElement("div");
      empty.className = "empty";
      empty.textContent =
        "No per-game reports yet. Use “Report a game” to add the first one, or run the test command above.";
      body.append(empty);
    } else {
      const visible = state.visible[system.id] ?? PAGE_SIZE;
      body.append(renderTable(system, systemGames.slice(0, visible)));
      if (systemGames.length > visible) {
        const more = document.createElement("button");
        more.type = "button";
        more.className = "button";
        more.style.marginTop = "12px";
        more.textContent = `Show next ${PAGE_SIZE} (${systemGames.length - visible} remaining)`;
        more.addEventListener("click", () => {
          state.visible[system.id] = visible + PAGE_SIZE;
          render();
        });
        body.append(more);
      }
    }
    details.append(body);
    el.systems.append(details);
  }

  if (el.systems.childElementCount === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No games match the current filters.";
    el.systems.append(empty);
  }
}

function render() {
  const games = state.data.games.filter(matches);
  renderSummary(state.data.games);
  renderSystems(games);
}

async function loadAccuracy() {
  const map = new Map();
  for (const url of ACCURACY_URLS) {
    try {
      const response = await fetch(url, { cache: "no-store" });
      if (!response.ok) continue;
      const text = await response.text();
      for (const line of text.split("\n")) {
        if (!line.trim()) continue;
        try {
          const row = JSON.parse(line);
          if (!row.sha256) continue;
          const byOracle = map.get(row.sha256) ?? {};
          byOracle[row.oracle || "unknown"] = row;
          map.set(row.sha256, byOracle);
        } catch {
          // A truncated last line during a live run is not a page error.
        }
      }
    } catch {
      // An oracle ledger is optional until that stage starts.
    }
  }
  return map;
}

async function boot() {
  try {
    const [response, accuracy] = await Promise.all([
      fetch(CATALOG_URL, { cache: "no-store" }),
      loadAccuracy(),
    ]);
    if (!response.ok) {
      throw new Error(`${response.status} ${response.statusText}`);
    }
    state.data = await response.json();
    for (const game of state.data.games) {
      const record = accuracy.get(game.sha256);
      if (record) game.accuracy = record;
    }
  } catch (error) {
    const failure = document.createElement("div");
    failure.className = "error";
    failure.textContent =
      `Could not load ${CATALOG_URL}: ${error.message}. ` +
      "Serve this directory over HTTP (the GitHub Pages workflow does this automatically).";
    el.systems.append(failure);
    return;
  }

  state.data.games.sort((a, b) => a.title.localeCompare(b.title, undefined, { numeric: true }));
  for (const status of state.data.statuses) {
    const option = document.createElement("option");
    option.value = status.id;
    option.textContent = status.label;
    el.status.append(option);
  }
  renderLegend();
  render();

  el.search.addEventListener("input", () => {
    state.query = el.search.value.trim().toLowerCase();
    state.visible = {};
    render();
  });
  el.status.addEventListener("change", () => {
    state.status = el.status.value;
    state.visible = {};
    render();
  });
  el.expand.addEventListener("click", () => {
    state.open = !state.open;
    el.expand.textContent = state.open ? "Collapse all" : "Expand all";
    for (const details of el.systems.querySelectorAll("details.system")) {
      details.open = state.open;
    }
  });
}

boot();
