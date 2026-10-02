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
      attributes: {}, setAttribute(name, value) { this.attributes[name] = value; },
      classList: { toggle(name, active) { element(id).dataset[name] = !!active; } }, addEventListener(name, handler) { listeners.set(`${id}:${name}`, handler); },
      getBoundingClientRect() { return { x: 2, y: 2, width: 67, height: 26 }; }, setPointerCapture() {},
      replaceChildren() {}, append() {} });
    return elements.get(id);
  };
  const state = { theme: 'glass', capsule: '38%', fiveHour: 38, weekly: 59, fiveReset: '将于 04:57 重置',
    weekReset: '10/4 01:10 重置', weeklyExhausted: false, freshness: 'fresh', progressBand: 'yellow',
    credits: [], creditCount: 2, creditOpen: false, topmost: false, menuRequestId: 7, ...overrides };
  const app = element('app');
  Object.defineProperty(app, 'innerHTML', { set(html) { for (const match of html.matchAll(/id="([^"]+)"/g)) element(match[1]); for(const match of html.matchAll(/id="([^"]+)"[^>]*>([^<]*)</g)) element(match[1]).textContent=match[2]; } });
  app.querySelector = selector => element(selector);
  const document = { body: {}, documentElement: { dataset: {} }, getElementById: element,
    createElement: element, querySelectorAll() { return [...elements.values()]; }, addEventListener(name, handler) { listeners.set(`document:${name}`, handler); } };
  const media = { matches: false, addEventListener(name, handler) { listeners.set(`media:${name}`, handler); } };
  const window = { __TAURI__: { core: { invoke(command, args) { calls.push([command, args]); return transport ? transport(command,args,state) : Promise.resolve(command === 'get_state' ? state : true); } },
    event: { listen(name,handler) { listeners.set(`tauri:${name}`,handler); return Promise.resolve(() => {}); } } }, matchMedia() { return media; },
    addEventListener(name,handler) { listeners.set(`window:${name}`,handler); } };
  vm.runInNewContext(fs.readFileSync(`${__dirname}/../web/main.js`, 'utf8'), {
    window, document, location: { search: `?view=${view}` }, URLSearchParams, Date, Math, innerWidth: 71, innerHeight: 30,
    devicePixelRatio: 1, getComputedStyle() { return {paddingBottom:'12px'}; }, requestAnimationFrame(handler) { frames.push(handler); }, console });
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

test('one two and three digit quotas update only the numeric span', async () => {
  const p = await page('capsule', { capsule: '8%' });
  assert.equal(p.elements.get('percent').textContent, '8');
  for (const value of ['88%', '100%']) {
    p.state.capsule = value;
    await p.listeners.get('tauri:state-updated')();
    assert.equal(p.elements.get('percent').textContent, value.slice(0, -1));
  }
  assert.ok(!p.calls.some(([command]) => /resize|size/.test(command)));
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

test('both capsule modes double click reset their position', async () => {
  const global = await page('capsule', { topmost: true });
  await global.listeners.get('.capsule:dblclick')();
  assert.deepEqual(global.calls.filter(([cmd]) => cmd === 'reset_position').map(([,args]) => args.fromCapsule), [true]);
  const normal = await page('capsule');
  await normal.listeners.get('.capsule:dblclick')();
  assert.deepEqual(normal.calls.filter(([cmd]) => cmd === 'reset_position').map(([,args]) => args.fromCapsule), [true]);
});

test('native capsule input preserves drag, reset, menu and wheel without DOM focus', async () => {
  const p = await page('capsule');
  const input = p.listeners.get('tauri:native-input-capsule');
  assert.equal(typeof input, 'function');
  for (const kind of ['down', 'up', 'double', 'right', 'wheel']) {
    await input({ payload: { kind, x: 12, y: 12, delta: 120 } });
  }
  for (const command of ['start_drag','stop_drag','reset_position','open_capsule_menu','cycle_theme']) {
    assert.equal(p.calls.filter(([cmd]) => cmd === command).length, 1, command);
  }
});

test('native detail release activates only the button pressed at matching coordinates', async () => {
  const p = await page('detail');
  const button = p.elements.get('credit-button');
  let clicks = 0;
  button.click = () => { clicks++; };
  button.closest = () => button;
  p.document.elementFromPoint = () => button;
  const input = p.listeners.get('tauri:native-input-detail');
  assert.equal(p.listeners.has('tauri:native-input-capsule'), false, 'detail clicks must not start capsule drag');
  assert.equal(typeof input, 'function');
  await input({payload:{kind:'up',x:10,y:10}});
  assert.equal(clicks,0, 'orphaned release must not activate a button');
  await input({payload:{kind:'down',x:10,y:10}});
  p.document.elementFromPoint = () => null;
  await input({payload:{kind:'up',x:10,y:20}});
  assert.equal(clicks,0, 'drag outside must not click');
  p.document.elementFromPoint = () => button;
  await input({payload:{kind:'down',x:10,y:10}});
  await input({payload:{kind:'up',x:10,y:10}});
  assert.equal(clicks,1);
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

test('three-item menu contains topmost settings and exit only', async () => {
  const p = await page('menu');
  assert.equal(p.elements.has('refresh'), false);
  assert.equal(p.elements.has('relocate'), false);
  assert.equal(p.elements.has('startup'), false);
  assert.ok(['settings','topmost','exit'].every(id => p.elements.has(id)));
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

test('capsule mode and hint are accessible', async () => {
  const p = await page('capsule', {topmost:true, hintLevel:'urgent', hintMessages:['本周额度偏低']});
  assert.equal(p.document.documentElement.dataset.topmost, 'true');
  assert.equal(p.elements.get('hint-dot').dataset.level, 'urgent');
  assert.match(p.elements.get('.capsule').attributes['aria-label'], /本周额度偏低/);
});

test('detail shows ordered hints and successful update without countdown or notification row', async () => {
  for (const freshness of ['stale', 'unavailable']) {
    const p = await page('detail', {freshness, fetchedAt:1790812800, countdown:'2小时15分', hintMessages:['本周额度偏低', '<img src=x>'], notificationsEnabled:false});
    assert.equal(p.elements.get('quota-hint').textContent, '本周额度偏低\n<img src=x>');
    assert.doesNotMatch(p.elements.get('five-reset').textContent, /2小时15分/);
    assert.match(p.elements.get('last-updated').textContent, /上次成功更新 \d{2}:\d{2}/);
    assert.equal(p.elements.has('notifications-toggle'), false);
  }
});

test('weekly exhaustion stays explicit and reminder state can be enabled', async () => {
  const p = await page('detail', {weeklyExhausted:true, fiveHour:null, weekly:0, notificationsEnabled:true});
  assert.equal(p.elements.get('five-value').textContent, '--');
  assert.match(p.elements.get('five-reset').textContent, /本周额度已用完/);
  assert.equal(p.elements.get('fill').dataset.band, 'blocked');
  assert.equal(p.elements.has('notifications-toggle'), false);
});

test('fresh detail hides last success time while retaining it for failed refreshes', async () => {
  const p = await page('detail', {freshness:'fresh', fetchedAt:1790812800});
  assert.equal(p.elements.get('last-updated').hidden, true);
  assert.match(p.elements.get('last-updated').textContent, /上次成功更新 \d{2}:\d{2}/);
  p.state.freshness = 'stale';
  await p.listeners.get('tauri:state-updated')();
  assert.equal(p.elements.get('last-updated').hidden, false);
});

test('native hover restores button feedback and clears it on leave', async () => {
  const p = await page('detail', {credits:[{id:'a',expiresAt:1792710000}]});
  const button=p.elements.get('credit-button');
  button.closest=()=>button;
  p.document.elementFromPoint=()=>button;
  const input=p.listeners.get('tauri:native-input-detail');
  await input({payload:{kind:'move',x:10,y:10}});
  assert.equal(button.dataset['native-hover'],true);
  await input({payload:{kind:'leave',x:10,y:10}});
  assert.equal(button.dataset['native-hover'],false);
});

test('capsule has no percent unit and details use separate small units', async () => {
  for(const topmost of [false,true]) {
    const p=await page('capsule',{topmost,capsule:'100%'});
    assert.equal(p.elements.get('capsule-unit').hidden,!topmost);
    assert.equal(p.elements.get('percent').textContent,'100');
  }
  const p=await page('detail',{fiveHour:9,weekly:99});
  assert.equal(p.elements.get('five-value').textContent,'9');
  assert.equal(p.elements.get('week-value').textContent,'99');
  assert.equal(p.elements.get('five-unit').textContent,'%');
});

test('detail height remains intrinsic after a constrained panel scrolls', async () => {
  const p=await page('detail');
  const card=p.elements.get('app').querySelector('.detail-card');
  card.scrollTop=0;
  card.getBoundingClientRect=()=>({top:2});
  card.children=[{hidden:false,getBoundingClientRect:()=>({bottom:182-card.scrollTop})}];
  async function measure() {
    p.listeners.get('window:resize')();
    while(p.frames.length) {await p.frames.shift()();}
    return p.calls.filter(([cmd])=>cmd==='report_detail_height').at(-1)[1].height;
  }
  assert.equal(await measure(),198);
  card.scrollTop=40;
  assert.equal(await measure(),198);
});

test('only the global capsule shows a small percent unit', async () => {
  const p=await page('capsule',{topmost:true,capsule:'100%'});
  assert.equal(p.elements.get('capsule-unit')?.hidden,false);
  p.state.topmost=false;
  await p.listeners.get('tauri:state-updated')();
  assert.equal(p.elements.get('capsule-unit').hidden,true);
});

test('detail reset line keeps the time without the duplicate countdown', async () => {
  const p=await page('detail',{countdown:'约3小时19分钟后重置'});
  assert.equal(p.elements.get('five-reset').textContent,p.state.fiveReset);
});

test('weekly-only detail uses weekly header percentage track and standalone credits', async () => {
  const p=await page('detail',{quotaMode:'weekly',fiveHour:null,weekly:69,progressBand:'green',planLabel:'PRO'});
  assert.equal(p.elements.get('quota-label').textContent,'本周剩余');
  assert.equal(p.elements.get('five-value').textContent,'69');
  assert.equal(p.elements.get('fill').style.width,'69%');
  assert.equal(p.elements.get('five-reset').hidden,true);
  assert.equal(p.elements.get('weekly-reset').textContent,p.state.weekReset);
  assert.equal(p.elements.get('weekly-row').hidden,true);
  assert.equal(p.elements.get('weekly-reset').hidden,false);
  assert.equal(p.elements.get('plan-label').textContent,'PRO');
});

test('unavailable weekly sample keeps mode without inventing a percentage and permits refresh', async () => {
  const p=await page('detail',{quotaMode:'weekly',fiveHour:null,weekly:null,freshness:'unavailable',quotaStatus:'连接超时'});
  assert.equal(p.elements.get('quota-label').textContent,'本周剩余');
  assert.equal(p.elements.get('five-value').textContent,'--');
  assert.equal(p.elements.get('five-unit').hidden,true);
  assert.equal(p.elements.has('refresh-quota'),false);
});

test('quota detail has no refresh or update controls and cannot retain update results', async () => {
 const p=await page('detail');
 for(const id of ['refresh-quota','check-update','update-status','update-notes','update-release','quota-status']) assert.equal(p.elements.has(id),false,id);
});
