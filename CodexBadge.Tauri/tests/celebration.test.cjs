const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');

function renderer(reduced = false) {
  const calls = [], timers = new Map();
  let next = 0, clears = 0, resets = 0;
  const fire = options => calls.push(options);
  fire.reset = () => resets++;
  const canvas = {width:600,height:450,getContext:()=>({clearRect:()=>clears++})};
  const window = {matchMedia:()=>({matches:reduced}),confetti:{
    create:(_canvas,options)=>{assert.equal(options.useWorker,false);assert.equal(options.resize,true);return fire;},
    shapeFromPath:({path})=>({path})
  }};
  vm.runInNewContext(fs.readFileSync(`${__dirname}/../web/celebration.js`,'utf8'),{
    window,Math,setTimeout:(callback,ms)=>{const id=++next;timers.set(id,{callback,ms});return id;},
    clearTimeout:id=>timers.delete(id)
  });
  const advance = ms => {
    for(const [id,timer] of [...timers]) if(timer.ms<=ms && timers.has(id)){timers.delete(id);timer.callback();}
  };
  return {api:window.BadgeCelebration,canvas,calls,timers,advance,get resets(){return resets;},get clears(){return clears;}};
}

test('welcome uses capsule circle and star shapes and stops at 2.5 seconds',()=>{
  const r=renderer(); r.api.play(r.canvas,'welcome');
  assert.equal(r.calls.length,1);
  assert.equal(r.calls[0].shapes[1],'circle');
  assert.ok(r.calls[0].shapes[0].path.includes('A 2 2'));
  assert.equal(r.calls[0].particleCount,34);
  r.advance(2500);assert.equal(r.resets,1);assert.equal(r.clears,1);assert.equal(r.timers.size,0);
});
test('reset has two bounded random bursts at the native panel origin and clears',()=>{
  const r=renderer(), origin={x:0.18,y:0.81};r.api.play(r.canvas,'reset',origin);
  assert.equal(r.calls.length,1);r.advance(400);assert.equal(r.calls.length,2);
  for(const call of r.calls){assert.equal(call.origin,origin);assert.ok(call.particleCount>=22 && call.particleCount<=36);assert.ok(call.angle>=50 && call.angle<130);}
  r.advance(2500);assert.equal(r.resets,1);assert.equal(r.timers.size,0);
});
test('cancel and replacement stop old bursts and reduced motion produces no particles',()=>{
  const r=renderer();r.api.play(r.canvas,'reset');r.api.cancel();r.advance(2500);assert.equal(r.calls.length,1);
  r.api.play(r.canvas,'reset');r.api.play(r.canvas,'welcome');r.advance(400);assert.equal(r.calls.length,3);
  const quiet=renderer(true);quiet.api.play(quiet.canvas,'reset');assert.equal(quiet.calls.length,0);assert.equal(quiet.timers.size,0);
});
