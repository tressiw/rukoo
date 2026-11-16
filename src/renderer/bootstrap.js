import { setLanguage, t } from './i18n.js';

// Native window controls must have space before the first interface is drawn.
document.documentElement.dataset.platform = window.mail.platform;

// Linux can put the window buttons on the left, as with GNOME's button-layout "close,minimize,maximize:". The window
// controls overlay's titlebar-area rectangle then starts after them, and styles.css lays the title bar out around them.
// Buttons on the right keep the fixed room they have always had.
const overlay = navigator.windowControlsOverlay;
function placeWindowControls() {
  if (overlay?.getTitlebarAreaRect().x > 0) document.documentElement.dataset.controls = 'left';
  else delete document.documentElement.dataset.controls;
}
overlay?.addEventListener('geometrychange', placeWindowControls);
placeWindowControls();

// Set the persisted language before importing modules that construct interface labels.
try {
  const { settings } = await window.mail.call('state');
  setLanguage(settings.language);
  if (window.mail.platform === 'darwin') {
    document.documentElement.style.setProperty('--mac-titlebar-inset', `${86 / ((settings.zoomLevel || 100) / 100)}px`);
  }
  const composing = document.body.classList.contains('compose-window');
  if (composing) {
    document.title = t('composer.titles.new');
    document.getElementById('compose-title').textContent = document.title;
  }
  await import(composing ? './compose.js' : './app.js');
} catch (error) {
  const toast = document.getElementById('toast');
  toast.textContent = error.message;
  toast.classList.add('show');
}
