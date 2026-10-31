// Hand-drawn, static SVG only. Never interpolate model text into these strings.
const svg = (paths: string) => `<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
export const icons = {
  copy: svg('<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V4H4v12h4"/>'),
  replace: svg('<path d="M4 7h15m-4-4 4 4-4 4M20 17H5m4-4-4 4 4 4"/>'),
  rewrite: svg('<path d="m4 16 11-11 4 4L8 20H4v-4ZM13 7l4 4"/>'),
  explain: svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.1"/>'),
  retry: svg('<path d="M20 10a8 8 0 1 0-2 8M20 4v6h-6"/>'),
  translate: svg('<path d="M3 5h12M9 3v2m-4 4 8 8M13 5c0 5-4 10-9 12m10 4 5-12 5 12m-8-4h6"/>'),
  chevron: svg('<path d="m6 9 6 6 6-6"/>'),
  close: svg('<path d="m6 6 12 12M6 18 18 6"/>'),
};
