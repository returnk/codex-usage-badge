const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm'),fs=require('node:fs');
async function page(state,transport){const elements=new Map(),calls=[],listeners=new Map();const element=id=>{if(!elements.has(id))elements.set(id,{textContent:'',hidden:false,disabled:false,style:{},setPointerCapture(){},children:[],replaceChildren(...items){this.children=items;this.textContent=items.map(item=>item.textContent).join('\n');},addEventListener(event,fn){listeners.set(`${id}:${event}`,fn)}});return elements.get(id)};
const document={documentElement:{dataset:{}},getElementById:element,createElement:()=>({textContent:''}),addEventListener(event,fn){listeners.set(`document:${event}`,fn)}};
const window={matchMedia:()=>({matches:false,addEventListener(){}}),__TAURI__:{core:{invoke:async(command,args)=>{calls.push([command,args]);return transport?transport(command,args):command==='get_update_state'?state:true}},event:{listen:async(event,fn)=>{listeners.set(event,fn)}}}};
vm.runInNewContext(fs.readFileSync(`${__dirname}/../web/update.js`,'utf8'),{document,window,requestAnimationFrame:fn=>fn(),console});await new Promise(resolve=>setImmediate(resolve));return {elements,calls,listeners};}
test('loading update popover retains fixed slots and disables release action',async()=>{const p=await page({session:1,checking:true,theme:'glass',currentVersion:'0.3.2'});assert.equal(p.elements.get('update-title').textContent,'正在检查更新…');assert.equal(p.elements.get('release-button').disabled,true);assert.ok(p.calls.some(([cmd])=>cmd==='update_ready'));});
test('release notes are text and closing never changes quota state',async()=>{const p=await page({session:2,checking:false,theme:'system',currentVersion:'0.3.2',release:{available:true,version:'0.3.3',tag:'v0.3.3',notes:'<script>bad()</script>'}});assert.equal(p.elements.get('update-title').textContent,'发现新版本');assert.equal(p.elements.get('update-notes').textContent,'<script>bad()</script>');await p.listeners.get('release-button:click')();assert.ok(p.calls.some(([cmd,args])=>cmd==='open_release'&&args.tag==='v0.3.3'));await p.listeners.get('close-update:click')();assert.ok(p.calls.some(([cmd])=>cmd==='close_update'));assert.ok(!p.calls.some(([cmd])=>cmd==='refresh_quota'));});
test('failed check offers retry in the primary control',async()=>{const p=await page({session:3,checking:false,theme:'glass',currentVersion:'0.3.2',error:'网络连接失败'});assert.equal(p.elements.get('update-title').textContent,'暂时无法检查更新');assert.equal(p.elements.get('update-notes').textContent,'网络连接失败');assert.equal(p.elements.get('install-button').textContent,'重新检查');await p.listeners.get('install-button:click')();assert.ok(p.calls.some(([cmd])=>cmd==='retry_update'));});

test('markdown notes use readable short bullets',async()=>{const p=await page({session:4,checking:false,theme:'glass',currentVersion:'0.3.2',release:{available:true,version:'0.3.3',tag:'v0.3.3',notes:'# 标题\n\n- **优化布局**\n- 修复定位\n\n[下载](https://example.test)'}});assert.equal(p.elements.get('update-notes').textContent,'优化布局\n修复定位');});
test('one click is enabled only for a newer installed release',async()=>{
 for(const [available,installed,label,disabled] of [[true,true,'一键更新',false],[false,true,'已是最新版本',true],[true,false,'一键更新',true]]){
  const p=await page({session:5,checking:false,installed,theme:'glass',currentVersion:'0.3.2',release:{available,version:'0.3.3',tag:'v0.3.3'}});
  assert.equal(p.elements.get('install-button').textContent,label);assert.equal(p.elements.get('install-button').disabled,disabled);
 }
});
test('unknown download length does not invent a percentage and cannot start twice',async()=>{
 const p=await page({session:6,checking:false,installed:true,installing:true,stage:'downloading',downloaded:4096,total:null,theme:'glass',currentVersion:'0.3.2',release:{available:true,tag:'v0.3.3'}});
 assert.equal(p.elements.get('install-button').disabled,true);
 assert.equal(p.elements.get('install-button').textContent,'正在下载…');
 assert.ok(!p.elements.get('download-status').textContent.includes('%'));
});
test('quick drag retains pointer deltas when native begin reply arrives after release',async()=>{
 let acknowledge;
 const state={session:9,theme:'glass',currentVersion:'0.3.2'};
 const p=await page(state,(cmd)=>cmd==='get_update_state'?state:cmd==='drag_update'?new Promise(resolve=>{acknowledge=resolve}):true);
 const event={pointerId:3,button:0,screenX:100,screenY:200,target:{closest:()=>null},preventDefault(){}};
 p.listeners.get('update-drag:pointerdown')(event);
 p.listeners.get('update-drag:pointermove')({...event,screenX:160,screenY:230});
 p.listeners.get('update-drag:pointerup')(event);
 acknowledge(41);await new Promise(resolve=>setImmediate(resolve));
 assert.ok(p.calls.some(([cmd,args])=>cmd==='move_update_drag'&&args.dragId===41&&args.dx===60&&args.dy===30));
 assert.ok(p.calls.some(([cmd,args])=>cmd==='end_update_drag'&&args.dragId===41));
});
test('failed move still ends drag so normal blur closing is restored',async()=>{
 const state={session:10,theme:'glass',currentVersion:'0.3.2'};
 const p=await page(state,(cmd)=>cmd==='get_update_state'?state:cmd==='drag_update'?42:cmd==='move_update_drag'?Promise.reject('simulated monitor unavailable'):true);
 const event={pointerId:4,button:0,screenX:100,screenY:200,target:{closest:()=>null},preventDefault(){}};
 p.listeners.get('update-drag:pointerdown')(event);
 p.listeners.get('update-drag:pointerup')(event);
 await new Promise(resolve=>setImmediate(resolve));
 assert.ok(p.calls.some(([cmd,args])=>cmd==='end_update_drag'&&args.dragId===42));
});
