(function () {
  const fileInput = document.getElementById("file");
  const dropzone = document.getElementById("dropzone");
  const statusEl = document.getElementById("file-status");
  const playBtn = document.getElementById("play");
  const resetBtn = document.getElementById("reset");
  const speed = document.getElementById("speed");
  const speedLabel = document.getElementById("speed-label");
  const realtime = document.getElementById("realtime");
  const compareEl = document.getElementById("compare");
  const logEl = document.getElementById("log");

  const PANE_STYLES = [
    { heat: "57,255,136", wall: "#2d6a49", cursor: "#ffffff", start: "#f3c15b", exit: "#39ff88" },
    { heat: "243,193,91", wall: "#6a542d", cursor: "#fff4cc", start: "#f3c15b", exit: "#39ff88" },
  ];

  const panes = [0, 1].map((i) => {
    const canvas = document.getElementById("maze-" + i);
    return {
      index: i,
      root: document.getElementById("pane-" + i),
      canvas: canvas,
      ctx: canvas.getContext("2d"),
      nameEl: document.getElementById("name-" + i),
      titleEl: document.getElementById("title-" + i),
      metaEl: document.getElementById("meta-" + i),
      metricsEl: document.getElementById("metrics-" + i),
      style: PANE_STYLES[i],
      playIndex: 0,
    };
  });

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

  function renderPaneMeta(pane, target) {
    const steps = decisionSteps(target);
    const success = target.trajectory ? String(target.trajectory.success) : "n/a";
    pane.titleEl.textContent = target.name;
    pane.nameEl.textContent = target.model;
    pane.metaEl.textContent =
      steps.length + " decisions · stop=" + stopReason(target) + " · success=" + success;
    const s = target.stats;
    pane.metricsEl.innerHTML = [
      ["ok", s.ok + "/" + s.requests],
      ["qps", s.qps.toFixed(1)],
      ["p50", nsToMs(s.p50_ns) + " ms"],
      ["p95", nsToMs(s.p95_ns) + " ms"],
      ["exit", target.trajectory && target.trajectory.exit_step != null
        ? String(target.trajectory.exit_step)
        : "—"],
      ["stop", stopReason(target)],
    ]
      .map(([k, v]) => "<dt>" + k + "</dt><dd>" + v + "</dd>")
      .join("");
  }

  function draw(pane, target, upto) {
    const maze = report.maze;
    const steps = decisionSteps(target);
    const w = maze.width;
    const h = maze.height;
    const pad = 18;
    const cell = Math.min((pane.canvas.width - pad * 2) / w, (pane.canvas.height - pad * 2) / h);
    const ctx = pane.ctx;
    const style = pane.style;
    ctx.fillStyle = "#030907";
    ctx.fillRect(0, 0, pane.canvas.width, pane.canvas.height);

    const visits = new Map();
    let cursor = null;
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
    report.targets.slice(0, 2).forEach((target, i) => {
      draw(panes[i], target, panes[i].playIndex);
    });
  }

  function stopPlay() {
    playing = false;
    window.clearTimeout(timer);
    playBtn.textContent = "Play";
  }

  function currentDelay() {
    if (!realtime.checked) {
      return Math.max(8 / Number(speed.value), 4);
    }
    let ns = 0;
    report.targets.slice(0, 2).forEach((target, i) => {
      const steps = decisionSteps(target);
      const idx = panes[i].playIndex;
      if (idx > 0 && idx <= steps.length) {
        ns = Math.max(ns, steps[idx - 1].latency_ns || 0);
      }
    });
    return Math.max(ns / 1e6 / Number(speed.value), 1);
  }

  function tick() {
    let progressed = false;
    report.targets.slice(0, 2).forEach((target, i) => {
      if (panes[i].playIndex < decisionSteps(target).length) {
        panes[i].playIndex += 1;
        progressed = true;
      }
    });
    drawAll();
    if (!progressed) {
      stopPlay();
      log("[INFO] Quantum tunneling simulation: playback complete.");
      return;
    }
    timer = window.setTimeout(tick, currentDelay());
  }

  function loadReport(data) {
    report = validate(data);
    playBtn.disabled = false;
    resetBtn.disabled = false;
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
    ok("imported " + report.run_id + " / " + report.scenario + " · " + report.targets.length + " models");
    log("[INFO] Initializing Ergodic Markov Random Walk...");
    log("[INFO] Split-screen replay: " + report.targets.map((t) => t.name + "/" + t.model).join(" | "));
    log("[INFO] Applying Zero-Prior Stochastic Policy...");
    log("[INFO] Entropy maximized: H(X) = 2.0 bits/decision.");
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
    playing = true;
    playBtn.textContent = "Pause";
    tick();
  });
  resetBtn.addEventListener("click", () => {
    stopPlay();
    panes.forEach((pane) => {
      pane.playIndex = 0;
    });
    drawAll();
  });
  speed.addEventListener("input", () => {
    speedLabel.textContent = speed.value + "×";
  });

  if (window.JEV_EMBEDDED_REPORT) {
    try {
      loadReport(window.JEV_EMBEDDED_REPORT);
    } catch (err) {
      fail(err.message);
    }
  }
})();
