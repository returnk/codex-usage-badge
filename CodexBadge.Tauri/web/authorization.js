const invoke=(command,args={})=>window.__TAURI__.core.invoke(command,args);
const el=id=>document.getElementById(id);
let current=null,submitting=false,focusedSession=0,rendering=false,again=false;
const systemTheme=window.matchMedia('(prefers-color-scheme: dark)');
async function render(){
 again=true;if(rendering)return;rendering=true;
 try{do{
  again=false;current=await invoke('get_authorization_state');
  document.documentElement.dataset.theme=current.theme==='system'?(systemTheme.matches?'dark':'light'):current.theme;
  const busy=!!current.busy,error=current.error;
  el('authorization-title').textContent=error?'未能开启临期用卡':'开启临期用卡';
  const account=(current.label||'').match(/^(.*) \(([^()]*)\)$/);
  el('authorization-account').textContent=account?account[1]:current.label||'';
  el('authorization-plan').textContent=account?account[2]:'';
  el('authorization-plan').hidden=!account;
  el('authorization-account-box').hidden=!current.label;
  el('authorization-terms').hidden=!!error;
  el('authorization-error').hidden=!error;
  el('authorization-error').textContent=error||'';
  el('authorization-status').textContent=busy?'正在核对账号并保存授权…':error?'此次未开启，请核查后重新操作。':'取消或关闭不会开启';
  el('authorization-cancel').textContent=error?'返回设置':'取消';
  el('authorization-accept').textContent=busy?'正在开启…':'授权并开启';
  el('authorization-accept').hidden=!!error;
  for(const id of ['authorization-cancel','authorization-close','authorization-accept'])el(id).disabled=busy||submitting;
  await invoke('sync_webview_scale',{pixelRatio:devicePixelRatio});
  await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  await invoke('authorization_ready',{session:current.session});
  if(!busy&&(focusedSession!==current.session||error)){el('authorization-cancel').focus();focusedSession=current.session;}
 }while(again);}catch(error){console.error(error);}finally{rendering=false;}
}
async function answer(accepted){
 if(!current||current.busy||submitting||(accepted&&current.error))return;
 submitting=true;
 for(const id of ['authorization-cancel','authorization-close','authorization-accept'])el(id).disabled=true;
 try{await invoke('answer_authorization',{session:current.session,accepted});}
 catch(error){el('authorization-error').textContent=String(error);el('authorization-error').hidden=false;}
 finally{submitting=false;await render();}
}
el('authorization-accept').addEventListener('click',()=>answer(true));
el('authorization-cancel').addEventListener('click',()=>answer(false));
el('authorization-close').addEventListener('click',()=>answer(false));
document.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();return answer(false);}});
document.addEventListener('contextmenu',event=>event.preventDefault());
window.addEventListener('resize',render);
function watchScale(){window.matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change',()=>{render();watchScale();},{once:true});}
watchScale();systemTheme.addEventListener('change',render);
window.__TAURI__.event.listen('authorization-state',render).then(render);
