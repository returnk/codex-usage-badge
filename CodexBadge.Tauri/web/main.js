const view = new URLSearchParams(location.search).get('view') || 'capsule';
document.body.className = `view-${view}`;
const app = document.getElementById('app');
const invoke = (command, args = {}) => window.__TAURI__.core.invoke(command, args);
let themeChoice = 'glass';
let topmost = false;
let menuRequestId = 0;
let rendering = false;
let renderAgain = false;
let readyReported = false;
let readyQueued = false;
function sendReady() {
  if (readyReported || readyQueued) return;
  readyQueued = true;
  settled(async () => {
    try {
      readyReported = await invoke('window_ready') === true;
      readyQueued = false;
      if (!readyReported) sendReady();
    } catch (error) { readyQueued=false; console.error(error); }
  });
}
let layoutQueued = false;
function settled(callback) { requestAnimationFrame(() => requestAnimationFrame(callback)); }
function reportLayout() {
  if (view !== 'capsule' || layoutQueued) return;
  layoutQueued = true;
  settled(() => {
    layoutQueued = false;
    const frame = app.querySelector('.capsule-frame').getBoundingClientRect();
    const pill = app.querySelector('.capsule').getBoundingClientRect();
    invoke('report_capsule_layout', { frame:[frame.x,frame.y,frame.width,frame.height],
      capsule:[pill.x,pill.y,pill.width,pill.height], viewport:[innerWidth,innerHeight,devicePixelRatio] }).catch(console.error);
  });
}
window.addEventListener('resize', reportLayout);
const systemTheme = window.matchMedia('(prefers-color-scheme: dark)');
const applyTheme = () => {
  document.documentElement.dataset.theme = themeChoice === 'system' ? (systemTheme.matches ? 'dark' : 'light') : themeChoice;
};
systemTheme.addEventListener('change', applyTheme);
document.addEventListener('contextmenu', event => event.preventDefault());
document.addEventListener('mousedown', event => {
  if (event.target.closest('button')) event.preventDefault();
});

if (view === 'capsule') {
  app.innerHTML = '<div class="capsule-frame"><div class="capsule"><span class="percentage"><span id="percent">--</span><span class="percent-sign">%</span></span></div></div>';
  const capsule = app.querySelector('.capsule');
  capsule.addEventListener('contextmenu', event => {
    event.preventDefault();
    invoke('open_capsule_menu');
  });
  capsule.addEventListener('wheel', event => {
    event.preventDefault();
    invoke('cycle_theme', { delta: Math.sign(event.deltaY) * -1 });
  }, { passive: false });
  capsule.addEventListener('dblclick', () => { if (!topmost) invoke('reset_position', {fromCapsule:true}); });
  capsule.addEventListener('pointerdown', event => {
    event.preventDefault();
    if (event.button !== 0 || event.detail > 1) return;
    capsule.setPointerCapture(event.pointerId);
    invoke('start_drag');
  });
  capsule.addEventListener('pointerup', () => invoke('stop_drag'));
  capsule.addEventListener('lostpointercapture', () => invoke('stop_drag'));
} else if (view === 'detail') {
  app.innerHTML = `<div class="card detail-card">
    <div class="top"><span>5 小时 <small id="five-note"></small></span><strong id="five-percent">--%</strong></div>
    <div class="track"><div id="fill" class="fill"></div></div>
    <div id="five-reset" class="reset muted">暂时无法读取额度</div>
    <div class="row weekly"><span>本周剩余</span><strong id="weekly-percent">--%</strong></div>
    <div class="row weekly-reset"><span id="weekly-reset" class="muted">重置时间未知</span><span id="credit-summary"><strong id="credit-count">重置机会未知</strong><span id="credit-suffix" hidden> 次重置机会</span> <button id="credit-button" type="button" hidden>查看</button></span></div>
  </div>`;
  document.getElementById('credit-button').addEventListener('click', async () => {
    await invoke('toggle_credit');
    render();
  });
} else if (view === 'credit') {
  app.innerHTML = '<div class="card credit-card" id="credit-list"></div>';
} else {
  app.innerHTML = `<div class="menu-card">
    <button id="startup" type="button">开机启动 <span id="startup-check">✓</span></button>
    <button id="topmost" type="button">置顶模式 <span id="topmost-check">✓</span></button>
    <button id="relocate" type="button">重新定位</button>
    <button id="exit" type="button">退出</button>
  </div>`;
  const menuAction = (id, command) => document.getElementById(id).addEventListener('click', async () => {
    const requestId = menuRequestId;
    try { await invoke(command); } finally { await invoke('hide_menu', { requestId }); }
  });
  menuAction('startup', 'toggle_startup');
  menuAction('topmost', 'toggle_topmost');
  menuAction('relocate', 'reset_position');
  document.getElementById('exit').addEventListener('click', () => invoke('exit_app'));
}

function percent(value) { return value == null ? '--%' : `${Math.round(value)}%`; }
function expiry(epoch) {
  const date = new Date(epoch * 1000);
  return `${date.getMonth() + 1}/${date.getDate()} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')} 到期`;
}

async function renderState() {
  const state = await invoke('get_state');
  topmost = state.topmost;
  menuRequestId = state.menuRequestId;
  themeChoice = state.theme;
  applyTheme();
  if (view === 'capsule') {
    if (state.defaultCursor) app.querySelector('.capsule').style.cursor = 'default';
    document.getElementById('percent').textContent = state.capsule.replace(/%$/, '');
  } else if (view === 'detail') {
    document.getElementById('five-percent').textContent = percent(state.fiveHour);
    document.getElementById('five-note').textContent = state.weeklyExhausted ? '（暂不可用）' : '';
    document.getElementById('five-reset').textContent = state.weeklyExhausted
      ? '本周额度已用完，5小时额度暂不可用'
      : state.freshness === 'unavailable' ? (state.quotaStatus || '暂时无法读取额度')
      : `${state.fiveReset}${state.freshness === 'stale' ? ' · 数据待更新' : ''}`;
    document.getElementById('five-reset').classList.toggle('warning', state.weeklyExhausted);
    const fill = document.getElementById('fill');
    fill.style.width = `${state.fiveHour ?? 0}%`;
    fill.dataset.band = state.weeklyExhausted ? 'blocked' : state.progressBand;
    document.getElementById('weekly-percent').textContent = percent(state.weekly);
    document.getElementById('weekly-reset').textContent = state.weekReset;
    document.getElementById('credit-count').textContent = state.creditCount == null ? '重置机会未知' : state.creditCount;
    document.getElementById('credit-suffix').hidden = state.creditCount == null;
    const button = document.getElementById('credit-button');
    button.hidden = state.credits.length === 0;
    button.textContent = state.creditOpen ? '收起' : '查看';
  } else if (view === 'credit') {
    const list = document.getElementById('credit-list');
    list.replaceChildren(...state.credits.map((credit, index) => {
      const row = document.createElement('div');
      row.className = 'credit-row';
      const label = document.createElement('span');
      label.className = 'muted';
      label.textContent = `第 ${index + 1} 次`;
      const time = document.createElement('span');
      time.textContent = expiry(credit.expiresAt);
      row.append(label, time);
      return row;
    }));
  } else {
    document.getElementById('startup-check').hidden = !state.startup;
    document.getElementById('topmost-check').hidden = !state.topmost;
  }
}

async function render() {
  renderAgain = true;
  if (rendering) return;
  rendering = true;
  try {
    do { renderAgain = false; await renderState(); } while (renderAgain);
    if (!readyReported) {
      sendReady();
      reportLayout();
    }
  } catch (error) { console.error(error); }
  finally { rendering = false; }
}
window.__TAURI__.event.listen('state-updated', render).then(render);
window.__TAURI__.event.listen('retry-ready', () => { readyReported=false; render(); });
