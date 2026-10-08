// Applies the stored theme before first paint (a classic script, so it runs
// before the module graph loads; the CSP forbids inline scripts). Mirrors
// theme.js `apply` for the two values that matter at paint time.
(function () {
  var theme = 'dark', size = 16;
  try {
    var p = JSON.parse(localStorage.getItem('wks.mnext.theme') || 'null');
    var light = window.matchMedia && matchMedia('(prefers-color-scheme: light)').matches;
    theme = !p || p.match ? (light ? 'light' : 'dark') : p.theme || 'dark';
    var n = Number(localStorage.getItem('wks.mnext.textSize'));
    if (n >= 13 && n <= 20) size = n;
  } catch (e) { /* defaults */ }
  document.documentElement.setAttribute('data-theme', theme);
  document.documentElement.style.setProperty('--chat-size', size + 'px');
})();
