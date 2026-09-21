/// <reference types="vite/client" />
// 开场动画:GS 线稿蓝图 → 逐笔发光绘出 → 爆炸分解 → 溶入 App
// 视觉语言取自项目参考图:纯黑底、电蓝/靛紫发光丝线、辉光、粒子。
// 点击任意处可跳过。只播一次(sessionStorage 记住)。

function wheel(cx: number, cy: number, r: number, spokes = 12): string {
  let s = `<circle class="ln" pathLength="1" cx="${cx}" cy="${cy}" r="${r}"/>`;
  s += `<circle class="ln" pathLength="1" cx="${cx}" cy="${cy}" r="${r - 14}"/>`;
  s += `<circle class="ln hub" pathLength="1" cx="${cx}" cy="${cy}" r="${r * 0.22}"/>`;
  for (let i = 0; i < spokes; i++) {
    const a = (i / spokes) * Math.PI * 2;
    const x1 = cx + Math.cos(a) * (r * 0.22), y1 = cy + Math.sin(a) * (r * 0.22);
    const x2 = cx + Math.cos(a) * (r - 14), y2 = cy + Math.sin(a) * (r - 14);
    s += `<line class="ln spk" pathLength="1" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}"/>`;
  }
  return s;
}

// 每个部件一个 <g>,带爆炸方向向量(--tx --ty)
function bikeSVG(): string {
  // 侧视 ADV 摩托(GS 神态):大前轮 + 高车身 + 高风挡 + 鸟嘴 + boxer 探出缸。
  // 轮距收紧,重心抬高。每个 <g> 一个部件,带爆炸向量。
  const rw = 250, fw = 720, wy = 400, R = 104;   // 后/前轮心 x、轮心 y、半径
  return `
  <svg id="bike" viewBox="0 0 1000 560" preserveAspectRatio="xMidYMid meet">
    <defs>
      <filter id="glow" x="-60%" y="-60%" width="220%" height="220%">
        <feGaussianBlur stdDeviation="3" result="b"/>
        <feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
      </filter>
      <linearGradient id="grad" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#6b9bff"/>
        <stop offset="0.55" stop-color="#4a7dff"/>
        <stop offset="1" stop-color="#9b7bff"/>
      </linearGradient>
    </defs>
    <g filter="url(#glow)" stroke="url(#grad)" fill="none" stroke-width="2.4"
       stroke-linecap="round" stroke-linejoin="round">

      <g class="part" style="--tx:-240px;--ty:150px">${wheel(rw, wy, R, 6)}</g>
      <g class="part" style="--tx:260px;--ty:150px">${wheel(fw, wy, R, 6)}</g>

      <!-- 发动机:boxer 缸,左右各探出一块带散热片的缸头(GS 招牌) -->
      <g class="part" style="--tx:0px;--ty:240px">
        <path class="ln" pathLength="1" d="M420 360 L520 360 L520 420 L420 420 Z"/>
        <path class="ln" pathLength="1" d="M420 372 L360 366 L358 410 L420 414 Z"/>  <!-- 左探出缸头 -->
        <line class="ln" pathLength="1" x1="416" y1="380" x2="364" y2="376"/>
        <line class="ln" pathLength="1" x1="416" y1="392" x2="362" y2="388"/>
        <line class="ln" pathLength="1" x1="416" y1="404" x2="360" y2="400"/>
        <line class="ln" pathLength="1" x1="420" y1="410" x2="${rw+40}" y2="${wy}"/> <!-- 传动轴→后轮 -->
      </g>

      <!-- 车架 + 油箱(高耸) + 前叉 telelever -->
      <g class="part" style="--tx:-30px;--ty:-260px">
        <path class="ln" pathLength="1" d="M478 322 Q505 268 588 274 Q616 280 600 322 Z"/> <!-- 油箱 -->
        <line class="ln" pathLength="1" x1="478" y1="322" x2="470" y2="360"/>  <!-- 接发动机 -->
        <line class="ln" pathLength="1" x1="600" y1="322" x2="560" y2="360"/>
        <line class="ln" pathLength="1" x1="600" y1="300" x2="${fw-24}" y2="${wy-40}"/> <!-- 前叉到前轮 -->
        <line class="ln" pathLength="1" x1="520" y1="360" x2="${fw-30}" y2="${wy}"/>   <!-- 底梁 -->
      </g>

      <!-- 座垫 + 尾箱(ADV 高尾) -->
      <g class="part" style="--tx:-230px;--ty:-170px">
        <path class="ln" pathLength="1" d="M336 316 Q404 300 478 320 L476 332 Q404 314 336 330 Z"/>
        <path class="ln" pathLength="1" d="M336 318 L292 322 L286 300 L328 304 Z"/> <!-- 尾箱 -->
        <line class="ln" pathLength="1" x1="336" y1="330" x2="${rw+30}" y2="${wy-40}"/> <!-- 尾架到后轮 -->
      </g>

      <!-- 前部:风挡(高) + 鸟嘴 + 大灯 + 手把 -->
      <g class="part" style="--tx:270px;--ty:-230px">
        <path class="ln" pathLength="1" d="M640 300 Q655 200 715 176"/>            <!-- 高风挡 -->
        <path class="ln" pathLength="1" d="M640 300 Q612 302 604 320 L636 316 Z"/> <!-- 大灯 -->
        <path class="ln beak" pathLength="1" d="M${fw+26} ${wy-96} Q822 ${wy-112} 878 ${wy-78} L856 ${wy-64} Q808 ${wy-90} ${fw+38} ${wy-82} Z"/> <!-- 鸟嘴 -->
        <line class="ln" pathLength="1" x1="648" y1="220" x2="590" y2="206"/>      <!-- 手把 -->
        <line class="ln" pathLength="1" x1="640" y1="300" x2="596" y2="300"/>
      </g>
    </g>
  </svg>`;
}


// 用户渲染的 GS 图(放 src/assets/gs-intro.png)。Vite 用 import.meta.glob 探测,
// 有图走高清碎裂爆炸,无图回退 SVG 线稿。
const gsImages = import.meta.glob("./assets/gs-intro.{png,jpg,jpeg,webp}", {
  eager: true, query: "?url", import: "default",
}) as Record<string, string>;
const gsUrl: string | undefined = Object.values(gsImages)[0];

/** 把一张图碎裂成网格碎片,各自沿径向飞散 —— 对任何位图都成立的"爆炸分解" */
function shatter(img: HTMLImageElement, host: HTMLElement, onBoom: () => void) {
  const COLS = 14, ROWS = 9;
  const W = Math.min(window.innerWidth * 0.8, 1000);
  const H = W * (img.naturalHeight / img.naturalWidth);
  const wrap = document.createElement("div");
  wrap.className = "shatter";
  wrap.style.width = W + "px"; wrap.style.height = H + "px";
  const cw = W / COLS, ch = H / ROWS;
  for (let r = 0; r < ROWS; r++) for (let c = 0; c < COLS; c++) {
    const f = document.createElement("div");
    f.className = "frag";
    f.style.cssText = `width:${cw}px;height:${ch}px;left:${c*cw}px;top:${r*ch}px;
      background-image:url(${img.src});background-size:${W}px ${H}px;
      background-position:-${c*cw}px -${r*ch}px`;
    // 径向向量:从中心指向该片,越外飞越远
    const dx = (c + .5)/COLS - .5, dy = (r + .5)/ROWS - .5;
    const mag = 260 + Math.random()*180;
    f.style.setProperty("--fx", dx*mag + "px");
    f.style.setProperty("--fy", dy*mag*0.8 - 40 + "px");
    f.style.setProperty("--fr", (Math.random()*60-30) + "deg");
    f.style.transitionDelay = (Math.abs(dx)+Math.abs(dy))*0.12 + "s";
    wrap.appendChild(f);
  }
  host.appendChild(wrap);
  // 触发:draw-in(整图辉光浮现)→ boom(碎裂)
  requestAnimationFrame(() => wrap.classList.add("in"));
  setTimeout(() => { wrap.classList.add("boom"); onBoom(); }, 1600);
}

export function runIntro(mount: HTMLElement, onDone: () => void) {
  let skipped = false;
  const done = () => { if (skipped) return; skipped = true; ov.remove(); onDone(); };

  const ov = document.createElement("div");
  ov.id = "intro";
  ov.innerHTML = `
    <div class="grid-floor"></div>
    <div class="stage"></div>
    <div class="spark-layer"></div>
    <div class="intro-title">
      <div class="zh">你的手册 · 你的 AI</div>
      <div class="en">motomech AI</div>
    </div>
    <div class="skip">点击任意处跳过 / tap to skip</div>`;
  mount.appendChild(ov);
  const stage = ov.querySelector(".stage") as HTMLElement;

  if (gsUrl) {
    // 高清图路径:载入 → 碎裂
    const img = new Image();
    img.onload = () => shatter(img, stage, () => ov.classList.add("boom"));
    img.onerror = () => { stage.innerHTML = bikeSVG(); ov.classList.add("draw"); };
    img.src = gsUrl;
  } else {
    stage.innerHTML = bikeSVG();
    requestAnimationFrame(() => ov.classList.add("draw"));
  }

  // 火花粒子
  const spark = ov.querySelector(".spark-layer")!;
  for (let i = 0; i < 40; i++) {
    const p = document.createElement("i");
    p.style.left = (30 + Math.random() * 40) + "%";
    p.style.top = (40 + Math.random() * 25) + "%";
    p.style.setProperty("--dx", (Math.random() * 2 - 1) * 260 + "px");
    p.style.setProperty("--dy", (Math.random() * 2 - 1) * 200 + "px");
    p.style.animationDelay = (1.8 + Math.random() * 0.5) + "s";
    spark.appendChild(p);
  }

  ov.onclick = done;

  // 时间线:标题 → 爆炸 → 溶出(图片路径的 boom 由 shatter 内部触发,这里管标题/收尾)
  setTimeout(() => ov.classList.add("show-title"), 1700);
  setTimeout(() => ov.classList.add("boom"), 2050);
  setTimeout(() => ov.classList.add("fade"), 3300);
  setTimeout(done, 3900);
}
