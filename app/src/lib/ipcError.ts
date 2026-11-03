import { t } from './i18n';
export function renderError(e: string): string {
  const [key, ...args] = e.split('|');
  return t(key, args);
}
