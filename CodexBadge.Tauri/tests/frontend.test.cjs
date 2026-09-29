const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

async function page(view, overrides = {}, transport) {
  const elements = new Map();
  const calls = [];
  const listeners = new Map();
  const frames = [];
  const element = id => {
    if (!elements.has(id)) elements.set(id, { textContent: '', hidden: false, style: {}, dataset: {},
      classList: { toggle() {} }, addEventListener(name, handler) { listeners.set(`${id}:${name}`, handler); },
      getBoundingClientRect() { return { x: 2, y: 2, width: 67, height: 26 }; }, setPointerCapture() {},
      replaceChildren() {}, append() {} });
    return elements.get(id);
  };
  const state = { theme: 'glass', capsule: '38%', fiveHour: 38, weekly: 59, fiveReset: '将于 04:57 重置',
    weekReset: '10月4日 01:10 重置', weeklyExhausted: false, freshness: 'fresh', progressBand: 'yellow',
    credits: [], creditCount: 2, creditOpen: false, topmost: false, menuRequestId: 7, ...overrides };
  const app = element('app');
  Object.defineProperty(app, 'innerHTML', { set(html) { for (const match of html.matchAll(/id="([^"]+)"/g)) element(match[1]); } });
  app.querySelector = selector => element(selector);
  const document = { body: {}, documentElement: { dataset: {} }, getElementById: element,
    createElement: element, addEventListener(name, handler) { listeners.set(`document:${name}`, handler); } };
  const media = { matches: false, addEventListener(name, handler) { listeners.set(`media:${name}`, handler); } };
  const window = { __TAURI__: { core: { invoke(command, args) { calls.push([command, args]); return transport ? transport(command,args,state) : Promise.resolve(command === 'get_state' ? state : true); } },
    event: { listen(name,handler) { listeners.set(`tauri:${name}`,handler); return Promise.resolve(() => {}); } } }, matchMedia() { return media; },
    addEventListener(name,handler) { listeners.set(`window:${name}`,handler); } };
  vm.runInNewContext(fs.readFileSync(`${__dirname}/../web/main.js`, 'utf8'), {
    window, document, location: { search: `?view=${view}` }, URLSearchParams, Date, Math, innerWidth: 71, innerHeight: 30,
    devicePixelRatio: 1, requestAnimationFrame(handler) { frames.push(handler); }, console });
  await new Promise(resolve => setImmediate(resolve));
  return { elements, listeners, calls, state, document, media, frames };
}

test('global capsule right click suppresses browser menu and requests our popup', async () => {
  const p = await page('capsule', { topmost: true });
  let prevented = false;
  const handler = p.listeners.get('.capsule:contextmenu') || p.listeners.get('document:contextmenu');
  assert.equal(typeof handler, 'function', 'right click must have a handler');
  await handler({ preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.ok(p.calls.some(([command]) => command === 'open_capsule_menu'));
  assert.ok(!p.calls.some(([command]) => command === 'start_drag'));
});

test('normal capsule right click suppresses browser menu and opens our popup', async () => {
  const p = await page('capsule');
  let prevented = false;
  await p.listeners.get('.capsule:contextmenu')({ preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  assert.equal(p.calls.filter(([cmd]) => cmd === 'open_capsule_menu').length, 1);
});

test('diagnostic cursor comparison is opt-in and does not disable drag', async () => {
  const normal = await page('capsule');
  assert.equal(normal.elements.get('.capsule').style.cursor, undefined);
  const comparison = await page('capsule', { defaultCursor: true });
  assert.equal(comparison.elements.get('.capsule').style.cursor, 'default');
  await comparison.listeners.get('.capsule:pointerdown')({button:0,detail:1,pointerId:9,preventDefault(){}});
  assert.equal(comparison.calls.filter(([cmd]) => cmd === 'start_drag').length, 1);
});

test('global double click never relocates but normal double click resets only its offset', async () => {
  const global = await page('capsule', { topmost: true });
  await global.listeners.get('.capsule:dblclick')();
  assert.ok(!global.calls.some(([cmd]) => cmd === 'reset_position'));
  const normal = await page('capsule');
  await normal.listeners.get('.capsule:dblclick')();
  assert.deepEqual(normal.calls.filter(([cmd]) => cmd === 'reset_position').map(([,args]) => args.fromCapsule), [true]);
});

test('pointer presses prevent browser focus defaults without dropping drag or right click', async () => {
  const p = await page('capsule', { topmost: true });
  let prevented = 0;
  await p.listeners.get('.capsule:pointerdown')({button:0,detail:1,pointerId:9,preventDefault(){prevented++;}});
  await p.listeners.get('.capsule:pointerdown')({button:2,detail:1,pointerId:9,preventDefault(){prevented++;}});
  assert.equal(prevented, 2);
  assert.equal(p.calls.filter(([cmd]) => cmd === 'start_drag').length, 1);
});

test('button press suppresses DOM focus but keeps the credit toggle click', async () => {
  const p=await page('detail',{credits:[{id:'a',expiresAt:1792710000}]});
  const handler=p.listeners.get('document:mousedown');
  assert.equal(typeof handler,'function');
  let prevented=false;
  handler({target:{closest:()=>({tagName:'BUTTON'})},preventDefault(){prevented=true;}});
  assert.equal(prevented,true);
  await p.listeners.get('credit-button:click')();
  assert.equal(p.calls.filter(([cmd])=>cmd==='toggle_credit').length,1);
});

test('four-item menu closes with the request identity captured at click', async () => {
  const p = await page('menu');
  assert.equal(p.elements.has('refresh'), false);
  assert.ok(['startup','topmost','relocate','exit'].every(id => p.elements.has(id)));
  await p.listeners.get('topmost:click')();
  assert.deepEqual(p.calls.filter(([cmd]) => cmd === 'hide_menu').map(([,args]) => args.requestId), [7]);
});

test('initial render waits two frames before ready and resize reports settled capsule dimensions', async () => {
  const p = await page('capsule');
  assert.ok(!p.calls.some(([cmd]) => cmd === 'window_ready'));
  for (let i=0;i<3;i++) { const current = p.frames.splice(0); for (const callback of current) callback(); await new Promise(r=>setImmediate(r)); }
  assert.equal(p.calls.filter(([cmd]) => cmd === 'window_ready').length,1);
  const before = p.calls.filter(([cmd]) => cmd === 'report_capsule_layout').length;
  p.listeners.get('window:resize')();
  for (let i=0;i<3;i++) { const current=p.frames.splice(0); for (const callback of current) callback(); }
  assert.equal(p.calls.filter(([cmd]) => cmd === 'report_capsule_layout').length,before+1);
});

test('bursts coalesce state reads and eventually apply the newest theme and number', async () => {
  let release;
  let count = 0;
  const p = await page('capsule',{},(command,args,state) => {
    if(command !== 'get_state') return Promise.resolve(true);
    count++;
    return new Promise(resolve => { release = () => resolve({...state}); });
  });
  for(let i=0;i<20;i++) p.listeners.get('tauri:state-updated')();
  assert.equal(count,1);
  release(); await new Promise(r=>setImmediate(r));
  assert.equal(count,2);
  p.state.theme='system'; p.state.capsule='12%'; release(); await new Promise(r=>setImmediate(r));
  assert.equal(p.elements.get('percent').textContent,'12');
  assert.equal(p.document.documentElement.dataset.theme,'light');
});

test('unaccepted ready is retried and a backend retry event resends the handshake', async () => {
  let attempts=0;
  const p=await page('menu',{},(cmd,args,state)=>Promise.resolve(cmd==='get_state' ? state : cmd==='window_ready' ? ++attempts>1 : true));
  for(let i=0;i<6;i++) { const current=p.frames.splice(0);for(const callback of current)callback(); await new Promise(r=>setImmediate(r)); }
  assert.equal(attempts,2);
  p.listeners.get('tauri:retry-ready')();
  for(let i=0;i<3;i++) { const current=p.frames.splice(0);for(const callback of current)callback(); await new Promise(r=>setImmediate(r)); }
  assert.equal(attempts,3);
});

test('credit summary uses server count even without expiry details', async () => {
  const p = await page('detail');
  assert.equal(p.elements.get('credit-count').textContent, 2);
  assert.equal(p.elements.get('credit-button').hidden, true);
});

test('unavailable credits are unknown rather than zero', async () => {
  const p = await page('detail', { freshness: 'unavailable', creditCount: null });
  assert.equal(p.elements.get('credit-count').textContent, '重置机会未知');
});

test('cached quotas visibly identify a failed refresh', async () => {
  const p = await page('detail', { freshness: 'stale' });
  assert.match(p.elements.get('five-reset').textContent, /数据待更新/);
});

test('system appearance reacts to OS changes without replacing the number', async () => {
  const p = await page('capsule', { theme: 'system', capsule: '38%' });
  assert.equal(p.document.documentElement.dataset.theme, 'light');
  p.media.matches = true;
  p.listeners.get('media:change')();
  assert.equal(p.document.documentElement.dataset.theme, 'dark');
  assert.equal(p.elements.get('percent').textContent, '38');
});
