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
      else reportContent();
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
function reportContent() {
  settled(() => {
    const buttons = [...document.querySelectorAll('button')].filter(button => !button.hidden).map(button => {
      const r=button.getBoundingClientRect(); return [r.x,r.y,r.width,r.height];
    });
    invoke('set_input_regions', {buttons, draggable:view==='capsule'}).catch(console.error);
    if(view==='detail') {
      const card=app.querySelector('.detail-card');
      if(!card?.children?.length) return;
      const visible=[...card.children].filter(item=>!item.hidden);
      const bottom=Math.max(...visible.map(item=>item.getBoundingClientRect().bottom));
      const height=Math.ceil(bottom-card.getBoundingClientRect().top+(card.scrollTop || 0)+parseFloat(getComputedStyle(card).paddingBottom)+6);
      invoke('report_detail_height', {height}).catch(console.error);
    }
  });
}
window.addEventListener('resize', () => { reportLayout(); reportContent(); });
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
  app.innerHTML = '<div class="capsule-frame"><div class="capsule"><span class="percentage"><span id="percent">--</span><span id="capsule-unit" hidden>%</span></span><span id="hint-dot" class="hint-dot" aria-hidden="true"></span></div></div>';
  const capsule = app.querySelector('.capsule');
  capsule.addEventListener('contextmenu', event => {
    event.preventDefault();
    invoke('open_capsule_menu');
  });
  capsule.addEventListener('wheel', event => {
    event.preventDefault();
    invoke('cycle_theme', { delta: Math.sign(event.deltaY) * -1 });
  }, { passive: false });
  capsule.addEventListener('dblclick', () => invoke('reset_position', {fromCapsule:true}));
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
    <div class="top"><span>5 小时 <small id="five-note"></small></span><strong id="five-percent"><span id="five-value">--</span><small id="five-unit" class="detail-unit">%</small></strong></div>
    <div class="track"><div id="fill" class="fill"></div></div>
    <div id="five-reset" class="reset muted">暂时无法读取额度</div>
    <div class="row weekly"><span>本周剩余</span><strong id="weekly-percent"><span id="week-value">--</span><small class="detail-unit">%</small></strong></div>
    <div class="row weekly-reset"><span id="weekly-reset" class="muted">重置时间未知</span><span id="credit-summary"><strong id="credit-count">重置机会未知</strong><span id="credit-suffix" hidden> 次重置机会</span> <button id="credit-button" type="button" hidden>查看</button></span></div>
    <div id="quota-hint" class="quota-hint" hidden></div>
    <div id="last-updated" class="last-updated muted" hidden></div>
  </div>`;
  document.getElementById('credit-button').addEventListener('click', async () => {
    await invoke('toggle_credit');
    render();
  });
} else if (view === 'credit') {
  app.innerHTML = '<div class="card credit-card" id="credit-list"></div>';
} else {
  app.innerHTML = `<div class="menu-card">
    <button id="topmost" type="button">置顶模式 <span id="topmost-check">✓</span></button>
    <button id="settings" type="button">设置…</button>
    <button id="exit" type="button">退出</button>
  </div>`;
  const menuAction = (id, command) => document.getElementById(id).addEventListener('click', async () => {
    const requestId = menuRequestId;
    try { await invoke(command); } finally { await invoke('hide_menu', { requestId }); }
  });
  menuAction('settings', 'open_settings');
  menuAction('topmost', 'toggle_topmost');
  document.getElementById('exit').addEventListener('click', () => invoke('exit_app'));
}

function percent(value) { return value == null ? '--' : `${Math.round(value)}`; }
function expiry(epoch) {
  const date = new Date(epoch * 1000);
  return `${date.getMonth() + 1}/${date.getDate()} ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')} 到期`;
}

async function renderState() {
  const state = await invoke('get_state');
  topmost = state.topmost;
  document.documentElement.dataset.topmost = String(!!topmost);
  menuRequestId = state.menuRequestId;
  themeChoice = state.theme;
  applyTheme();
  if (view === 'capsule') {
    if (state.defaultCursor) app.querySelector('.capsule').style.cursor = 'default';
    document.getElementById('percent').textContent = state.capsule.replace(/%$/, '');
    document.getElementById('capsule-unit').hidden = !topmost || !/%$/.test(state.capsule);
    const hintLevel = ['notice', 'urgent'].includes(state.hintLevel) ? state.hintLevel : 'none';
    document.getElementById('hint-dot').dataset.level = hintLevel;
    app.querySelector('.capsule').setAttribute('aria-label', [`剩余额度 ${state.capsule}`, ...(state.hintMessages || [])].join('，'));
  } else if (view === 'detail') {
    document.getElementById('five-value').textContent = percent(state.fiveHour);
    document.getElementById('five-note').textContent = state.weeklyExhausted ? '（暂不可用）' : '';
    document.getElementById('five-reset').textContent = state.weeklyExhausted
      ? '本周额度已用完，5小时额度暂不可用'
      : state.freshness === 'unavailable' ? (state.quotaStatus || '暂时无法读取额度')
      : `${state.fiveReset}${state.freshness === 'stale' ? ' · 数据待更新' : ''}`;
    document.getElementById('five-reset').classList.toggle('warning', state.weeklyExhausted);
    const hint = document.getElementById('quota-hint');
    hint.textContent = (state.hintMessages || []).join('\n');
    hint.hidden = !hint.textContent;
    hint.dataset.level = state.hintLevel || 'none';
    const updated = document.getElementById('last-updated');
    updated.hidden = !state.fetchedAt || !['stale', 'unavailable'].includes(state.freshness);
    if (state.fetchedAt) {
      const date = new Date(state.fetchedAt * 1000);
      updated.textContent = `上次成功更新 ${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
    }
    const fill = document.getElementById('fill');
    fill.style.width = `${state.fiveHour ?? 0}%`;
    fill.dataset.band = state.weeklyExhausted ? 'blocked' : state.progressBand;
    document.getElementById('week-value').textContent = percent(state.weekly);
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
    document.getElementById('topmost-check').hidden = !state.topmost;
  }
}

async function render() {
  renderAgain = true;
  if (rendering) return;
  rendering = true;
  try {
    do { renderAgain = false; await renderState(); } while (renderAgain);
    reportContent();
    if (!readyReported) {
      sendReady();
      reportLayout();
    }
  } catch (error) { console.error(error); }
  finally { rendering = false; }
}
let nativePressed = null;
window.__TAURI__.event.listen(`native-input-${view}`, async ({ payload: input }) => {
  if (view === 'capsule') {
    if (input.kind === 'down') await invoke('start_drag');
    else if (input.kind === 'up' || input.kind === 'cancel') await invoke('stop_drag');
    else if (input.kind === 'double') await invoke('reset_position', { fromCapsule: true });
    else if (input.kind === 'right') await invoke('open_capsule_menu');
    else if (input.kind === 'wheel') await invoke('cycle_theme', { delta: input.delta });
    return;
  }
  const button = input.kind === 'leave' ? null : document.elementFromPoint(input.x, input.y)?.closest('button');
  if (input.kind === 'move' || input.kind === 'leave') {
    for (const item of document.querySelectorAll('button')) item.classList.toggle('native-hover', item === button);
    return;
  }
  if (input.kind === 'down') nativePressed = button || null;
  else if (input.kind === 'up') {
    if (button && button === nativePressed) button.click();
    nativePressed = null;
  } else if (input.kind === 'cancel') nativePressed = null;
  else if (input.kind === 'wheel') {
    const card = app.querySelector('.card') || app.querySelector('.menu-card');
    if (card) card.scrollTop -= input.delta;
  }
});
window.__TAURI__.event.listen('state-updated', render).then(render);
window.__TAURI__.event.listen('retry-ready', () => { readyReported=false; render(); });
