// Local canvas-confetti renderer for both celebrations in the outside layer.
window.BadgeCelebration = (() => {
  let stop = () => {};
  function play(canvas, kind, origin = {x:0.5,y:0.65}) {
    stop();
    if (!['welcome','reset'].includes(kind) || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const fire = window.confetti.create(canvas,{resize:true,useWorker:false,disableForReducedMotion:true});
    const timers = [];
    let cancelled = false;
    stop = () => {
      cancelled = true;
      timers.forEach(clearTimeout);
      fire.reset();
      canvas.getContext('2d')?.clearRect(0,0,canvas.width,canvas.height);
    };
    const colors = ['#4389ed','#a8cfff','#e8ce87','#ffffff'];
    const common = {origin,colors,ticks:145,gravity:0.65,decay:0.94,scalar:0.65,disableForReducedMotion:true};
    if (kind === 'welcome') {
      const capsule = window.confetti.shapeFromPath({path:'M -4 -2 L 4 -2 A 2 2 0 0 1 4 2 L -4 2 A 2 2 0 0 1 -4 -2 Z'});
      const star = window.confetti.shapeFromPath({path:'M 0 -5 L 1.3 -1.3 L 5 0 L 1.3 1.3 L 0 5 L -1.3 1.3 L -5 0 L -1.3 -1.3 Z'});
      fire({...common,shapes:[capsule,'circle',star],particleCount:34,spread:85,startVelocity:16});
    } else {
      const burst = () => {
        if (cancelled) return;
        fire({...common,particleCount:22+Math.floor(Math.random()*15),angle:50+Math.random()*80,
          spread:65+Math.random()*60,startVelocity:24+Math.random()*10});
      };
      burst();
      timers.push(setTimeout(burst,400));
    }
    timers.push(setTimeout(() => stop(),2500));
  }
  return {play,cancel:()=>stop()};
})();
