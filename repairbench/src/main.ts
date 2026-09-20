import { runIntro } from "./intro";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";

// 在 Tauri 里走 invoke；在浏览器里走 HTTP API（repairbench serve）。
// 同一份数据两条路 —— 界面因此能在浏览器里验证，不依赖 GUI 截图权限。
// 两条路背后是同一个 api 层，所以「已排除 N 份」这类提示两边一字不差。
const inTauri = "__TAURI_INTERNALS__" in window;
const API = "http://127.0.0.1:8765";

async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (inTauri) return invoke<T>(cmd, args);
  const qs = Object.entries(args)
    .filter(([, v]) => v != null && v !== "")
    .map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join("&");
  const r = await fetch(`${API}/v1/${cmd}${qs ? "?" + qs : ""}`);
  if (!r.ok) throw new Error(`HTTP ${r.status} —— repairbench serve 没在跑？`);
  const j = await r.json();
  if (j && j.error) throw new Error(j.error);
  return j as T;
}

type Rule = { id: string; rule: string };
type Recall = { id: string; component: string; severity: string; note: string };
type Gate = { level: string; rule: string };
type TaskSpec = { label: string; value: string; source: string; status: string };
type Task = { id: string; name: string; detail?: string; hint?: string; source?: string; done: boolean; specs: TaskSpec[] };
type Phase = { id: string; name: string; touch_vehicle: boolean; task_count: number; is_current: boolean };
type WB = {
  vehicle_id: string; make: string; model: string; year: number; engine_code: string; vin?: string;
  hard_rules: Rule[]; open_recalls: Recall[]; phases: Phase[];
  current_phase: string; current_name: string; current_summary: string;
  gates: Gate[]; tasks: Task[]; phase_locked: boolean; phase_lock_reason: string;
  specs_usable: number; specs_pending: number; manual_pages: number;
  manual_excluded: number; photo_count: number; warnings: string[];
};
type VehicleBrief = { id: string; engine_code?: string; year?: number };
type SpecHit = { item: string; value: string; source_level: string; source_ref: string; page: number };
type SpecResult = { vehicle: string; query: string; specs: SpecHit[]; pending_related: number };
type ManualHit = { manual: string; tier?: string; page: number; snippet: string };
type SearchResult = { hits: ManualHit[]; excluded_manuals: number };
type Part = {
  id: string; name: string; spec?: string; spec_ref?: string; qty: number;
  lead_time_days: number; needed_phase: string; status: string; why: string;
  blocked_by?: string; needs_choice?: string; est_cost?: number;
  order_within_days: number; urgency: string;
};
type Blocker = { id: string; what: string; why: string; phase: string; unblocks: string[] };
type PartsPlan = {
  current_phase: string; assumed_days_per_phase: number;
  parts: Part[]; blockers: Blocker[]; est_total: number; unknown_cost_count: number;
};
type LogRead = { total: number; entries: string[]; file: string };
type Topic = { id: string; title: string; phase: string; one_shot: boolean };
type VlogView = {
  vlog_path: string; exists: boolean; topics: Topic[]; one_shot_count: number;
  animations: { name: string; bytes: number; path: string }[];
  docs: string[]; scripts: string[]; tagged_media: number; notes: string[];
};

const esc = (s: string) => String(s).replace(/[&<>]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]!));
// 资料里用 **强调** 标记关键后果，渲染成高亮
const em = (s: string) => esc(s).replace(/\*\*(.+?)\*\*/g, "<b>$1</b>");
const mb = (n: number) => n > 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.round(n / 1e3) + " KB";

type Tab = "工作台" | "诊断" | "图纸" | "知识库" | "零件" | "日志" | "Vlog";
const TABS: Tab[] = ["工作台", "诊断", "图纸", "知识库", "零件", "日志", "Vlog"];

let vehicleId: string | null = null;
let phaseId: string | null = null;
let tab: Tab = "工作台";
let specQ = "", manQ = "";

const app = () => document.querySelector<HTMLDivElement>("#app")!;
const errBox = (e: unknown) => `<div class="err">加载失败：\n${esc(String(e))}</div>`;

/** 来源级别徽章 —— 全局一致：A 官方 / B 权威三方。C/D 不可能出现在可用规格里 */
const badge = (lvl: string, ref: string, page: number) =>
  `<span class="badge lv${esc(lvl)}">${esc(lvl)}级</span><span class="ref">${esc(ref)} · p${page}</span>`;

async function render() {
  let w: WB;
  try {
    w = await call<WB>("workbench", { vehicle: vehicleId, phase: phaseId });
  } catch (e) { app().innerHTML = errBox(e); return; }

  let vehicles: VehicleBrief[] = [];
  try { vehicles = await call<VehicleBrief[]>("vehicles"); }
  catch { vehicles = [{ id: w.vehicle_id }]; }

  app().innerHTML = `
  <header>
    <select id="vsel">${vehicles.map(v =>
      `<option value="${esc(v.id)}" ${v.id === w.vehicle_id ? "selected" : ""}>${esc(v.id)}</option>`).join("")}</select>
    <span class="bike">${w.year} ${esc(w.make)} ${esc(w.model)}</span>
    <span class="code">${esc(w.engine_code)}</span>
    <span class="vin">VIN ${w.vin ? esc(w.vin) : "— 未采集 —"}</span>
    <nav class="tabs">${TABS.map(t =>
      `<button class="tab ${t === tab ? "on" : ""}" data-t="${t}">${t}</button>`).join("")}</nav>
  </header>
  <main>
    <aside>
      <h3>维修阶段</h3>
      <div class="phases">${w.phases.map((p, i) => `
        <div class="ph ${p.is_current ? "cur" : i < w.phases.findIndex(x => x.is_current) ? "done" : ""}" data-p="${p.id}">
          <span class="n">${p.id}</span><span>${esc(p.name)}</span>
        </div>`).join("")}</div>
      <h3>资料</h3>
      <div class="stats">
        <div><b>${w.specs_usable}</b>可用规格</div>
        <div><b>${w.specs_pending}</b>待核实</div>
        <div><b>${w.manual_pages}</b>手册页</div>
        <div><b>${w.photo_count}</b>照片</div>
      </div>
      ${w.manual_excluded > 0 ? `<div class="hint">🛡 ${w.manual_excluded} 份不适用本车的资料已排除</div>` : ""}
    </aside>
    <section id="view"></section>
  </main>`;

  app().querySelectorAll<HTMLElement>(".ph").forEach(el =>
    el.onclick = () => { phaseId = el.dataset.p!; if (tab !== "工作台") tab = "工作台"; render(); });
  app().querySelectorAll<HTMLElement>(".tab").forEach(el =>
    el.onclick = () => { tab = el.dataset.t as Tab; render(); });
  const sel = app().querySelector<HTMLSelectElement>("#vsel")!;
  sel.onchange = () => { vehicleId = sel.value; phaseId = null; render(); };

  const view = app().querySelector<HTMLElement>("#view")!;
  if (tab === "工作台") viewWorkbench(view, w);
  else if (tab === "诊断") viewDiag(view);
  else if (tab === "图纸") viewCircuit(view);
  else if (tab === "知识库") viewKnowledge(view);
  else if (tab === "零件") viewParts(view);
  else if (tab === "日志") viewLog(view);
  else viewVlog(view);
}

function viewWorkbench(el: HTMLElement, w: WB) {
  el.innerHTML = `
    ${w.open_recalls.length ? `
    <h3>未执行召回 —— 全部由 BMW 免费执行</h3>
    <div class="card danger">${w.open_recalls.map(r => `
      <div class="recall">
        <div class="h"><span class="id">${esc(r.id)}</span>
          <span class="sev ${r.severity !== "critical" ? "minor" : ""}">${esc(r.severity)}</span>
          <span>${esc(r.component)}</span></div>
        <div class="n">${em(r.note)}</div>
      </div>`).join("")}</div>` : ""}

    <h3>项目硬红线 —— 顺序不可调换</h3>
    <div class="card danger">${w.hard_rules.map((h, i) =>
      `<div class="rule"><span class="i">${i + 1}</span><span>${em(h.rule)}</span></div>`).join("")}</div>

    <h3>${esc(w.current_phase)} ${esc(w.current_name)} —— ${esc(w.current_summary)}</h3>
    ${w.gates.length ? `<div class="card gate">${w.gates.map(g =>
      `<div class="rule"><span class="i">${esc(g.level)}</span><span>${em(g.rule)}</span></div>`).join("")}</div>` : ""}

    ${w.phase_locked ? `<div class="card danger"><div class="rule">
      <span class="i">⛔</span><span>${em(w.phase_lock_reason)}</span></div></div>` : `
    <div class="card">${w.tasks.map(t => `
      <div class="task ${t.name.includes("⚠️") ? "warn" : ""}">
        <span class="box">${t.done ? "☑" : "☐"}</span>
        <div><div class="t">${esc(t.name)}</div>
          ${t.detail ? `<div class="d">${em(t.detail)}</div>` : ""}
          ${t.hint ? `<div class="d">要点：${em(t.hint)}</div>` : ""}
          ${t.source ? `<div class="src">来源：${esc(t.source)}</div>` : ""}
          ${(t.specs||[]).map(sp => `<div class="spec spec-${sp.status.startsWith("已核实")?"ok":sp.status.startsWith("待定")?"conflict":"missing"}">
            <span class="sl">${esc(sp.label)}</span>
            <span class="sv">${esc(sp.value)}</span>
            <span class="sstat">${esc(sp.status)}</span>
            <span class="ssrc">${esc(sp.source)}</span></div>`).join("")}</div>
      </div>`).join("")}</div>`}

    ${w.warnings.length ? `<h3>待办提示</h3>
    <div class="card warn-list">${w.warnings.map(x => `<div>· ${em(x)}</div>`).join("")}</div>` : ""}`;
}


// ── 图纸（电路图）──
type CircLead = { model: string; model_codes: string[]; year_from: number; year_to: number;
  pages: number[]; book_pages: string[]; systems: string[]; applicable: boolean;
  detail: [number, string][]; images: string[]; note?: string };
type CircReport = { vehicle_id: string; model: string; year: number;
  groups: CircLead[]; filtered_out: number; notes: string[] };


// ── 脚本体检（E6）──
type SClaim = { line_no: number; text: string; context: string; state: string; matched?: string };
type SRep = { file: string; title: string; claims: SClaim[]; verified: number; unsourced: number; conflicted: number };
type ScriptReport = { dir: string; scripts: SRep[]; notes: string[] };

async function renderScriptCheck(host: HTMLElement) {
  let r: ScriptReport;
  try { r = await call<ScriptReport>("script_check", {}); }
  catch { host.innerHTML = `<div class="dimline">脚本体检读不到</div>`; return; }
  const row = (c: SClaim) =>
    c.state === "conflicted"
      ? `<div class="cv cv-bad"><span class="k">第${c.line_no}行</span>
           <span class="v">${esc(c.text)}</span>
           <span class="badge">🔴 冲突规格·不能上镜</span></div>`
      : c.state === "unsourced"
      ? `<div class="cv cv-single"><span class="k">第${c.line_no}行</span>
           <span class="v">${esc(c.text)}</span>
           <span class="badge">⚠️ 说不出出处</span></div>`
      : `<div class="cv"><span class="k">第${c.line_no}行 ${esc(c.text)}</span>
           <span class="v dimline">库中同值项 → ${esc(c.matched || "")}</span>
           <span class="badge">◻️ 核对是不是同一个件</span></div>`;
  host.innerHTML = `
    <h3>脚本体检</h3>
    <div class="dimline">作业单写错一个扭力值错的是一个人；<b>视频里念错会被观众照抄</b>，
      而没有人会回来核对。「库中有同值项」<b>不等于已核实</b>。</div>
    ${r.notes.map(n => `<div class="card warn-list"><div>${em(n)}</div></div>`).join("")}
    ${r.scripts.map(s => `<div class="card">
      <div class="t">${s.conflicted ? "🔴" : s.unsourced ? "⚠️" : "✅"} ${esc(s.title)}</div>
      <div class="d">技术数字 ${s.claims.length} 个：库中有同值项 ${s.verified} ·
        <b>说不出出处 ${s.unsourced}</b> · 冲突 ${s.conflicted}</div>
      ${s.claims.map(row).join("")}
    </div>`).join("") || `<div class="card">脚本目录里还没有脚本</div>`}`;
}

// ── 资料获取计划 ──
type ABook = { title: string; publisher: string; year?: number; isbn?: string;
  owned: boolean; names_model: boolean; covers: string; years?: [number, number];
  ebook_access: string; buy: { what: string; url: string }[] };
type AcqReport = { make: string; model: string; year: number; verdict: string;
  books: ABook[]; leads: { what: string; url: string; source_level: string }[];
  diagrams: { what: string; url: string; source_level: string }[]; notes: string[] };

async function renderAcquire(host: HTMLElement) {
  host.innerHTML = `<div class="dimline">查书目中（要打外网，慢）…</div>`;
  let r: AcqReport;
  try { r = await call<AcqReport>("acquire", {}); }
  catch (e) { host.innerHTML = `<div class="dimline">资料获取计划读不到：${esc(String(e))}</div>`; return; }
  const bk = (b: ABook) => `<div class="cv ${b.covers === "not_covered" ? "cv-bad" : b.owned ? "" : "cv-single"}">
      <span class="k">${b.owned ? "✓" : b.covers === "not_covered" ? "✗" : "·"} ${esc(b.title)}</span>
      <span class="v dimline">${esc(b.publisher)}${b.isbn ? " · ISBN " + esc(b.isbn) : ""}${
        b.years ? " · 适用 " + b.years[0] + "–" + b.years[1] : ""}</span>
      <span class="badge">${b.owned ? "库里已有" : b.covers === "not_covered" ? "年份不覆盖·不要买" : ""}</span>
    </div>`;
  host.innerHTML = `
    <h3>资料获取计划</h3>
    <div class="card"><div class="t">结论</div><div class="d">${em(r.verdict)}</div></div>
    ${r.books.length ? `<div class="card">${r.books.map(bk).join("")}</div>` : ""}
    ${r.notes.map(n => `<div class="card warn-list"><div>${em(n)}</div></div>`).join("")}
    <div class="card"><div class="t">图纸来源（实测过的直达入口）</div>
      ${r.diagrams.map(l => `<div class="d">[${esc(l.source_level)}] ${esc(l.what)}<br>
        <span class="dimline">${esc(l.url)}</span></div>`).join("")}</div>
    <div class="dimline">本页<b>不下载任何文件</b>：网上抓来的是 C 级而作业单只吃 A/B，
      且权利不明的扫描件不可下载。</div>`;
}

async function viewCircuit(el: HTMLElement) {
  el.innerHTML = `<div class="dim">读取电路图索引…</div>`;
  let r: CircReport;
  try { r = await call<CircReport>("circuit", {}); }
  catch (e) { el.innerHTML = `<div class="card err">读不到：${esc(String(e))}</div>`; return; }

  const grp = (g: CircLead) => `
    <div class="card ${g.applicable ? "" : "warn"}">
      <div class="t">${g.applicable ? "✅ 适用本车" : "⚠️ 不适用本车"} ·
        ${esc(g.model)} <span class="dimline">[${g.model_codes.map(esc).join(" / ")}]
        ${g.year_from}–${g.year_to >= 9999 ? "…" : g.year_to}</span></div>
      <div class="d">系统：${g.systems.map(esc).join("、")}</div>
      ${g.pages.map((p, i) => {
        const d = (g.detail || []).find(x => x[0] === p);
        const img = g.images[i];
        return `<div class="pg">
          <div class="pn">PDF p${p} <span class="qty">书页 ${esc(g.book_pages[i] || "")}</span></div>
          ${d ? `<div class="d">${esc(d[1])}</div>` : ""}
          ${img ? `<img class="wd" src="${imgSrc(img)}" alt="p${p}" loading="lazy">` : ""}
        </div>`;
      }).join("")}
      ${g.note ? `<div class="src">${em(g.note)}</div>` : ""}
    </div>`;

  el.innerHTML = `
    <h3>电路图 · ${r.year} ${esc(r.model)}</h3>
    <div class="dimline">图是横向印在竖页上的，点开后转 90° 看。
      线色 / 端子号 / 线径一律以图为准 —— 索引不复述任何数值。</div>
    ${r.groups.map(grp).join("") || `<div class="card">没有匹配的电路图组</div>`}
    ${r.filtered_out > 0 ? `<div class="card warn-list"><div>· 另有 <b>${r.filtered_out} 组</b>
      属于别的车系/年款，已按适用性挡住。<b>电路图按年份分档</b> ——
      照别的年款的图去背刺插头、量电压，量的是另一代车的线。</div></div>` : ""}
    ${r.notes.map(n => `<div class="card warn-list"><div>${em(n)}</div></div>`).join("")}`;
}

// 页图是本地绝对路径 —— Tauri 里要转成 asset 协议，浏览器里走 HTTP 静态不可达，
// 所以浏览器模式下不显示图，只给路径（诚实优于装作能显示）。
function imgSrc(p: string): string {
  if (!inTauri) return "";
  try { return convertFileSrc(p); } catch { return ""; }
}

type CrossItem = { canonical: string; value: string; docs: string[]; corroborated: boolean; claims_only: boolean };
type CrossReport = { critical_total: number; corroborated: CrossItem[]; single_source: CrossItem[];
  all_docs: [string, number][]; notes: string[] };

/// 交叉验证挂在知识库下面：它讲的是「这些数字有多少人背书」，
/// 和规格表是同一件事的两面，分开放人不会主动去看。
async function renderCrossval(host: HTMLElement) {
  let r: CrossReport;
  try { r = await call<CrossReport>("crossval", {}); }
  catch { host.innerHTML = `<div class="dimline">交叉验证读不到（serve 没跑？）</div>`; return; }
  host.innerHTML = `
    <h3>交叉验证 · 安全关键项</h3>
    <div class="card">
      <div class="t">安全关键 ${r.critical_total} 条：已交叉验证 ${r.corroborated.length} ·
        <b>单来源 ${r.single_source.length}</b></div>
      <div class="d">全库独立来源文档：${r.all_docs.map(d => `${esc(d[0])}(${d[1]})`).join("、")}</div>
    </div>
    ${r.notes.map(n => `<div class="card warn-list"><div>${em(n)}</div></div>`).join("")}
    ${r.single_source.length ? `<div class="card">${r.single_source.slice(0, 40).map(i => `
      <div class="cv cv-single">
        <span class="k">${esc(i.canonical)}</span>
        <span class="v">${esc(i.value)}</span>
        <span class="badge">⚠️ 单来源${i.claims_only ? "·出处自称有印证" : ""}</span>
      </div>`).join("")}
      ${r.single_source.length > 40 ? `<div class="dimline">…另有 ${r.single_source.length - 40} 条</div>` : ""}
      </div>` : ""}
    <div class="dimline">单来源<b>不扣值</b>——这是披露不是拦截。
      要拦截的是冲突，那类已经从规格表里扣下了。</div>`;
}

async function viewKnowledge(el: HTMLElement) {
  el.innerHTML = `
    <h3>规格 —— 只返回 A/B 级且已核实</h3>
    <div class="searchbar"><input id="sq" placeholder="规格名，如 valve、spark、oil、torque" value="${esc(specQ)}"/>
      <button id="sgo">查</button></div>
    <div id="sres" class="card"><div class="dimline">输入关键词开始查。查不到就是查不到 —— 不会用别处的数字代替。</div></div>

    <h3>手册全文 —— 自动排除不适用本车的资料</h3>
    <div class="searchbar"><input id="mq" placeholder="英文关键词，如 valve clearance、torque" value="${esc(manQ)}"/>
      <button id="mgo">检索</button></div>
    <div id="mres" class="card"><div class="dimline">命中后回原图核对数字 —— 页码即 PDF 页码。</div></div>

    <div id="cvbox"><div class="dimline">交叉验证载入中…</div></div>
    <div id="aqbox"></div>`;

  // 交叉验证异步补上 —— 它要跑一遍全库，不该拖慢规格检索的首屏
  const cvbox = el.querySelector<HTMLElement>("#cvbox");
  if (cvbox) renderCrossval(cvbox);
  const aqbox = el.querySelector<HTMLElement>("#aqbox");
  if (aqbox) renderAcquire(aqbox);

  const doSpec = async () => {
    specQ = (el.querySelector<HTMLInputElement>("#sq")!).value.trim();
    const box = el.querySelector<HTMLElement>("#sres")!;
    if (!specQ) return;
    box.innerHTML = `<div class="dimline">查询中…</div>`;
    try {
      const r = await call<SpecResult>("specs", { vehicle: vehicleId, q: specQ });
      box.innerHTML = r.specs.length
        ? r.specs.map(s => `<div class="spec">
            <div class="k">${esc(s.item)}</div>
            <div class="v">${esc(s.value)}</div>
            <div class="b">${badge(s.source_level, s.source_ref, s.page)}</div></div>`).join("")
          + (r.pending_related ? `<div class="dimline">另有 ${r.pending_related} 条相关项待核实，未列出 —— 它们不可用于作业</div>` : "")
        : `<div class="nores">没有「${esc(specQ)}」的已核实规格。${r.pending_related
            ? `<b>有 ${r.pending_related} 条相关项待核实</b>（C/D 级，不可用于作业）—— 不要用其他来源的数字代替。` : ""}</div>`;
    } catch (e) { box.innerHTML = errBox(e); }
  };
  const doMan = async () => {
    manQ = (el.querySelector<HTMLInputElement>("#mq")!).value.trim();
    const box = el.querySelector<HTMLElement>("#mres")!;
    if (!manQ) return;
    box.innerHTML = `<div class="dimline">检索中…</div>`;
    try {
      const r = await call<SearchResult>("search", { vehicle: vehicleId, q: manQ, limit: 8 });
      box.innerHTML = (r.hits.length
        ? r.hits.map(h => `<div class="hit">
            <div class="b"><span class="badge lv${esc(h.tier ?? "?")}">${esc(h.tier ?? "?")}级</span>
              <span class="ref">${esc(h.manual)} · p${h.page}</span></div>
            <div class="sn">${esc(h.snippet).replace(/\[(.+?)\]/g, "<mark>$1</mark>")}</div></div>`).join("")
        : `<div class="nores">无结果</div>`)
        + (r.excluded_manuals ? `<div class="dimline">🛡 已自动排除 ${r.excluded_manuals} 份不适用本车的资料 —— 它们不可能出现在上面。数字请回原图核对。</div>` : "");
    } catch (e) { box.innerHTML = errBox(e); }
  };
  el.querySelector<HTMLButtonElement>("#sgo")!.onclick = doSpec;
  el.querySelector<HTMLButtonElement>("#mgo")!.onclick = doMan;
  el.querySelector<HTMLInputElement>("#sq")!.onkeydown = e => { if (e.key === "Enter") doSpec(); };
  el.querySelector<HTMLInputElement>("#mq")!.onkeydown = e => { if (e.key === "Enter") doMan(); };
  if (specQ) doSpec();
  if (manQ) doMan();
}

async function viewParts(el: HTMLElement) {
  el.innerHTML = `<div class="dimline">加载中…</div>`;
  let p: PartsPlan;
  try { p = await call<PartsPlan>("parts", { vehicle: vehicleId }); }
  catch (e) { el.innerHTML = errBox(e); return; }

  const groups: [string, string][] = [
    ["立即", "🔴 现在就下单 —— 再等就卡住关键路径"],
    ["本周", "🟡 本周内下单"],
    ["待拍板", "🤔 等你拍板 —— 规格是确定的，选型是你的偏好"],
    ["被挡", "🚧 下不了单 —— 等你先做完一件事"],
    ["在途", "🚚 已订，在途"],
    ["稍后", "⚪ 稍后"],
    ["完成", "✅ 已到 / 不需要"],
  ];
  el.innerHTML = `
    <h3>采购清单 —— 按到货周期倒排</h3>
    <div class="dimline">假设每阶段约 ${p.assumed_days_per_phase} 天（这是假设不是事实，改它会改变排序）</div>
    ${groups.map(([k, label]) => {
      const list = p.parts.filter(x => x.urgency === k);
      if (!list.length) return "";
      return `<h4>${label}</h4><div class="card">${list.map(x => `
        <div class="part">
          <div class="pn">${esc(x.name)} <span class="qty">×${x.qty}</span>
            ${x.est_cost != null ? `<span class="cost">约 $${x.est_cost}</span>` : ""}</div>
          ${x.spec ? `<div class="d">${esc(x.spec)}</div>` : ""}
          ${x.spec_ref ? `<div class="src">规格出处：${esc(x.spec_ref)}</div>`
            : x.spec ? `<div class="src bad">⚠️ 有规格但无出处 —— 不要据此下单</div>` : ""}
          <div class="meta">${esc(x.needed_phase)} 用 · 到货约 ${x.lead_time_days} 天${
            (k === "立即" || k === "本周") ? ` · 最晚 ${Math.max(0, x.order_within_days)} 天内下单` : ""}</div>
          ${x.needs_choice ? `<div class="d choice">🤔 ${em(x.needs_choice)}</div>` : ""}
          ${x.blocked_by ? `<div class="d blocked">🚧 ${em(x.blocked_by)}</div>` : ""}
        </div>`).join("")}</div>`;
    }).join("")}
    ${p.blockers.length ? `<h3>只有你能做的事 —— 挡住了上面的采购</h3>
    <div class="card gate">${p.blockers.map(b => `
      <div class="task"><span class="box">☐</span><div>
        <div class="t">[${esc(b.phase)}] ${esc(b.what)}</div>
        <div class="d">${em(b.why)}</div>
        ${b.unblocks.length ? `<div class="src">做完解锁：${b.unblocks.map(esc).join("、")}</div>` : ""}
      </div></div>`).join("")}</div>` : ""}
    <div class="dimline">已知价格合计约 $${Math.round(p.est_total)}；另有 ${p.unknown_cost_count} 项价格未知（多数是件号还没查到）。</div>`;
}

async function viewLog(el: HTMLElement) {
  el.innerHTML = `<div class="dimline">加载中…</div>`;
  try {
    const r = await call<LogRead>("log", { vehicle: vehicleId, limit: 30 });
    el.innerHTML = `<h3>维修日志 —— 共 ${r.total} 条</h3>
      ${r.total === 0
        ? `<div class="card"><div class="nores">还没有任何记录。<br/>
             agent 可以通过 <code>repair.log.append</code> 写入（需幂等键），
             你也可以直接编辑文件 —— <b>文件才是真相源</b>。</div></div>`
        : r.entries.map(e => `<div class="card logentry">${em(e)}</div>`).join("")}
      <div class="dimline">${esc(r.file)}</div>`;
  } catch (e) { el.innerHTML = errBox(e); }
}


type Cause = { item:string; level:string; complaints:number; matched_symptoms:string[]; note:string; diag_tree:string|null };
type SymptomRes = { causes:Cause[]; note:string };
type DStep = { check:string; expect:string; basis:string; resolved_value:string|null; value_status:string;
  if_pass:string|null; if_fail:string|null; conclusion_on_fail:string|null; note:string|null; gated:boolean };
type DTree = { id:string; symptom:string; severity:string; applies:boolean; applies_note:string; steps:DStep[]; cross_check:string[] };

let symptomQ = "";
async function viewDiag(el: HTMLElement) {
  el.innerHTML = `
    <h3>症状 → 原因 → 排查</h3>
    <div class="dimline">说出你观察到的现象（打不着 / 后轮异响 / 燃油表乱跳 / 渗油），
      系统按本车真实故障频率排出候选，再带你逐步定位。</div>
    <div class="searchbar">
      <input id="sq" placeholder="观察到的现象，如「后轮有异响」" value="${esc(symptomQ)}"/>
      <button id="sgo">诊断</button></div>
    <div id="sres"></div>`;
  const run = async () => {
    symptomQ = (el.querySelector<HTMLInputElement>("#sq")!).value.trim();
    const box = el.querySelector<HTMLElement>("#sres")!;
    if (!symptomQ) return;
    box.innerHTML = `<div class="dimline">分析中…</div>`;
    try {
      const r = await call<SymptomRes>("symptom", { vehicle: vehicleId, text: symptomQ });
      if (!r.causes.length) { box.innerHTML = `<div class="card"><div class="nores">${em(r.note)}</div></div>`; return; }
      box.innerHTML = `<div class="card">${r.causes.map((c,i)=>`
        <div class="cause">
          <div><span class="rank">${i+1}.</span>
            <span class="badge lv${esc(c.level)}">${esc(c.level)}级</span>
            <b>${esc(c.item)}</b><span class="freq">· NHTSA ${c.complaints} 条投诉</span></div>
          <div class="sym">命中：${c.matched_symptoms.map(esc).join("、")}</div>
          ${c.note?`<div class="task"><div class="d">${em(c.note)}</div></div>`:""}
          ${c.diag_tree?`<span class="tree" data-tree="${esc(c.diag_tree)}">▸ 打开排查树：${esc(c.diag_tree)}</span>`:
            `<div class="dimline">暂无专门排查树</div>`}
        </div>`).join("")}</div>
        <div class="dimline">${em(r.note)}</div>
        <div id="tree"></div>`;
      box.querySelectorAll<HTMLElement>(".tree").forEach(t =>
        t.onclick = () => openTree(t.dataset.tree!, box.querySelector<HTMLElement>("#tree")!));
    } catch(e){ box.innerHTML = errBox(e); }
  };
  el.querySelector<HTMLButtonElement>("#sgo")!.onclick = run;
  el.querySelector<HTMLInputElement>("#sq")!.onkeydown = e => { if(e.key==="Enter") run(); };
  if (symptomQ) run();
}

async function openTree(id: string, box: HTMLElement) {
  box.innerHTML = `<div class="dimline">加载排查树…</div>`;
  try {
    const t = await call<DTree>(inTauri?"diag_tree":"diag", { vehicle: vehicleId, tree: id });
    box.innerHTML = `<h3>排查树 · ${esc(t.symptom)}</h3>
      ${!t.applies?`<div class="card danger"><div class="d">${em(t.applies_note)}</div></div>`:""}
      ${t.steps.map((s,i)=>`
        <div class="step">
          <div class="chk">步骤 ${i+1}：${esc(s.check)}${s.gated?' <span class="gated">⚠️需过安全门禁</span>':''}</div>
          <div class="exp ${s.value_status==="冲突-已扣下"?"conflict":""}">判据：${esc(s.expect)}
            ${s.value_status==="已核实"&&s.resolved_value?`<span class="val">（已核实：${esc(s.resolved_value)}）</span>`:""}
            ${s.value_status==="冲突-已扣下"?`⚠️ 依赖的规格有冲突、已扣下 — 先解决冲突`:""}
            ${s.value_status==="缺失"?`⚠️ 依赖的规格还没核实`:""}</div>
          <div class="basis">出处：${esc(s.basis)}</div>
          ${s.if_pass?`<div class="branch">✓ 通过：${esc(s.if_pass)}</div>`:""}
          ${s.if_fail?`<div class="branch">✗ 不通过：${esc(s.if_fail)}</div>`:""}
          ${s.conclusion_on_fail?`<div class="branch">→ 结论：${esc(s.conclusion_on_fail)}</div>`:""}
        </div>`).join("")}
      ${t.cross_check.map(c=>`<div class="dimline">⚠️ ${em(c)}</div>`).join("")}`;
    box.scrollIntoView({behavior:"smooth",block:"start"});
  } catch(e){ box.innerHTML = errBox(e); }
}

async function viewVlog(el: HTMLElement) {
  el.innerHTML = `<div class="dimline">加载中…</div>`;
  let v: VlogView;
  try { v = await call<VlogView>("vlog", { vehicle: vehicleId }); }
  catch (e) { el.innerHTML = errBox(e); return; }

  if (!v.exists) {
    el.innerHTML = `<div class="card"><div class="nores">
      Vlog 目录不存在：<code>${esc(v.vlog_path)}</code><br/>
      路径来自 VEHICLE.yml 的 <code>vlog_path</code>。</div></div>`;
    return;
  }
  const byPhase = new Map<string, Topic[]>();
  v.topics.forEach(t => { const k = t.phase || "其他"; byPhase.set(k, [...(byPhase.get(k) ?? []), t]); });

  el.innerHTML = `
    <h3>选题库 —— ${v.topics.length} 条，其中 <b>${v.one_shot_count}</b> 条是一次性素材</h3>
    <div class="dimline">🔴 = 一次性：那个镜头拍不到就永远没有了（拆过一次就回不去）</div>
    ${[...byPhase].map(([ph, list]) => `
      <h4>${esc(ph)}</h4>
      <div class="card">${list.map(t => `
        <div class="topic ${t.one_shot ? "oneshot" : ""}">
          <span class="box">${t.one_shot ? "🔴" : "○"}</span>
          <div><span class="sid">${esc(t.id)}</span> ${esc(t.title)}</div>
        </div>`).join("")}</div>`).join("")}

    <h3>技术图解动画</h3>
    <div class="card">${v.animations.length
      ? v.animations.map(a => `<div class="part"><div class="pn">${esc(a.name)}
          <span class="cost">${mb(a.bytes)}</span></div>
          <div class="src">${esc(a.path)}</div></div>`).join("")
      : `<div class="nores">还没有成品动画</div>`}</div>

    <h3>素材与文档</h3>
    <div class="card">
      <div class="part"><div class="pn">素材库</div>
        <div class="d">已打标签 <b>${v.tagged_media}</b> 张</div></div>
      <div class="part"><div class="pn">规范文档</div>
        <div class="d">${v.docs.length ? v.docs.map(esc).join("、") : "—"}</div></div>
      <div class="part"><div class="pn">脚本</div>
        <div class="d">${v.scripts.length ? v.scripts.map(esc).join("、") : "— 还是空的 —"}</div></div>
    </div>
    ${v.notes.length ? `<div class="card warn-list">${v.notes.map(n => `<div>· ${esc(n)}</div>`).join("")}</div>` : ""}
    <div class="dimline">Vlog 数据留在纪录片项目里，平台只读不写：${esc(v.vlog_path)}</div>`;

  const sc = document.createElement("div");
  el.appendChild(sc);
  renderScriptCheck(sc);
}

window.addEventListener("DOMContentLoaded", () => {
  const played = (() => { try { return sessionStorage.getItem("introPlayed"); } catch { return null; } })();
  if (played) { render(); return; }
  try { sessionStorage.setItem("introPlayed", "1"); } catch {}
  runIntro(document.body, render);
});
