// Hand-drawn, static SVG only. Never interpolate model text into these strings.
const svg = (paths: string) => `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
export const icons = {
  trash: svg('<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>'),
  copy: svg('<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V4H4v12h4"/>'),
  replace: svg('<path d="M4 7h15m-4-4 4 4-4 4M20 17H5m4-4-4 4 4 4"/>'),
  rewrite: svg('<path d="m4 16 11-11 4 4L8 20H4v-4ZM13 7l4 4"/>'),
  explain: svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.1"/>'),
  retry: svg('<path d="M20 10a8 8 0 1 0-2 8M20 4v6h-6"/>'),
  translate: svg('<path d="M3 5h12M9 3v2m-4 4 8 8M13 5c0 5-4 10-9 12m10 4 5-12 5 12m-8-4h6"/>'),
  chevron: svg('<path d="m6 9 6 6 6-6"/>'),
  // Settings tab icons.
  sliders: svg('<path d="M4 6h10m4 0h2M4 12h2m4 0h10M4 18h12m4 0h2"/><circle cx="16" cy="6" r="2"/><circle cx="8" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>'),
  cpu: svg('<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9.5" y="9.5" width="5" height="5"/><path d="M9 3v3m6-3v3M9 18v3m6-3v3M3 9h3m-3 6h3m12-6h3m-3 6h3"/>'),
  command: svg('<path d="M9 9V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V9Z"/>'),
  chart: svg('<path d="M4 20V10m6 10V4m6 16v-8m6 8H2"/>'),
  sparkles: svg('<path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3Zm7 12 .8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8L19 15Z"/>'),
  info: svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.1"/>'),
  plus: svg('<path d="M12 5v14M5 12h14"/>'),
  minus: svg('<path d="M5 12h14"/>'),
  close: svg('<path d="m6 6 12 12M6 18 18 6"/>'),
  star: svg('<path d="m12 3 2.7 5.6 6.1.8-4.5 4.2 1.1 6.1L12 16.8 6.6 19.7l1.1-6.1L3.2 9.4l6.1-.8L12 3Z" fill="currentColor"/>'),
  key: svg('<circle cx="8" cy="15" r="4"/><path d="m11 12 9-9m-3 3 3 3"/>'),
  check: svg('<circle cx="12" cy="12" r="9"/><path d="m8 12.5 3 3 5-6"/>'),
  external: svg('<path d="M14 4h6v6m0-6-9 9M18 14v5H5V6h5"/>'),
  undo: svg('<path d="M4 10a8 8 0 1 1 2 8M4 4v6h6"/>'),
  list: svg('<path d="M8 6h12M8 12h12M8 18h12M4 6v.1M4 12v.1M4 18v.1"/>'),
};
// Provider glyphs (one per `ProviderType`, like the mac SF Symbols) and their mac `iconColor`.
export const providerIcons = {
  'Claude': svg('<path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8L12 3Z"/>'),
  'OpenAI': svg('<path d="m12 3 7 4v8l-7 4-7-4V7l7-4Zm0 5-3 1.7v3.6L12 15l3-1.7V9.7L12 8Z"/>'),
  'Gemini': svg('<path d="m12 3 8 9-8 9-8-9 8-9Z"/>'),
  'Grok': svg('<path d="M13 3 5 13h6l-1 8 8-10h-6l1-8Z"/>'),
  'Google (Free)': svg('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>'),
  'Google Translate': svg('<path d="M6 4h12v16H6zM6 16h12M10 8h4"/>'),
  'DeepL': svg('<path d="M4 5h16v11H11l-4 4v-4H4V5Z"/>'),
  'Microsoft Translator': svg('<path d="M4 18 9 6l5 12m-8-3h6m4-6h6m-3-2v2c0 3-1 5-3 7m1-3c1 1 2 1.5 3 2"/>'),
} as const;
/** mac `ProviderType.iconColor`; OpenAI is the primary text colour there. */
export const providerColors = {
  'Claude': 'rgb(217,119,87)',
  'OpenAI': 'currentColor',
  'Gemini': 'rgb(66,133,244)',
  'Grok': 'rgb(142,68,173)',
  'Google (Free)': 'rgb(52,168,83)',
  'Google Translate': 'rgb(66,133,244)',
  'DeepL': 'rgb(45,82,122)',
  'Microsoft Translator': 'rgb(0,164,239)',
} as const;
