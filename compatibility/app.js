const REPO = "https://github.com/lordra9-svg/unko";
const CATALOG_URL = "games.json";
const ACCURACY_URLS = ["accuracy.jsonl", "accuracy-ares.jsonl", "accuracy-snes.jsonl"];
const PAGE_SIZE = 200;

const state = {
  query: "",
  status: "all",
  open: true,
  visible: {},
  sort: { key: "title", dir: 1 },
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
  dialog: document.querySelector("#game-dialog"),
  detailTitle: document.querySelector("#detail-title"),
  detailSystem: document.querySelector("#detail-system"),
  detailBody: document.querySelector("#detail-body"),
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

function systemById(id) {
  return state.data.systems.find((system) => system.id === id);
}

function oracleLabel(oracle) {
  return {
    rustynes: "RustyNES",
    ares: "ares",
    unknown: "Unknown oracle",
  }[oracle] ?? oracle;
}

function addMetaRow(list, label, value) {
  const term = document.createElement("dt");
  term.textContent = label;
  const detail = document.createElement("dd");
  if (value instanceof Node) {
    detail.append(value);
  } else {
    detail.textContent = value;
  }
  list.append(term, detail);
}

function accuracyMetrics(record) {
  const rows = [];
  if (record.geometry_pct != null) {
    if (record.best_geometry_pct != null) {
      rows.push(["Aligned geometry", `${record.best_geometry_pct.toFixed(2)}%`]);
    }
    rows.push(["Same-index geometry", `${record.geometry_pct.toFixed(2)}%`]);
    if (record.steady_geometry_pct != null) {
      rows.push(["Steady geometry (frame 30+)", `${record.steady_geometry_pct.toFixed(2)}%`]);
    }
    rows.push(["Alignment", `${record.best_shift ?? 0} frame${Math.abs(record.best_shift ?? 0) === 1 ? "" : "s"}`]);
    rows.push(["Audio", "not scored (oracle stream is DC-blocked)"]);
  } else if (record.video_pct != null) {
    rows.push(["Video (final frame)", `${record.video_pct.toFixed(2)}%`]);
    if (record.audio_pct != null) {
      rows.push(["Audio (raw S-DSP PCM)", `${record.audio_pct.toFixed(2)}%`]);
    }
    if (record.region) {
      rows.push(["Timing region", record.region]);
    }
  }
  if (record.frames != null) {
    rows.push(["Frames", String(record.frames)]);
  }
  if (record.tested) {
    rows.push(["Tested", record.tested]);
  }
  return rows;
}

function showGameDetail(system, game) {
  el.detailTitle.textContent = game.title;
  el.detailSystem.textContent = [system.name, game.format].filter(Boolean).join(" · ");
  el.detailBody.replaceChildren();

  const status = statusById(game.status);
  const statusLine = document.createElement("div");
  statusLine.className = "detail-status";
  const badge = document.createElement("span");
  badge.className = "badge";
  badge.dataset.status = game.status;
  badge.textContent = status.label;
  const statusText = document.createElement("p");
  statusText.textContent = status.description || "No status note.";
  statusLine.append(badge, statusText);
  el.detailBody.append(statusLine);

  const meta = document.createElement("dl");
  meta.className = "detail-meta";
  addMetaRow(meta, "System", system.name);
  addMetaRow(meta, "Format", game.format || "—");
  addMetaRow(meta, "Region", game.region || "—");
  if (game.revision) addMetaRow(meta, "Revision", game.revision);
  if (game.mapping) addMetaRow(meta, "Mapping", game.mapping);
  if (game.tested) addMetaRow(meta, "Status tested", game.tested);
  if (game.sha256) {
    const code = document.createElement("code");
    code.textContent = game.sha256;
    addMetaRow(meta, "SHA-256", code);
  }
  el.detailBody.append(meta);

  const reported = document.createElement("section");
  reported.className = "detail-section";
  const heading = document.createElement("h3");
  heading.textContent = "What has been reported";
  reported.append(heading);

  const evidence = document.createElement("p");
  evidence.textContent = game.evidence || "No status evidence recorded.";
  reported.append(evidence);

  const records = game.accuracy ? Object.entries(game.accuracy) : [];
  if (records.length === 0) {
    const none = document.createElement("p");
    none.className = "muted";
    none.textContent = "No oracle accuracy report has been recorded for this entry yet.";
    reported.append(none);
  } else {
    for (const [oracle, record] of records) {
      const card = document.createElement("div");
      card.className = "report-card";
      const head = document.createElement("div");
      head.className = "report-head";
      const name = document.createElement("strong");
      name.textContent = `${oracleLabel(oracle)} report`;
      const stateLabel = document.createElement("span");
      stateLabel.className = "report-status";
      stateLabel.textContent = record.status === "ok" ? "measured" : "error";
      if (record.status !== "ok") stateLabel.classList.add("error");
      head.append(name, stateLabel);
      card.append(head);

      if (record.status !== "ok") {
        const error = document.createElement("p");
        error.className = "report-error";
        error.textContent = record.error || "The oracle run failed.";
        card.append(error);
      } else {
        const metrics = document.createElement("dl");
        metrics.className = "report-metrics";
        for (const [label, value] of accuracyMetrics(record)) {
          addMetaRow(metrics, label, value);
        }
        card.append(metrics);
        if (record.note) {
          const note = document.createElement("p");
          note.className = "report-note";
          note.textContent = record.note;
          card.append(note);
        }
      }
      reported.append(card);
    }
  }
  el.detailBody.append(reported);

  const actions = document.createElement("div");
  actions.className = "detail-actions";
  const report = document.createElement("a");
  report.className = "button primary";
  report.href = reportUrl(system, game);
  report.textContent = "Report a result";
  const search = document.createElement("a");
  search.className = "button";
  search.href = `${REPO}/issues?q=${encodeURIComponent(`is:issue "${game.title}"`)}`;
  search.textContent = "Search GitHub reports";
  actions.append(report, search);
  if (game.issue) {
    const issue = document.createElement("a");
    issue.className = "button";
    issue.href = `${REPO}/issues/${game.issue}`;
    issue.textContent = `Issue #${game.issue}`;
    actions.append(issue);
  }
  el.detailBody.append(actions);

  if (system.test) {
    const command = document.createElement("div");
    command.className = "detail-command";
    const code = document.createElement("code");
    code.textContent = system.test;
    command.append(code);
    el.detailBody.append(command);
  }

  el.dialog.showModal();
}

function accuracyScore(game) {
  const records = game.accuracy ? Object.values(game.accuracy) : [];
  let best = -1;
  for (const record of records) {
    if (record.status !== "ok") continue;
    const value = record.best_geometry_pct ?? record.geometry_pct ?? record.video_pct;
    if (typeof value === "number" && value > best) best = value;
  }
  return best;
}

function compareText(a, b) {
  if (!a && b) return 1;
  if (a && !b) return -1;
  return String(a || "").localeCompare(String(b || ""), undefined, { numeric: true, sensitivity: "base" });
}

function compareGames(a, b, key) {
  let result = 0;
  if (key === "title") {
    result = compareText(a.title, b.title);
  } else if (key === "format") {
    result = compareText(a.format, b.format);
  } else if (key === "region") {
    result = compareText(a.region, b.region);
  } else if (key === "status") {
    const order = new Map(state.data.statuses.map((status, index) => [status.id, index]));
    result = (order.get(a.status) ?? 999) - (order.get(b.status) ?? 999);
  } else if (key === "accuracy") {
    result = accuracyScore(a) - accuracyScore(b);
  }
  if (result === 0) result = compareText(a.title, b.title);
  return result;
}

function sortGames(games) {
  return [...games].sort((a, b) => compareGames(a, b, state.sort.key) * state.sort.dir);
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
  const columns = [
    ["Game", "title"],
    ["Format", "format"],
    ["Region", "region"],
    ["Mapping", null],
    ["Status", "status"],
    ["Accuracy", "accuracy"],
    ["Evidence", null],
    ["", null],
  ];
  for (const [label, key] of columns) {
    const th = document.createElement("th");
    th.scope = "col";
    if (!key) {
      th.textContent = label;
    } else {
      const active = state.sort.key === key;
      th.setAttribute("aria-sort", active ? (state.sort.dir > 0 ? "ascending" : "descending") : "none");
      const button = document.createElement("button");
      button.type = "button";
      button.className = "sort-button";
      if (active) button.classList.add("active");
      button.textContent = label;
      button.title = `Sort by ${label.toLowerCase()}`;
      const arrow = document.createElement("span");
      arrow.className = "sort-arrow";
      arrow.textContent = active ? (state.sort.dir > 0 ? "▲" : "▼") : "↕";
      button.append(arrow);
      button.addEventListener("click", () => {
        if (state.sort.key === key) {
          state.sort.dir *= -1;
        } else {
          state.sort = { key, dir: key === "accuracy" ? -1 : 1 };
        }
        state.visible = {};
        render();
      });
      th.append(button);
    }
    headRow.append(th);
  }
  thead.append(headRow);
  table.append(thead);

  const tbody = document.createElement("tbody");
  for (const game of games) {
    const row = document.createElement("tr");
    row.className = "game-row";
    row.addEventListener("click", (event) => {
      if (event.target.closest("a,button")) return;
      showGameDetail(system, game);
    });

    const gameCell = document.createElement("td");
    gameCell.className = "game";
    const title = document.createElement("button");
    title.type = "button";
    title.className = "game-link";
    title.textContent = game.title;
    title.setAttribute("aria-label", `View details for ${game.title}`);
    title.addEventListener("click", () => showGameDetail(system, game));
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
      const ordered = sortGames(systemGames);
      const visible = state.visible[system.id] ?? PAGE_SIZE;
      body.append(renderTable(system, ordered.slice(0, visible)));
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
  el.dialog.addEventListener("click", (event) => {
    if (event.target === el.dialog) el.dialog.close();
  });
}

boot();
