let requesting = false;
async function ready() {
  if (requesting) return;
  requesting = true;
  try {
    const effect = await window.__TAURI__.core.invoke('celebration_ready');
    if (['welcome','reset'].includes(effect?.kind)) {
      try {
        window.BadgeCelebration.play(document.getElementById('confetti'),effect.kind,effect.origin);
        // A frame must run before the native side durably consumes this event.
        await new Promise(resolve=>requestAnimationFrame(resolve));
        if (!await window.__TAURI__.core.invoke('celebration_started',{playId:effect.playId})) throw new Error('celebration acknowledgement rejected');
      } catch(error) {
        window.BadgeCelebration.cancel();
        await window.__TAURI__.core.invoke('celebration_failed',{playId:effect.playId});
        throw error;
      }
    }
  } catch(error) { console.error(error); }
  finally { requesting=false; }
}
window.__TAURI__.event.listen('celebration-request',ready).then(() => {
  requestAnimationFrame(() => requestAnimationFrame(ready));
  // Creation can complete just after the page loads; the native HWND must be registered first.
  setTimeout(ready,150);
});
