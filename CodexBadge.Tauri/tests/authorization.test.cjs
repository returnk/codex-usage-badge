const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm'),fs=require('node:fs');
async function page(state,transport){
 const elements=new Map(),listeners=new Map(),calls=[];
 const element=id=>{if(!elements.has(id))elements.set(id,{textContent:'',hidden:false,disabled:false,focus(){this.focused=true;},addEventListener(event,fn){listeners.set(`${id}:${event}`,fn);}});return elements.get(id);};
 const document={documentElement:{dataset:{}},getElementById:element,addEventListener(event,fn){listeners.set(`document:${event}`,fn);}};
 const window={addEventListener(){},matchMedia:()=>({matches:false,addEventListener(){}}),__TAURI__:{core:{invoke:async(cmd,args)=>{calls.push([cmd,args]);return transport?transport(cmd,args):cmd==='get_authorization_state'?state:true;}},event:{listen:async(event,fn)=>listeners.set(event,fn)}}};
 vm.runInNewContext(fs.readFileSync(`${__dirname}/../web/authorization.js`,'utf8'),{document,window,devicePixelRatio:1.25,requestAnimationFrame:fn=>fn(),console});
 await new Promise(resolve=>setImmediate(resolve));return {elements,listeners,calls};
}
test('opening shows account as text and focuses cancel without granting permission',async()=>{
 const p=await page({session:3,label:'<b>demo@example.test (PLUS)</b>',theme:'glass',busy:false});
 assert.equal(p.elements.get('authorization-account').textContent,'<b>demo@example.test (PLUS)</b>');
 assert.equal(p.elements.get('authorization-cancel').focused,true);
 assert.ok(!p.calls.some(([cmd])=>cmd==='answer_authorization'));
});
test('Escape cancels the current session; only the explicit primary button accepts',async()=>{
 for(const accepted of [false,true]){
  const p=await page({session:7,label:'demo@example.test (PLUS)',theme:'dark',busy:false});
  if(accepted)await p.listeners.get('authorization-accept:click')();
  else await p.listeners.get('document:keydown')({key:'Escape',preventDefault(){}});
  const call=p.calls.find(([cmd])=>cmd==='answer_authorization');assert.equal(call[1].accepted,accepted);assert.equal(call[1].session,7);
 }
});
test('saving disables duplicate accept and Escape; a failed save offers return without granting again',async()=>{
 const state={session:8,label:'demo@example.test (PLUS)',theme:'glass',busy:true};const p=await page(state);
 assert.equal(p.elements.get('authorization-accept').disabled,true);
 await p.listeners.get('document:keydown')({key:'Escape',preventDefault(){}});
 assert.ok(!p.calls.some(([cmd])=>cmd==='answer_authorization'));
 state.busy=false;state.error='确认期间账号变化，未开启';await p.listeners.get('authorization-state')();
 assert.equal(p.elements.get('authorization-accept').hidden,true);
 assert.equal(p.elements.get('authorization-error').textContent,state.error);
 await p.listeners.get('authorization-cancel:click')();assert.equal(p.calls.find(([cmd])=>cmd==='answer_authorization')[1].accepted,false);
});
