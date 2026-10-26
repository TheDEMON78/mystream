'use strict';

(async function () {
  const MS = window.MS;

  MS.appInfo = await window.api.appInfo();
  document.getElementById('app-version').textContent = `v${MS.appInfo.version}`;

  await MS.store.load();
  MS.audio.init();
  MS.compositor.init(document.getElementById('program'));
  MS.preview.init(
    document.getElementById('preview-area'),
    document.getElementById('preview-wrap'),
    document.getElementById('program'),
    document.getElementById('overlay')
  );
  MS.outputs.init();
  MS.syncGlobalAudio();
  MS.runtimes.sync();
  MS.ui.init();

  if (!MS.appInfo.ffmpeg) MS.toast('FFmpeg introuvable : le streaming est désactivé et les enregistrements seront en WebM/MKV bruts.', 'error', 10000);

  // Fermeture : on termine proprement les sorties actives avant de quitter.
  let closing = false;
  window.addEventListener('beforeunload', (e) => {
    if (closing || !MS.outputs.isAnyActive()) return;
    e.preventDefault();
    e.returnValue = false;
    MS.confirm('Quitter MyStream', 'Un streaming ou un enregistrement est en cours. Arrêter et quitter ?', 'Arrêter et quitter').then(async (ok) => {
      if (!ok) return;
      closing = true;
      await MS.outputs.stopAll();
      window.api.quit();
    });
  });
})().catch((e) => {
  console.error(e);
  document.body.append(Object.assign(document.createElement('pre'), { className: 'fatal', textContent: `Erreur au démarrage : ${e.stack || e}` }));
});
