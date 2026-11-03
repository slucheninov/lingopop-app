import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import Popup from './popup/Popup.svelte';

const target = document.getElementById('app')!;
const label = getCurrentWindow().label;

// The popup keeps its own bundle and mounts at once (it bootstraps itself before `popup_ready`).
// Settings and History are separate lazy chunks, loaded after the language catalog is installed.
if (label === 'settings') {
  const { bootstrap } = await import('./lib/bootstrap');
  const boot = await bootstrap();
  const { default: Settings } = await import('./settings/Settings.svelte');
  mount(Settings, { target, props: { initialTab: boot.initialTab } });
} else if (label === 'history') {
  const { bootstrap } = await import('./lib/bootstrap');
  await bootstrap();
  const { default: History } = await import('./history/History.svelte');
  mount(History, { target });
} else {
  mount(Popup, { target });
}
