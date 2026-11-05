const api = window.__TAURI__;
const invoke = api ? api.core.invoke : null;
const listen = api ? api.event.listen : null;

let S = null;
let pushTimer = null;

const el = (id) => document.getElementById(id);

function hexToRgb(hex) {
  const v = hex.replace("#", "");
  return [
    parseInt(v.slice(0, 2), 16),
    parseInt(v.slice(2, 4), 16),
    parseInt(v.slice(4, 6), 16),
  ];
}

function rgbToHex(rgb) {
  return "#" + rgb.map((n) => n.toString(16).padStart(2, "0")).join("");
}

// 同一个颜色压暗或提亮，用来给预设圆点做一点立体感
function shade(hex, factor) {
  const [r, g, b] = hexToRgb(hex);
  const f = (v) => Math.round(Math.max(0, Math.min(255, v * factor)));
  return `rgb(${f(r)},${f(g)},${f(b)})`;
}

function toHHMM(minutes) {
  const h = Math.floor(minutes / 60) % 24;
  const m = Math.round(minutes % 60);
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

function fromHHMM(text) {
  const [h, m] = text.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

function presetName(id) {
  const p = S.presets.find((x) => x.id === id);
  return p ? p.name : "自定义";
}

const darkMedia = window.matchMedia("(prefers-color-scheme: dark)");
const THEME_ORDER = ["system", "light", "dark"];
const THEME_LABEL = { system: "跟随系统", light: "白天", dark: "黑夜" };

function resolvedTheme(mode) {
  if (mode === "light" || mode === "dark") return mode;
  return darkMedia.matches ? "dark" : "light";
}

function applyTheme() {
  const mode = S.config.theme || "system";
  const root = document.documentElement;
  root.dataset.mode = mode;
  root.dataset.theme = resolvedTheme(mode);
  el("theme").title = `主题：${THEME_LABEL[mode]}`;
}

function renderStatus() {
  const c = S.config;
  const e = S.effective;
  const parts = [presetName(c.preset)];

  if (!c.enabled) {
    parts.push("已暂停");
  } else if (e && e.kind === "kelvin") {
    parts.push(`${e.kelvin}K`);
  } else if (e && e.kind === "rgb") {
    parts.push("滤色");
  }
  if (c.brightness < 0.995) parts.push(`亮度 ${Math.round(c.brightness * 100)}%`);
  parts.push(`${S.displays.length} 块屏`);

  el("status").textContent = parts.join(" · ");

  const mode = el("mode");
  if (!c.enabled) {
    mode.textContent = "已暂停";
    mode.className = "pill off";
  } else if (c.schedule.enabled) {
    mode.textContent = "自动";
    mode.className = "pill";
  } else {
    mode.textContent = "手动";
    mode.className = "pill off";
  }

  el("power").classList.toggle("on", c.enabled);

  const hdr = S.displays.some((d) => d.hdr === true);
  el("hdr-warn").classList.toggle("hidden", !hdr);
}

function selectPreset(p) {
  S.config.preset = p.id;
  S.config.enabled = true;
  if (p.kelvin != null) S.config.kelvin = p.kelvin;
  pushNow().then(renderAll);
}

function renderPresets() {
  const wrap = el("presets");
  wrap.innerHTML = "";

  for (const p of S.presets) {
    const row = document.createElement("div");
    row.className = "p-item" + (S.config.preset === p.id ? " active" : "");
    row.title = p.hint;
    row.tabIndex = 0;
    row.setAttribute("role", "button");
    row.addEventListener("click", () => selectPreset(p));
    row.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        selectPreset(p);
      }
    });

    const sw = document.createElement("span");
    sw.className = "sw";
    sw.style.background = `linear-gradient(150deg, ${shade(p.swatch, 1.16)} 0%, ${p.swatch} 46%, ${shade(p.swatch, 0.82)} 100%)`;

    const nm = document.createElement("span");
    nm.className = "p-name";
    nm.textContent = p.name;

    const val = document.createElement("span");
    val.className = "p-val";
    val.textContent = p.kelvin != null ? `${p.kelvin}K` : "滤色";

    const del = document.createElement("button");
    del.type = "button";
    del.className = "p-del";
    del.textContent = "×";
    del.title = "移出预设";
    del.addEventListener("click", async (e) => {
      e.stopPropagation();
      S = await invoke("delete_preset", { id: p.id });
      renderAll();
    });

    row.append(sw, nm, val, del);
    wrap.appendChild(row);
  }

  const add = document.createElement("button");
  add.type = "button";
  add.className = "p-add";
  add.textContent = "＋ 保存当前颜色";
  add.title = "把当前生效的颜色存成一个预设";
  add.addEventListener("click", showPresetForm);
  wrap.appendChild(add);
}

function suggestName() {
  const e = S.effective;
  if (!S.config.enabled || !e) return "我的预设";
  return e.kind === "kelvin" ? `${e.kelvin}K` : "我的颜色";
}

function showPresetForm() {
  const wrap = el("presets");
  const existing = wrap.querySelector(".p-form");
  if (existing) {
    existing.querySelector("input").focus();
    return;
  }

  const form = document.createElement("div");
  form.className = "p-form";

  const input = document.createElement("input");
  input.type = "text";
  input.maxLength = 12;
  input.placeholder = "预设名称";
  input.value = suggestName();

  const save = document.createElement("button");
  save.type = "button";
  save.className = "primary";
  save.textContent = "保存";

  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.textContent = "取消";

  const submit = async () => {
    const name = input.value.trim();
    if (!name) {
      input.focus();
      return;
    }
    S = await invoke("save_preset", { name });
    renderAll();
  };

  save.addEventListener("click", submit);
  cancel.addEventListener("click", () => form.remove());
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    }
    if (e.key === "Escape") form.remove();
  });

  form.append(input, save, cancel);
  wrap.appendChild(form);
  input.focus();
  input.select();
}

function renderControls() {
  const c = S.config;
  el("kelvin").value = c.kelvin;
  el("kelvinNum").textContent = c.kelvin;
  el("brightness").value = Math.round(c.brightness * 100);
  el("brightnessVal").textContent = `${Math.round(c.brightness * 100)}%`;
  if (c.color) el("color").value = rgbToHex(c.color);
  el("kelvin").classList.toggle("dim", c.schedule.enabled);
  el("autoNote").classList.toggle("hidden", !c.schedule.enabled);
}

function renderSchedule() {
  const s = S.config.schedule;
  el("auto").checked = s.enabled;
  el("dayStart").value = toHHMM(s.day_start);
  el("nightStart").value = toHHMM(s.night_start);
  el("dayK").value = s.day_kelvin;
  el("nightK").value = s.night_kelvin;
  el("transition").value = s.transition;
  el("autoNote").classList.toggle("hidden", !s.enabled);
}

function renderTargets() {
  const sel = el("target");
  sel.innerHTML = "";
  const all = document.createElement("option");
  all.value = "all";
  all.textContent = `全部屏幕（${S.displays.length} 块）`;
  sel.appendChild(all);
  for (const d of S.displays) {
    const o = document.createElement("option");
    o.value = d.id;
    o.textContent = `${d.name} · ${d.width}×${d.height}${d.primary ? " · 主屏" : ""}`;
    sel.appendChild(o);
  }
  sel.value = S.config.target;
}

function renderAll() {
  renderStatus();
  renderPresets();
  renderControls();
  renderSchedule();
  renderTargets();
}

async function pushNow() {
  S = await invoke("update_config", { cfg: S.config });
  renderStatus();
}

function pushSoon() {
  clearTimeout(pushTimer);
  pushTimer = setTimeout(pushNow, 120);
}

function bindEvents() {
  el("power").addEventListener("click", () => {
    S.config.enabled = !S.config.enabled;
    pushNow().then(renderAll);
  });

  el("kelvin").addEventListener("input", (e) => {
    const v = Number(e.target.value);
    S.config.kelvin = v;
    S.config.preset = "custom";
    S.config.color = null;
    el("kelvinNum").textContent = v;
    renderPresets();
    pushSoon();
  });

  el("brightness").addEventListener("input", (e) => {
    const v = Number(e.target.value);
    S.config.brightness = v / 100;
    el("brightnessVal").textContent = `${v}%`;
    pushSoon();
  });

  el("color").addEventListener("input", (e) => {
    S.config.color = hexToRgb(e.target.value);
    S.config.preset = "custom";
    S.config.enabled = true;
    renderPresets();
    pushSoon();
  });

  el("auto").addEventListener("change", (e) => {
    S.config.schedule.enabled = e.target.checked;
    pushNow().then(renderAll);
  });

  for (const [id, key] of [
    ["dayK", "day_kelvin"],
    ["nightK", "night_kelvin"],
    ["transition", "transition"],
  ]) {
    el(id).addEventListener("change", (e) => {
      S.config.schedule[key] = Number(e.target.value);
      pushNow().then(renderStatus);
    });
  }

  el("dayStart").addEventListener("change", (e) => {
    S.config.schedule.day_start = fromHHMM(e.target.value);
    pushNow().then(renderStatus);
  });

  el("nightStart").addEventListener("change", (e) => {
    S.config.schedule.night_start = fromHHMM(e.target.value);
    pushNow().then(renderStatus);
  });

  el("target").addEventListener("change", (e) => {
    S.config.target = e.target.value;
    pushNow();
  });

  el("autostart").addEventListener("change", async (e) => {
    const want = e.target.checked;
    // 以注册表实际写入结果为准，失败就把开关拨回去
    const ok = await invoke("set_autostart", { on: want });
    e.target.checked = ok;
    if (!ok) el("autostart").title = "写入注册表失败";
  });

  el("theme").addEventListener("click", () => {
    const cur = S.config.theme || "system";
    const next = THEME_ORDER[(THEME_ORDER.indexOf(cur) + 1) % THEME_ORDER.length];
    S.config.theme = next;
    applyTheme();
    pushNow();
  });

  // 跟随系统时，系统切换深浅色要立刻响应
  darkMedia.addEventListener("change", () => {
    if (!S || (S.config.theme || "system") !== "system") return;
    document.documentElement.dataset.theme = darkMedia.matches ? "dark" : "light";
  });
}

async function boot() {
  if (!invoke) {
    el("status").textContent = "请从应用内打开";
    return;
  }

  S = await invoke("bootstrap");
  applyTheme();
  bindEvents();
  renderAll();
  el("autostart").checked = await invoke("get_autostart");

  if (listen) {
    await listen("config-changed", (event) => {
      S = event.payload;
      renderAll();
    });
    await listen("status", (event) => {
      S = event.payload;
      renderStatus();
    });
  }

  setInterval(async () => {
    if (!S || !S.config.schedule.enabled) return;
    S = await invoke("bootstrap");
    renderStatus();
  }, 30000);
}

boot();
