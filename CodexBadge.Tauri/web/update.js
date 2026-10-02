const invoke=(command,args={})=>window.__TAURI__.core.invoke(command,args);
const el=id=>document.getElementById(id);
let releaseTag=null, rendering=false,again=false,latestState=null;
const systemTheme=window.matchMedia('(prefers-color-scheme: dark)');
let theme='glass';
function applyTheme(){document.documentElement.dataset.theme=theme==='system'?(systemTheme.matches?'dark':'light'):theme;}
systemTheme.addEventListener('change',applyTheme);
function noteSummary(notes){
 const lines=(notes||'暂无更新说明').replace(/\r/g,'').split('\n').map(line=>line.trim()).filter(line=>line&&!/^\|/.test(line));
 const bullets=lines.filter(line=>/^[-*•]\s/.test(line));
 const selected=bullets.length?bullets:lines.filter(line=>!/^#{1,6}\s/.test(line));
 return selected.slice(0,2).map(line=>line.replace(/^[-*•]\s/,'').replace(/\*\*/g,'').replace(/\[([^\]]+)\]\([^)]*\)/g,'$1'));
}
function notes(lines,message=false){
 const list=el('update-notes');list.className=message?'message':'';
 list.replaceChildren(...lines.map(text=>{const item=document.createElement('li');item.textContent=text;return item;}));
}
async function render(){again=true;if(rendering)return;rendering=true;try{do{again=false;const state=await invoke('get_update_state');latestState=state;theme=state.theme;applyTheme();const release=state.release;releaseTag=release?.tag||null;
 el('current-version').textContent=state.currentVersion;
 el('latest-version').textContent=release?.version||'—';
 el('update-title').textContent=state.installing?'正在更新…':state.error?(state.stage==='failed'?'更新未完成':'暂时无法检查更新'):state.checking?'正在检查更新…':release?.available?'发现新版本':'当前无需更新';
 el('notes-title').textContent=state.error?'更新结果':'更新说明';
 notes(state.checking?['正在获取发布信息，请稍候。']:state.error?[state.error]:noteSummary(release?.notes),state.checking||!!state.error);
 if(!state.error&&(state.installing||(release?.available&&!state.installed)))el('update-notes').className='status';
 el('release-button').disabled=state.checking||!releaseTag;
 const primary=el('install-button');
 primary.textContent=state.installing?(state.stage==='preparing'?'正在准备…':state.stage==='installing'?'正在安装…':'正在下载…'):state.checking?'正在检查…':state.stage==='failed'?'重试更新':state.error?'重新检查':release?.available?'一键更新':'已是最新版本';
 primary.disabled=!!state.checking||!!state.installing||(!state.error&&!release?.available)||(!!release?.available&&!state.installed);
 primary.title=release?.available&&!state.installed?'绿色版或未确认安装位置，请通过发布页下载新版。':'下载并验证后升级，完成后重新启动。';
 const status=el('download-status');status.hidden=!state.installing&&!(release?.available&&!state.installed);
 status.textContent=!state.installed?'绿色版／未确认安装位置，请通过发布页更新。':state.stage==='preparing'?'正在获取签名更新包…':state.stage==='installing'?'验证完成，即将退出升级并重新启动。':state.total>0?`已下载 ${Math.min(100,Math.floor(state.downloaded/state.total*100))}%`:`已下载 ${((state.downloaded||0)/1048576).toFixed(1)} MB`;
 requestAnimationFrame(()=>requestAnimationFrame(()=>invoke('update_ready',{session:state.session}).catch(console.error)));
 }while(again);}catch(error){console.error(error);el('update-title').textContent='暂时无法检查更新';notes(['窗口连接失败，请关闭后重试。'],true);}finally{rendering=false;}}
el('close-update').addEventListener('click',()=>invoke('close_update'));
el('release-button').addEventListener('click',async()=>{if(releaseTag){try{await invoke('open_release',{tag:releaseTag});}catch(error){notes([String(error)],true);}}});
el('install-button').addEventListener('click',async()=>{
 if(el('install-button').disabled)return;
 el('install-button').disabled=true;
 try{await invoke(latestState?.release?.available?'install_update':'retry_update');}
 catch(error){notes([String(error)],true);}
 finally{render();}
});
let headerDrag=null;
async function flushDrag(drag){
 if(!drag.token||drag.flushing)return;
 drag.flushing=true;
 try{do{drag.dirty=false;await invoke('move_update_drag',{dragId:drag.token,dx:drag.dx,dy:drag.dy});
  if(drag.ended&&!drag.dirty){await invoke('end_update_drag',{dragId:drag.token});if(headerDrag===drag)headerDrag=null;break;}
 }while(drag.dirty);}catch(error){
  console.error(error);drag.ended=true;
  try{await invoke('end_update_drag',{dragId:drag.token});}catch(cleanupError){console.error(cleanupError);}
  if(headerDrag===drag)headerDrag=null;
 }finally{drag.flushing=false;}
}
const header=el('update-drag');
header.addEventListener('pointerdown',event=>{
 if(event.button!==0||event.target.closest('button'))return;
 event.preventDefault();header.setPointerCapture(event.pointerId);
 const drag={pointer:event.pointerId,x:event.screenX,y:event.screenY,dx:0,dy:0,dirty:true,ended:false};headerDrag=drag;
 invoke('drag_update').then(token=>{drag.token=token;flushDrag(drag);}).catch(console.error);
});
header.addEventListener('pointermove',event=>{
 const drag=headerDrag;if(!drag||drag.pointer!==event.pointerId||drag.ended)return;
 drag.dx=event.screenX-drag.x;drag.dy=event.screenY-drag.y;drag.dirty=true;flushDrag(drag);
});
function finishDrag(event){const drag=headerDrag;if(!drag||drag.pointer!==event.pointerId||drag.ended)return;drag.ended=true;drag.dirty=true;flushDrag(drag);}
header.addEventListener('pointerup',finishDrag);
header.addEventListener('pointercancel',finishDrag);
header.addEventListener('lostpointercapture',finishDrag);
document.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();invoke('close_update');}});
document.addEventListener('contextmenu',event=>event.preventDefault());
window.__TAURI__.event.listen('update-state',render).then(render);
window.__TAURI__.event.listen('state-updated',render);
