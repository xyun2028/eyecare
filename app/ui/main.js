const api = window.__TAURI__;
const tauriInvoke = api ? api.core.invoke : null;
const listen = api ? api.event.listen : null;

/* 没有宿主时（浏览器直接打开 ui/index.html）用内存里的演示后端，
   交互和布局都能跑，方便单独调界面。 */
function createMock() {
  let cfg = {
    enabled: true,
    preset: "office",
    kelvin: 4500,
    brightness: 1,
    color: null,
    target: "all",
    theme: "system",
    extra_presets: [],
    hidden_presets: [],
    schedule: {
      enabled: false,
      mode: "clock",
      day_kelvin: 5500,
      night_kelvin: 3000,
      day_start: 420,
      night_start: 1140,
      transition: 60,
      lat: 22.55,
      lon: 114.06,
      sun_margin: 40,
      points: [
        { at: 420, kelvin: 5500 },
        { at: 540, kelvin: 5200 },
        { at: 840, kelvin: 5000 },
        { at: 1110, kelvin: 4000 },
        { at: 1350, kelvin: 3200 },
      ],
      weekend_points: [],
    },
    break_plan: {
      enabled: true,
      work_minutes: 20,
      rest_seconds: 20,
      dim_screen: true,
      due_at: Math.floor(Date.now() / 1000) + 20 * 60,
    },
  };

  const builtin = [
    { id: "off", name: "原色", hint: "不做任何调整", kelvin: 6500, color: null, swatch: "#d7e2f4" },
    { id: "soft", name: "轻度", hint: "轻微偏暖", kelvin: 5500, color: null, swatch: "#f5e3c8" },
    { id: "office", name: "办公", hint: "长时间写代码", kelvin: 4500, color: null, swatch: "#ffd096" },
    { id: "night", name: "夜间", hint: "晚上看屏幕", kelvin: 3400, color: null, swatch: "#ffb173" },
    { id: "sleep", name: "睡前", hint: "睡前半小时", kelvin: 2700, color: null, swatch: "#ff8f4f" },
    { id: "green", name: "豆沙绿", hint: "老式护眼绿", kelvin: null, color: [199, 237, 204], swatch: "#a9ddb7" },
    { id: "paper", name: "纸质暖黄", hint: "看文档不刺眼", kelvin: null, color: [250, 235, 210], swatch: "#f0d29f" },
  ];

  const displays = [{ id: "mock-1", name: "演示屏幕", width: 1920, height: 1080, primary: true, hdr: false }];

  const presets = () => [
    ...builtin.filter((p) => !cfg.hidden_presets.includes(p.id)),
    ...cfg.extra_presets,
  ];

  const effective = () => {
    if (!cfg.enabled) return null;
    if (cfg.schedule.enabled) {
      const pts = cfg.schedule.mode === "sun" ? sunPoints() : cfg.schedule.points;
      return { kind: "kelvin", kelvin: Math.round(sampleKelvin(pts, minutesNow(), cfg.schedule.transition)) };
    }
    if (cfg.preset === "custom") {
      return cfg.color ? { kind: "rgb", rgb: cfg.color } : { kind: "kelvin", kelvin: cfg.kelvin };
    }
    const p = presets().find((x) => x.id === cfg.preset);
    if (p?.color) return { kind: "rgb", rgb: p.color };
    return { kind: "kelvin", kelvin: p?.kelvin ?? cfg.kelvin };
  };

  const sunPoints = () => [
    { at: 362, kelvin: cfg.schedule.night_kelvin },
    { at: 442, kelvin: cfg.schedule.day_kelvin },
    { at: 1043, kelvin: cfg.schedule.day_kelvin },
    { at: 1123, kelvin: cfg.schedule.night_kelvin },
  ];

  const status = () => ({
    config: cfg,
    presets: presets(),
    displays,
    effective: effective(),
    clock: "12:00",
    schedule_view: {
      effective_points: cfg.schedule.mode === "sun" ? sunPoints() : cfg.schedule.points,
      sun_times: { sunrise: 402, sunset: 1083 },
      weekend: false,
    },
    break_view: {
      remaining_seconds: cfg.break_plan.enabled
        ? Math.max(0, cfg.break_plan.due_at - Math.floor(Date.now() / 1000))
        : null,
      resting: false,
    },
    stats: {
      today: { seconds: 2 * 3600 + 14 * 60, breaks: 5, longest_streak_seconds: 52 * 60 },
      streak_seconds: 18 * 60,
      week: Array.from({ length: 7 }, (_, i) => {
        const d = new Date(Date.now() - (6 - i) * 86400000);
        return { date: d.toISOString().slice(0, 10), seconds: [3, 5, 2, 6, 4, 1, 2][i] * 3600, breaks: 4 };
      }),
    },
  });

  return async (method, args = {}) => {
    if (method === "bootstrap") return status();
    if (method === "update_config") {
      cfg = args.cfg;
      return status();
    }
    if (method === "get_autostart") return false;
    if (method === "set_autostart") return args.on;
    if (method === "end_rest") return status();
    if (method === "restore_presets") {
      cfg.hidden_presets = [];
      return status();
    }
    if (method === "save_preset") {
      const id = `u${Date.now()}`;
      const rgb = cfg.color;
      cfg.extra_presets.push({
        id,
        name: args.name || "未命名",
        hint: "自己保存的",
        kelvin: rgb ? null : cfg.kelvin,
        color: rgb,
        swatch: rgb ? rgbToHex(rgb) : "#ffd096",
      });
      cfg.preset = id;
      return status();
    }
    if (method === "delete_preset") {
      cfg.hidden_presets.push(args.id);
      cfg.extra_presets = cfg.extra_presets.filter((p) => p.id !== args.id);
      return status();
    }
    throw new Error(`演示后端没有实现 ${method}`);
  };
}

const mockInvoke = tauriInvoke ? null : createMock();
const invoke = tauriInvoke || mockInvoke;

let S = null;
let pushTimer = null;

/** 曲线里当前选中的锚点下标 */
let selectedAnchor = 0;
/** 正在编辑哪条曲线：weekday / weekend */
let editDay = "weekday";
/** 拖动过程中的临时点，松手才提交，避免每帧一次写盘 */
let draftPoints = null;
let dragging = null;

let breakRemain = 0;
let bannerLeft = 0;

const el = (id) => document.getElementById(id);

/* 曲线的坐标系。viewBox 固定 720×132，容器按 aspect-ratio 等比缩放，
   所以拖动时用 getBoundingClientRect 换算，不需要管实际像素宽度。 */
const CW = 720;
const CH = 100;
const CTOP = 10;
const CBOT = 88;
const KMIN = 1000;
const KMAX = 6500;
const SVG_NS = "http://www.w3.org/2000/svg";

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function hexToRgb(hex) {
  const v = hex.replace("#", "");
  return [
    parseInt(v.slice(0, 2), 16),
    parseInt(v.slice(2, 4), 16),
    parseInt(v.slice(4, 6), 16),
  ];
}

function rgbToHex(rgb) {
  return "#" + rgb.map((n) => Math.round(n).toString(16).padStart(2, "0")).join("");
}

// 同一个颜色压暗或提亮，用来给预设圆点做一点立体感
function shade(hex, factor) {
  const [r, g, b] = hexToRgb(hex);
  const f = (v) => Math.round(clamp(v * factor, 0, 255));
  return `rgb(${f(r)},${f(g)},${f(b)})`;
}

function toHHMM(minutes) {
  const m = ((Math.round(minutes) % 1440) + 1440) % 1440;
  const h = Math.floor(m / 60);
  return `${String(h).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

function fromHHMM(text) {
  const [h, m] = text.split(":").map(Number);
  return clamp((h || 0) * 60 + (m || 0), 0, 1439);
}

function fmtClock(seconds) {
  const s = Math.max(0, seconds | 0);
  const m = Math.floor(s / 60);
  return `${String(m).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

function fmtDuration(seconds) {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s} 秒`;
  const h = Math.floor(s / 3600);
  const m = Math.round((s % 3600) / 60);
  return h > 0 ? `${h} 小时 ${m} 分` : `${m} 分`;
}

function minutesNow() {
  const d = new Date();
  return d.getHours() * 60 + d.getMinutes() + d.getSeconds() / 60;
}

function presetName(id) {
  const p = S.presets.find((x) => x.id === id);
  return p ? p.name : "自定义";
}

/* ------------------------------------------------------------------ 主题 */

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

/* ------------------------------------------------------------------ 曲线计算 */

/** 和后端 config::sample 保持一致的取值规则，否则拖动时预览和实际写屏会对不上 */
function sampleKelvin(points, minutes, transition) {
  if (!points || points.length === 0) return KMAX;
  if (points.length === 1) return points[0].kelvin;

  const pts = [...points].sort((a, b) => a.at - b.at);
  const n = pts.length;
  const m = ((minutes % 1440) + 1440) % 1440;

  let idx = n - 1;
  for (let i = 0; i < n; i++) {
    if (pts[i].at <= m) idx = i;
    else break;
  }

  const cur = pts[idx];
  const next = pts[(idx + 1) % n];
  const curAt = cur.at;
  const nextAt = idx + 1 === n ? next.at + 1440 : next.at;
  const into = m >= curAt ? m - curAt : m + 1440 - curAt;
  const span = Math.max(0.001, nextAt - curAt);
  const tr = Math.min(transition, span);

  if (tr <= 0 || into < span - tr) return cur.kelvin;
  const t = clamp((into - (span - tr)) / tr, 0, 1);
  return cur.kelvin + (next.kelvin - cur.kelvin) * t;
}

const kx = (at) => (at / 1440) * CW;
const ky = (kelvin) => CTOP + (1 - (kelvin - KMIN) / (KMAX - KMIN)) * (CBOT - CTOP);

/** 当前在编辑的那条曲线（还没拖动时） */
function basePoints() {
  const s = S.config.schedule;
  if (s.mode === "sun") return S.schedule_view.effective_points || s.points || [];
  if (editDay === "weekend") {
    return s.weekend_points && s.weekend_points.length ? s.weekend_points : s.points || [];
  }
  return s.points || [];
}

function editPoints() {
  return draftPoints || basePoints();
}

function commitPoints(points) {
  if (!S || S.config.schedule.mode === "sun") return;
  const sorted = [...points]
    .map((p) => ({
      at: clamp(Math.round(p.at), 0, 1439),
      kelvin: clamp(Math.round(p.kelvin), KMIN, KMAX),
    }))
    .sort((a, b) => a.at - b.at);

  if (editDay === "weekend") {
    S.config.schedule.weekend_points = sorted;
  } else {
    S.config.schedule.points = sorted;
  }
  pushNow().then(() => renderCurve());
}

/* ------------------------------------------------------------------ 渲染 */

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
    mode.textContent = c.schedule.mode === "sun" ? "跟随日光" : "自动";
    mode.className = "pill";
  } else {
    mode.textContent = "手动";
    mode.className = "pill off";
  }

  el("power").classList.toggle("on", c.enabled);

  const hdr = S.displays.some((d) => d.hdr === true);
  el("hdr-warn").classList.toggle("hidden", !hdr);

  renderHero(e);
}

/// 左上角那个大数字。色温模式下是「4500 K」，滤色模式下换成色块加「滤色」。
/// 不跟着 effective 变的话，用户在滤色预设下看到数字没动，会以为没切过去。
function renderHero(eff) {
  const hero = el("heroValue");
  const slider = el("kelvin");

  if (eff && eff.kind === "rgb") {
    hero.classList.add("filtered");
    hero.textContent = "";

    const dot = document.createElement("span");
    dot.className = "hero-dot";
    dot.style.background = rgbToHex(eff.rgb);

    const label = document.createElement("i");
    label.textContent = "滤色";

    hero.append(dot, label);
    slider.classList.add("dim");
    slider.title = "当前是滤色模式，拖动会切回色温";
    return;
  }

  hero.classList.remove("filtered");
  slider.title = "";

  if (!hero.querySelector("#kelvinNum")) {
    hero.textContent = "";
    const num = document.createElement("span");
    num.id = "kelvinNum";
    const unit = document.createElement("i");
    unit.textContent = "K";
    hero.append(num, unit);
  }

  const shown = eff && eff.kind === "kelvin" ? eff.kelvin : S.config.kelvin;
  hero.querySelector("#kelvinNum").textContent = shown;
}

function selectPreset(p) {
  S.config.preset = p.id;
  S.config.enabled = true;
  if (p.kelvin != null) S.config.kelvin = p.kelvin;
  S.config.color = p.color || null;
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

  // 只有确实移出过内置预设，才显示「恢复默认」
  const hiddenCount = (S.config.hidden_presets || []).length;
  el("restorePresets").classList.toggle("hidden", hiddenCount === 0);
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
  el("brightness").value = Math.round(c.brightness * 100);
  el("brightnessVal").textContent = `${Math.round(c.brightness * 100)}%`;
  if (c.color) el("color").value = rgbToHex(c.color);
  el("kelvin").classList.toggle("dim", c.schedule.enabled);
  el("autoNote").classList.toggle("hidden", !c.schedule.enabled);
}

function renderScheduleMode() {
  const mode = S.config.schedule.mode || "clock";
  el("auto").checked = S.config.schedule.enabled;

  for (const btn of el("modeSeg").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.mode === mode);
  }

  const isSun = mode === "sun";
  el("daySeg").classList.toggle("disabled", isSun);
  el("anchorTime").disabled = isSun;
  el("anchorK").disabled = isSun;
  el("anchorHint").textContent = isSun
    ? "日出日落模式下，时刻由太阳决定"
    : "点曲线上的圆点选中，拖动可改时间和色温";

  el("transition").value = S.config.schedule.transition;

  const note = el("dayNote");
  note.classList.remove("clickable");
  if (isSun) {
    note.textContent = "日光模式不分工作日和周末";
    return;
  }
  if (editDay !== "weekend") {
    note.textContent = "";
    return;
  }
  const own = (S.config.schedule.weekend_points || []).length > 0;
  if (own) {
    note.textContent = "恢复跟随工作日";
    note.classList.add("clickable");
  } else {
    note.textContent = "跟随工作日，拖动任意圆点即可单独设置";
  }

  for (const btn of el("daySeg").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.day === editDay);
  }
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

/* ---------------------------------------------------------------- 曲线绘制 */

function renderCurve() {
  if (!S) return;

  const points = editPoints();
  const transition = S.config.schedule.transition || 0;
  const isSun = S.config.schedule.mode === "sun";

  renderGrid();

  let d = "";
  for (let m = 0; m <= 1440; m += 5) {
    const k = sampleKelvin(points, m, transition);
    d += `${m === 0 ? "M" : "L"}${kx(m).toFixed(2)} ${ky(k).toFixed(2)} `;
  }
  el("curveLine").setAttribute("d", d);
  el("curveArea").setAttribute("d", `${d}L${CW} ${CBOT} L0 ${CBOT} Z`);

  const dots = el("curveDots");
  dots.innerHTML = "";
  points.forEach((p, i) => {
    const c = document.createElementNS(SVG_NS, "circle");
    c.setAttribute("cx", kx(p.at));
    c.setAttribute("cy", ky(p.kelvin));
    c.setAttribute("r", i === selectedAnchor ? 6 : 4.5);
    c.setAttribute("class", "anchor-dot" + (i === selectedAnchor ? " active" : "") + (isSun ? " locked" : ""));
    c.dataset.index = String(i);
    if (!isSun) c.addEventListener("pointerdown", onDotDown);
    dots.appendChild(c);
  });

  const now = minutesNow();
  el("curveNow").setAttribute("x1", kx(now).toFixed(2));
  el("curveNow").setAttribute("x2", kx(now).toFixed(2));
  el("curveNow").setAttribute("class", "curve-now" + (S.config.schedule.enabled ? " live" : ""));

  syncAnchorInputs();
}

function renderGrid() {
  const grid = el("curveGrid");
  grid.innerHTML = "";

  const add = (tag, attrs) => {
    const node = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
    grid.appendChild(node);
    return node;
  };

  for (const k of [2000, 3500, 5000]) {
    add("line", { x1: 0, y1: ky(k).toFixed(1), x2: CW, y2: ky(k).toFixed(1), class: "grid-h" });
  }

  for (let h = 0; h <= 24; h += 6) {
    const x = kx(h * 60);
    add("line", { x1: x, y1: CTOP, x2: x, y2: CBOT, class: "grid-v" });
    if (h > 0 && h < 24) {
      const label = add("text", { x, y: 9, class: "grid-label", "text-anchor": "middle" });
      label.textContent = `${h}:00`;
    }
  }

  const times = S.schedule_view && S.schedule_view.sun_times;
  if (S.config.schedule.mode === "sun" && times) {
    for (const [key, caption] of [["sunrise", "日出"], ["sunset", "日落"]]) {
      const x = kx(times[key]);
      add("line", { x1: x, y1: CTOP, x2: x, y2: CBOT, class: "grid-sun" });
      const label = add("text", { x: x + 3, y: CH - 3, class: "grid-sun-label" });
      label.textContent = `${caption} ${toHHMM(times[key])}`;
    }
  }
}

function syncAnchorInputs() {
  const points = editPoints();
  const p = points[selectedAnchor];
  if (!p) return;
  el("anchorTime").value = toHHMM(p.at);
  el("anchorK").value = p.kelvin;
}

/* ---------------------------------------------------------------- 锚点拖动 */

function pointerToCurve(event) {
  const rect = el("curve").getBoundingClientRect();
  return {
    x: ((event.clientX - rect.left) / rect.width) * CW,
    y: ((event.clientY - rect.top) / rect.height) * CH,
  };
}

function onDotDown(event) {
  if (S.config.schedule.mode === "sun") return;
  event.preventDefault();

  selectedAnchor = Number(event.currentTarget.dataset.index);
  dragging = { index: selectedAnchor };

  window.addEventListener("pointermove", onDragMove);
  window.addEventListener("pointerup", onDragUp, { once: true });

  renderCurve();
}

function onDragMove(event) {
  if (!dragging) return;

  const { x, y } = pointerToCurve(event);
  const points = editPoints().map((p) => ({ ...p }));
  const index = Math.min(dragging.index, points.length - 1);
  if (!points[index]) return;

  points[index] = {
    at: clamp(Math.round((x / CW) * 1440 / 5) * 5, 0, 1435),
    kelvin: clamp(Math.round(((1 - (y - CTOP) / (CBOT - CTOP)) * (KMAX - KMIN) + KMIN) / 50) * 50, KMIN, KMAX),
  };

  draftPoints = points;
  renderCurve();
}

function onDragUp() {
  window.removeEventListener("pointermove", onDragMove);
  if (!dragging) return;

  const points = draftPoints;
  const index = dragging.index;
  dragging = null;
  draftPoints = null;

  if (!points) return;

  const target = points[index];
  const sorted = [...points].sort((a, b) => a.at - b.at);
  const nextIndex = sorted.findIndex((p) => p.at === target.at && p.kelvin === target.kelvin);
  if (nextIndex >= 0) selectedAnchor = nextIndex;

  commitPoints(points);
}

/* ---------------------------------------------------------------- 休息提醒 */

function renderBreak() {
  const plan = S.config.break_plan;
  const view = S.break_view || {};

  el("breakOn").checked = plan.enabled;
  el("workMinutes").value = plan.work_minutes;
  el("dimScreen").checked = plan.dim_screen;

  if (!plan.enabled) {
    el("breakTime").textContent = "--:--";
    el("breakBar").style.width = "0%";
    el("breakCap").textContent = "提醒已关闭";
    return;
  }

  breakRemain = view.remaining_seconds ?? 0;
  updateBreakClock();
}

function updateBreakClock() {
  if (!S || !S.config.break_plan.enabled) return;

  const plan = S.config.break_plan;
  el("breakTime").textContent = fmtClock(breakRemain);

  const resting = S.break_view && S.break_view.resting;
  el("breakCap").textContent = resting ? "正在休息" : "后提醒休息";

  const total = Math.max(1, plan.work_minutes * 60);
  el("breakBar").style.width = `${Math.min(100, (breakRemain / total) * 100)}%`;
}

function showBanner(seconds) {
  bannerLeft = seconds;
  el("break-banner").classList.remove("hidden");
  el("bannerCount").textContent = bannerLeft;
}

function hideBanner() {
  el("break-banner").classList.add("hidden");
}

/* ------------------------------------------------------------------ 统计 */

function renderStats() {
  const stats = S.stats;
  if (!stats) return;

  const today = stats.today || { seconds: 0, breaks: 0, longest_streak_seconds: 0 };
  const total = today.seconds || 0;

  el("todayHours").textContent = Math.floor(total / 3600);
  el("todayMins").textContent = Math.floor((total % 3600) / 60);
  el("todayMeta").textContent =
    total < 60
      ? "今天还没有记录"
      : `休息 ${today.breaks} 次 · 最长连续 ${fmtDuration(today.longest_streak_seconds || 0)}`;

  const streak = stats.streak_seconds || 0;
  const pill = el("streakPill");
  if (!S.config.enabled) {
    pill.textContent = "已暂停";
    pill.className = "pill off";
  } else if (streak < 60) {
    pill.textContent = "刚休息过";
    pill.className = "pill off";
  } else {
    pill.textContent = `${fmtDuration(streak)} 连续`;
    pill.className = streak >= S.config.break_plan.work_minutes * 60 ? "pill" : "pill off";
  }

  const week = stats.week || [];
  const max = Math.max(1, ...week.map((d) => d.seconds || 0));
  const wrap = el("weekBars");
  wrap.innerHTML = "";

  week.forEach((day, i) => {
    const col = document.createElement("div");
    col.className = "week-col" + (i === week.length - 1 ? " today" : "");
    col.title = `${day.date} · ${fmtDuration(day.seconds || 0)}`;

    const track = document.createElement("div");
    track.className = "week-track";

    const fill = document.createElement("div");
    fill.className = "week-fill";
    fill.style.height = `${day.seconds ? Math.max(6, ((day.seconds || 0) / max) * 100) : 0}%`;
    track.appendChild(fill);

    const cap = document.createElement("span");
    cap.className = "week-day";
    cap.textContent = "日一二三四五六"[new Date(day.date + "T00:00:00").getDay()];

    col.append(track, cap);
    wrap.appendChild(col);
  });
}

/* ------------------------------------------------------------------ 渲染入口 */

function renderAll() {
  renderStatus();
  renderPresets();
  renderControls();
  renderScheduleMode();
  renderTargets();
  renderCurve();
  renderBreak();
  renderStats();
}

async function pushNow() {
  S = await invoke("update_config", { cfg: S.config });
  renderStatus();
  return S;
}

function pushSoon() {
  clearTimeout(pushTimer);
  pushTimer = setTimeout(pushNow, 120);
}

/* ------------------------------------------------------------------ 事件 */

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
    renderHero({ kind: "kelvin", kelvin: v });
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

  el("target").addEventListener("change", (e) => {
    S.config.target = e.target.value;
    pushNow();
  });

  el("auto").addEventListener("change", (e) => {
    S.config.schedule.enabled = e.target.checked;
    pushNow().then(renderAll);
  });

  el("modeSeg").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-mode]");
    if (!btn || btn.dataset.mode === S.config.schedule.mode) return;
    S.config.schedule.mode = btn.dataset.mode;
    selectedAnchor = 0;
    pushNow().then(renderAll);
  });

  el("daySeg").addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-day]");
    if (!btn || S.config.schedule.mode === "sun") return;
    editDay = btn.dataset.day;
    selectedAnchor = 0;
    draftPoints = null;
    renderScheduleMode();
    renderCurve();
  });

  el("dayNote").addEventListener("click", () => {
    if (!el("dayNote").classList.contains("clickable")) return;
    S.config.schedule.weekend_points = [];
    pushNow().then(renderAll);
  });

  el("transition").addEventListener("change", (e) => {
    S.config.schedule.transition = Number(e.target.value);
    pushNow().then(renderCurve);
  });

  el("anchorTime").addEventListener("change", (e) => {
    const points = editPoints().map((p) => ({ ...p }));
    if (!points[selectedAnchor]) return;
    points[selectedAnchor] = { ...points[selectedAnchor], at: fromHHMM(e.target.value) };
    commitPoints(points);
  });

  el("anchorK").addEventListener("change", (e) => {
    const points = editPoints().map((p) => ({ ...p }));
    if (!points[selectedAnchor]) return;
    points[selectedAnchor] = {
      ...points[selectedAnchor],
      kelvin: clamp(Number(e.target.value) || KMIN, KMIN, KMAX),
    };
    commitPoints(points);
  });

  el("breakOn").addEventListener("change", (e) => {
    S.config.break_plan.enabled = e.target.checked;
    if (!e.target.checked) S.config.break_plan.due_at = null;
    pushNow().then(renderBreak);
  });

  el("workMinutes").addEventListener("change", (e) => {
    const v = clamp(Number(e.target.value) || 20, 1, 180);
    S.config.break_plan.work_minutes = v;
    S.config.break_plan.due_at = Math.floor(Date.now() / 1000) + v * 60;
    pushNow().then(renderBreak);
  });

  el("dimScreen").addEventListener("change", (e) => {
    S.config.break_plan.dim_screen = e.target.checked;
    pushNow();
  });

  el("skip").addEventListener("click", () => {
    const minutes = S.config.break_plan.work_minutes || 20;
    S.config.break_plan.due_at = Math.floor(Date.now() / 1000) + minutes * 60;
    hideBanner();
    pushNow().then(renderBreak);
  });

  el("bannerClose").addEventListener("click", hideBanner);

  el("autostart").addEventListener("change", async (e) => {
    const want = e.target.checked;
    // 以注册表实际写入结果为准，失败就把开关拨回去
    const ok = await invoke("set_autostart", { on: want });
    e.target.checked = ok;
    if (!ok) el("autostart").title = "写入注册表失败";
  });

  el("restorePresets").addEventListener("click", async () => {
    S = await invoke("restore_presets");
    renderAll();
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

/* ------------------------------------------------------------------ 启动 */

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
      renderBreak();
      renderStats();
      renderScheduleMode();
      renderCurve();
    });
    await listen("break-due", (event) => {
      showBanner(event.payload.restSeconds || S.config.break_plan.rest_seconds || 20);
      breakRemain = S.config.break_plan.work_minutes * 60;
    });
  }

  // 本地秒表：后端每 30 秒对一次表，中间由前端自己走
  setInterval(() => {
    if (!S || !S.config.break_plan.enabled) return;
    breakRemain = Math.max(0, breakRemain - 1);
    updateBreakClock();
  }, 1000);

  // 休息横幅自己的秒数
  setInterval(() => {
    if (el("break-banner").classList.contains("hidden")) return;
    bannerLeft -= 1;
    if (bannerLeft <= 0) {
      hideBanner();
    } else {
      el("bannerCount").textContent = bannerLeft;
    }
  }, 1000);

  setInterval(async () => {
    if (!S || !S.config.schedule.enabled) return;
    S = await invoke("bootstrap");
    renderStatus();
  }, 30000);
}

boot();
