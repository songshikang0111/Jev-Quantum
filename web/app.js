(function () {
  const fileInput = document.getElementById("file");
  const dropzone = document.getElementById("dropzone");
  const statusEl = document.getElementById("file-status");
  const playBtn = document.getElementById("play");
  const resetBtn = document.getElementById("reset");
  const endBtn = document.getElementById("end");
  const scaleInput = document.getElementById("view-scale");
  const scaleLabel = document.getElementById("scale-label");
  const columnsInput = document.getElementById("columns");
  const captureInput = document.getElementById("capture");
  const fitBtn = document.getElementById("fit");
  const fullscreenBtn = document.getElementById("fullscreen");
  const viewport = document.getElementById("stage-viewport");
  const stage = document.getElementById("stage");
  const SPEEDS = [0.1, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64];
  const BASE_STEP_MS = 250;
  let fitEnabled = false;
  let autoColumns = null;
  let layoutFrame = 0;
  const speed = document.getElementById("speed");
  const speedLabel = document.getElementById("speed-label");
  const realtime = document.getElementById("realtime");
  const compareEl = document.getElementById("compare");
  const overviewEl = document.getElementById("overview");
  const logEl = document.getElementById("log");

  const PANE_STYLES = [
    { heat: "57,255,136", wall: "#2d6a49", cursor: "#ffffff", start: "#f3c15b", exit: "#39ff88" },
    { heat: "243,193,91", wall: "#6a542d", cursor: "#fff4cc", start: "#f3c15b", exit: "#39ff88" },
    { heat: "91,192,255", wall: "#315d80", cursor: "#e0f5ff", start: "#f3c15b", exit: "#5bc0ff" },
    { heat: "199,143,255", wall: "#684887", cursor: "#f4e5ff", start: "#f3c15b", exit: "#c78fff" },
    { heat: "255,133,112", wall: "#88483c", cursor: "#fff0e9", start: "#f3c15b", exit: "#ff8570" },
    { heat: "84,230,219", wall: "#327d76", cursor: "#e0ffff", start: "#f3c15b", exit: "#54e6db" },
    { heat: "255,150,213", wall: "#864568", cursor: "#fff0fa", start: "#f3c15b", exit: "#ff96d5" },
    { heat: "190,218,100", wall: "#687b32", cursor: "#f6ffd9", start: "#f3c15b", exit: "#beda64" },
    { heat: "175,190,255", wall: "#586590", cursor: "#f0f2ff", start: "#f3c15b", exit: "#afbeff" },
  ];

  let panes = [];
  const paneTemplate = document.getElementById("pane-0").cloneNode(true);
  function createPanes(targets) {
    compareEl.replaceChildren();
    panes = targets.map((target, i) => {
      const root = paneTemplate.cloneNode(true);
      root.id = "pane-" + i;
      root.querySelectorAll("[id]").forEach((el) => { el.id = el.id.replace(/-0$/, "-" + i); });
      compareEl.appendChild(root);
      const progressEl = document.createElement("p");
      progressEl.className = "playback-progress";
      progressEl.setAttribute("aria-label", target.name + " playback progress");
      const footer = document.createElement("div");
      footer.className = "pane-footer";
      const latencyEl = document.createElement("p");
      latencyEl.className = "playback-latency";
      const p50 = target.stats?.p50_ns;
      const scope = target.endpoint === "in-process" ? "本地计算" : target.endpoint === "subagent-session" ? "含工具调度" : "HTTP";
      latencyEl.textContent = "P50 " + (typeof p50 === "number" ? nsToMs(p50) + " ms" : "—");
      latencyEl.title = "全程响应时间中位数 · " + scope + "；不是回放速度";
      const scopeEl = document.createElement("small");
      scopeEl.textContent = scope;
      latencyEl.appendChild(scopeEl);
      footer.append(progressEl, latencyEl);
      root.appendChild(footer);
      const canvas = root.querySelector("canvas");
      const style = PANE_STYLES[i % PANE_STYLES.length];
      root.style.setProperty("--pane-accent", "rgb(" + style.heat + ")");
      return { index: i, root, canvas, ctx: canvas.getContext("2d"),
        nameEl: root.querySelector("h2"), titleEl: root.querySelector(".pane-kicker"),
        metaEl: root.querySelector(".pane-meta"), metricsEl: root.querySelector("dl"),
        style, progressEl, steps: decisionSteps(target), playIndex: 0 };
    });
  }

  let report = null;
  let playing = false;
  let timer = 0;

  function log(line) {
    logEl.textContent += line + "\n";
    logEl.scrollTop = logEl.scrollHeight;
  }

  function fail(message) {
    statusEl.textContent = message;
    statusEl.classList.add("error");
  }

  function ok(message) {
    statusEl.textContent = message;
    statusEl.classList.remove("error");
  }

  function validate(data) {
    if (!data || data.schema_version !== 1) {
      throw new Error("unsupported schema_version");
    }
    if (!data.maze || !Array.isArray(data.maze.cells)) {
      throw new Error("report has no maze definition");
    }
    if (!Array.isArray(data.targets) || data.targets.length === 0) {
      throw new Error("report has no targets");
    }
    return data;
  }

  function nsToMs(ns) {
    return (ns / 1e6).toFixed(3);
  }

  function decisionSteps(target) {
    const steps = (target.trajectory && target.trajectory.steps) || [];
    return steps.filter((step) => step.kind !== "pace_gap");
  }

  function stopReason(target) {
    return target.trajectory && target.trajectory.stop_reason
      ? target.trajectory.stop_reason
      : "n/a";
  }

  const STRATEGY_NOTES = {
    local: "Random legal move",
    jev: "Original local features",
    "flood-fill": "Online BFS planner",
    "memory-rules": "Spatial v2 rules in code · no model or BFS",
    "jev-memory-v1": "Recent trajectory + spatial memory",
    "jev-memory-no-distance": "Full history · proximity cues removed",
    "jev-memory-v1-free": "Full history · free strategy prompt only",
    "jev-memory-v1-long": "Original v1 rules · full trajectory · 30 min budget",
    "jev-memory-v2": "Spatial memory · least-traversed edge first",
    "jev-flood-fill": "Jev chooses using a local BFS distance field",
    "luna-session": "One continuing Luna session · local observations only",
  };

  function renderOverview() {
    overviewEl.replaceChildren();
    const table = document.createElement("table");
    const head = table.createTHead().insertRow();
    ["Strategy", "Result", "Decisions", "Explored", "Decision rule"].forEach((label) => {
      const th = document.createElement("th"); th.textContent = label; head.appendChild(th);
    });
    const body = table.createTBody();
    report.targets.forEach((target, i) => {
      const steps = decisionSteps(target);
      const explored = new Set([report.maze.start.join(","), ...steps.map((s) => s.x + "," + s.y)]).size;
      const values = [target.name, target.trajectory?.success ? "Exit reached" : stopReason(target), steps.length, explored + "/" + report.maze.cells.length, STRATEGY_NOTES[target.name] || ""];
      const row = body.insertRow();
      values.forEach((value, col) => {
        const cell = row.insertCell(); cell.textContent = value;
        if (col === 0) cell.style.color = "rgb(" + PANE_STYLES[i % PANE_STYLES.length].heat + ")";
      });
    });
    overviewEl.appendChild(table);
    overviewEl.hidden = false;
  }

  function renderPaneMeta(pane, target) {
    const steps = decisionSteps(target);
    const success = target.trajectory ? String(target.trajectory.success) : "n/a";
    pane.titleEl.textContent = target.name;
    pane.nameEl.textContent = target.model;
    pane.metaEl.textContent =
      (target.endpoint === "in-process" ? "In-process planning · " : target.endpoint === "subagent-session" ? "Stateful agent turns · " : "HTTP decisions · ") + steps.length + " decisions · stop=" + stopReason(target) + " · success=" + success;
    const description = document.createElement("p");
    description.className = "pane-meta strategy-description";
    description.textContent = STRATEGY_NOTES[target.name] || target.name;
    pane.metaEl.after(description);
    const s = target.stats;
    const audit = steps.map((step) => step.planning).filter(Boolean);
    const floodAudit = audit.filter((a) => typeof a.downhill === "boolean");
    const metrics = [
      ["ok", s.ok + "/" + s.requests],
      ["qps", s.qps.toFixed(1)],
      ["p50", nsToMs(s.p50_ns) + " ms"],
      ["p95", nsToMs(s.p95_ns) + " ms"],
      ["exit", target.trajectory && target.trajectory.exit_step != null
        ? String(target.trajectory.exit_step)
        : "—"],
      ["stop", stopReason(target)],
      ["explored", new Set([report.maze.start.join(","), ...steps.map((step) => step.x + "," + step.y)]).size + "/" + (report.maze.width * report.maze.height)],
      ["collisions", steps.filter((step) => step.collision).length],
    ];
    if (target.endpoint === "subagent-session") {
      const source = (report.config.sources || []).find((s) => s.config.targets === "luna-session");
      const session = target.session_metrics || (source ? source.config.session_metrics : report.config.session_metrics);
      if (session) {
        metrics.push(["decision turns", session.decision_turns ?? steps.length]);
        metrics.push(["game elapsed", (session.wall_ns / 1e9).toFixed(1) + " s"]);
        const note = document.createElement("p");
        note.className = "pane-meta";
        note.textContent = "Timing includes controller/tool delays. Agent active excludes parent dispatch gaps; this is not inference-only timing.";
        pane.metaEl.after(note);
        if (session.subagent_turns != null) metrics.push(["session turns", session.subagent_turns]);
        if (session.model_calls != null) metrics.push(["model calls", session.model_calls]);
        if (session.agent_active_wall_seconds != null) metrics.push(["agent active", session.agent_active_wall_seconds.toFixed(1) + " s"]);
      }
    }
    if (audit.length) metrics.push(["max request", (audit.reduce((max, a) => Math.max(max, a.request_bytes || 0), 0) / 1024).toFixed(1) + " KiB"]);
    if (floodAudit.length) metrics.push(["downhill", floodAudit.filter((a) => a.downhill).length + "/" + floodAudit.length]);
    pane.metricsEl.innerHTML = metrics
      .map(([k, v]) => "<dt>" + k + "</dt><dd>" + v + "</dd>")
      .join("");
  }

  function draw(pane, target, upto) {
    const maze = report.maze;
    const steps = pane.steps;
    pane.progressEl.textContent = "Step " + upto + " / " + steps.length +
      (upto === steps.length ? (target.trajectory?.success ? " · Exit reached" : " · " + stopReason(target)) : "");
    const w = maze.width;
    const h = maze.height;
    const pad = 18;
    const cell = Math.min((pane.canvas.width - pad * 2) / w, (pane.canvas.height - pad * 2) / h);
    const ctx = pane.ctx;
    const style = pane.style;
    ctx.fillStyle = "#030907";
    ctx.fillRect(0, 0, pane.canvas.width, pane.canvas.height);

    const visits = new Map();
    let cursor = { x: maze.start[0], y: maze.start[1] };
    for (let i = 0; i < upto && i < steps.length; i += 1) {
      const step = steps[i];
      const key = step.x + "," + step.y;
      visits.set(key, (visits.get(key) || 0) + 1);
      cursor = step;
    }

    for (let y = 0; y < h; y += 1) {
      for (let x = 0; x < w; x += 1) {
        const px = pad + x * cell;
        const py = pad + y * cell;
        const walls = maze.cells[y * w + x];
        const heat = visits.get(x + "," + y) || 0;
        if (heat) {
          const gutter = Math.max(3.5, Math.min(cell * 0.14, 9));
          const seam = 0.6;
          const left = px + (walls.w ? gutter : -seam);
          const top = py + (walls.n ? gutter : -seam);
          const right = px + cell + (walls.e ? -gutter : seam);
          const bottom = py + cell + (walls.s ? -gutter : seam);
          ctx.fillStyle = "rgba(" + style.heat + "," + Math.min(0.12 + heat / 40, 0.7) + ")";
          ctx.fillRect(left, top, right - left, bottom - top);
        }
        ctx.strokeStyle = style.wall;
        ctx.lineWidth = 2;
        ctx.beginPath();
        if (walls.n) { ctx.moveTo(px, py); ctx.lineTo(px + cell, py); }
        if (walls.e) { ctx.moveTo(px + cell, py); ctx.lineTo(px + cell, py + cell); }
        if (walls.s) { ctx.moveTo(px, py + cell); ctx.lineTo(px + cell, py + cell); }
        if (walls.w) { ctx.moveTo(px, py); ctx.lineTo(px, py + cell); }
        ctx.stroke();
      }
    }

    const start = maze.start;
    const exit = maze.exit;
    ctx.fillStyle = style.start;
    ctx.fillRect(pad + start[0] * cell + cell * 0.25, pad + start[1] * cell + cell * 0.25, cell * 0.5, cell * 0.5);
    ctx.fillStyle = style.exit;
    ctx.fillRect(pad + exit[0] * cell + cell * 0.25, pad + exit[1] * cell + cell * 0.25, cell * 0.5, cell * 0.5);

    if (cursor) {
      ctx.fillStyle = style.cursor;
      ctx.beginPath();
      ctx.arc(pad + cursor.x * cell + cell / 2, pad + cursor.y * cell + cell / 2, cell * 0.22, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  function drawAll() {
    document.getElementById("seek-step").value = Math.max(0, ...panes.map(p => p.playIndex));
    report.targets.forEach((target, i) => {
      draw(panes[i], target, panes[i].playIndex);
    });
  }

  function stopPlay() {
    playing = false;
    window.clearTimeout(timer);
    playBtn.textContent = "Play";
  }

  function speedFactor() { return SPEEDS[Number(speed.value)]; }

  function updateSpeedLabel() {
    speedLabel.textContent = speedFactor() + "× · " + (realtime.checked
      ? "recorded delays" : (BASE_STEP_MS / speedFactor()) + " ms/step");
  }

  function currentDelay() {
    if (!realtime.checked) return BASE_STEP_MS / speedFactor();
    // Use the next decision, and exclude already-finished strategies from pacing.
    const ns = panes.reduce((max, pane) => Math.max(max, pane.steps[pane.playIndex]?.latency_ns || 0), 0);
    return Math.max(ns / 1e6 / speedFactor(), 4);
  }

  function scheduleNext() {
    window.clearTimeout(timer);
    if (playing) timer = window.setTimeout(tick, currentDelay());
  }

  function tick() {
    panes.forEach((pane) => {
      if (pane.playIndex < pane.steps.length) pane.playIndex += 1;
    });
    drawAll();
    if (panes.every((pane) => pane.playIndex === pane.steps.length)) {
      stopPlay();
      log("[INFO] Playback complete.");
      return;
    }
    scheduleNext();
  }

  function applyLayout() {
    const width = viewport.clientWidth;
    if (!width) return;
    const factor = Number(scaleInput.value) / 100;
    const cols = columnsInput.value === "auto" ? (autoColumns || (width < 700 ? 1 : 2)) : Number(columnsInput.value);
    stage.style.width = width + "px";
    compareEl.style.gridTemplateColumns = "repeat(" + Math.min(cols, panes.length || 1) + ", minmax(0, 1fr))";
    stage.style.transform = "scale(" + factor + ")";
    stage.style.left = Math.max(0, (width - width * factor) / 2) + "px";
    viewport.style.height = Math.ceil(stage.scrollHeight * factor) + 2 + "px";
    scaleLabel.textContent = scaleInput.value + "%";
  }

  function fitOnScreen() {
    if (!report) return;
    captureInput.checked = true;
    document.body.classList.add("recording");
    // Fit uses a clean top-of-page viewport rather than the previous scroll offset.
    window.scrollTo(0, 0);
    const width = viewport.clientWidth;
    const height = Math.max(40, window.innerHeight - viewport.getBoundingClientRect().top - 12);
    stage.style.width = width + "px";
    let best = { score: -1, cols: 1, scale: 0.1 };
    const candidates = columnsInput.value === "auto" ? Array.from({length: Math.min(panes.length, 11)}, (_, i) => i + 1) : [Number(columnsInput.value)];
    candidates.forEach((cols) => {
      compareEl.style.gridTemplateColumns = "repeat(" + cols + ", minmax(0, 1fr))";
      const factor = Math.min(1, height / (stage.scrollHeight + 2));
      const score = width / cols * factor;
      if (score > best.score) best = { score, cols, scale: factor };
    });
    autoColumns = best.cols;
    scaleInput.value = String(Math.max(10, Math.floor(best.scale * 100)));
    applyLayout();
    // Auto fit chooses the arrangement with the largest visible maze cells.
    compareEl.style.gridTemplateColumns = "repeat(" + best.cols + ", minmax(0, 1fr))";
    viewport.style.height = Math.ceil(stage.scrollHeight * Number(scaleInput.value) / 100) + 2 + "px";
  }

  function refreshLayout() {
    window.cancelAnimationFrame(layoutFrame);
    layoutFrame = window.requestAnimationFrame(() => fitEnabled ? fitOnScreen() : applyLayout());
  }

  function loadReport(data) {
    const validated = validate(data);
    stopPlay();
    report = validated;
    createPanes(report.targets);
    renderOverview();
    playBtn.disabled = false;
    resetBtn.disabled = false;
    endBtn.disabled = false;
    const seek = document.getElementById("seek-step");
    seek.disabled = false;
    seek.max = Math.max(...panes.map(p => p.steps.length));
    seek.value = 0;
    fitBtn.disabled = false;
    compareEl.classList.toggle("single", report.targets.length < 2);
    panes.forEach((pane, i) => {
      pane.playIndex = 0;
      const target = report.targets[i];
      pane.root.hidden = !target;
      if (!target) {
        pane.nameEl.textContent = "—";
        pane.metaEl.textContent = "no second target in this report";
        pane.metricsEl.innerHTML = "";
        return;
      }
      renderPaneMeta(pane, target);
      draw(pane, target, 0);
    });
    ok("imported " + report.run_id + " / " + report.scenario + " · " + report.targets.length + " strategies");
    refreshLayout();
    log("[INFO] Recorded strategy comparison loaded.");
    log("[INFO] Split-screen replay: " + report.targets.map((t) => t.name + "/" + t.model).join(" | "));

  }

  function readFile(file) {
    const reader = new FileReader();
    reader.onload = function () {
      try {
        loadReport(JSON.parse(String(reader.result)));
      } catch (err) {
        fail(err.message);
      }
    };
    reader.readAsText(file);
  }

  dropzone.addEventListener("dragover", (e) => {
    e.preventDefault();
    dropzone.classList.add("drag");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("drag"));
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropzone.classList.remove("drag");
    if (e.dataTransfer.files[0]) readFile(e.dataTransfer.files[0]);
  });
  fileInput.addEventListener("change", () => {
    if (fileInput.files[0]) readFile(fileInput.files[0]);
  });
  playBtn.addEventListener("click", () => {
    if (!report) return;
    if (playing) {
      stopPlay();
      return;
    }
    if (panes.every((pane) => pane.playIndex === pane.steps.length)) {
      panes.forEach((pane) => { pane.playIndex = 0; });
      drawAll();
    }
    playing = true;
    playBtn.textContent = "Pause";
    scheduleNext();
  });
  resetBtn.addEventListener("click", () => {
    stopPlay();
    panes.forEach((pane) => {
      pane.playIndex = 0;
    });
    drawAll();
  });
  endBtn.addEventListener("click", () => {
    stopPlay();
    panes.forEach((pane) => { pane.playIndex = pane.steps.length; });
    drawAll();
  });
  document.getElementById("seek-step").addEventListener("input", (event) => {
    if (!report) return;
    stopPlay();
    const index = Math.max(0, Math.min(Number(event.target.max), Math.floor(Number(event.target.value) || 0)));
    panes.forEach(p => { p.playIndex = Math.min(index, p.steps.length); });
    drawAll();
  });
  speed.addEventListener("input", () => { updateSpeedLabel(); scheduleNext(); });
  realtime.addEventListener("change", () => { updateSpeedLabel(); scheduleNext(); });
  scaleInput.addEventListener("input", () => { fitEnabled = false; applyLayout(); });
  columnsInput.addEventListener("change", refreshLayout);
  captureInput.addEventListener("change", () => {
    fitEnabled = false;
    document.body.classList.toggle("recording", captureInput.checked);
    refreshLayout();
  });
  fitBtn.addEventListener("click", () => { fitEnabled = true; fitOnScreen(); });
  fullscreenBtn.addEventListener("click", async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await document.documentElement.requestFullscreen();
    } catch (err) { fail("Fullscreen unavailable: " + err.message); }
  });
  document.addEventListener("fullscreenchange", () => {
    fullscreenBtn.textContent = document.fullscreenElement ? "Exit fullscreen" : "Fullscreen";
    refreshLayout();
  });
  window.addEventListener("resize", refreshLayout);
  updateSpeedLabel();

  if (window.JEV_EMBEDDED_REPORT) {
    try {
      loadReport(window.JEV_EMBEDDED_REPORT);
    } catch (err) {
      fail(err.message);
    }
  }
})();
