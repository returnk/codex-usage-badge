let requesting = false;
let requestAgain = false;
let cancelGeneration = 0;
async function ready() {
  if (requesting) { requestAgain = true; return; }
  requesting = true;
  requestAgain = false;
  const generation = cancelGeneration;
  try {
    const effect = await window.__TAURI__.core.invoke('celebration_ready');
    if (['welcome','reset'].includes(effect?.kind)) {
      try {
        if (generation !== cancelGeneration) throw new Error('celebration cancelled');
        window.BadgeCelebration.play(document.getElementById('confetti'),effect.kind,effect.origin,effect.durationMs);
        // A frame must run before the native side durably consumes this event.
        await new Promise(resolve=>requestAnimationFrame(resolve));
        if (generation !== cancelGeneration) throw new Error('celebration cancelled');
        if (!await window.__TAURI__.core.invoke('celebration_started',{playId:effect.playId})) throw new Error('celebration acknowledgement rejected');
      } catch(error) {
        if (generation === cancelGeneration) window.BadgeCelebration.cancel();
        await window.__TAURI__.core.invoke('celebration_failed',{playId:effect.playId});
        throw error;
      }
    }
  } catch(error) { console.error(error); }
  finally { requesting=false; if (requestAgain) ready(); }
}
Promise.all([
  window.__TAURI__.event.listen('celebration-cancel',()=>{ cancelGeneration++; window.BadgeCelebration.cancel(); }),
  window.__TAURI__.event.listen('celebration-request',ready)
]).then(() => {
  requestAnimationFrame(() => requestAnimationFrame(ready));
  // Creation can complete just after the page loads; the native HWND must be registered first.
  setTimeout(ready,150);
});
