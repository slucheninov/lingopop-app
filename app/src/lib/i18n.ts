export type UiLanguage = 'en' | 'pl' | 'uk' | 'de';

let language: UiLanguage = 'en';
let catalog: Readonly<Record<string, string>> = {};

/**
 * Installs the catalog delivered by the shell (`ui_bootstrap`); English is the empty catalog.
 * The catalog is not bundled: the popup and the other windows receive it at startup.
 */
export function setLanguage(code: UiLanguage, entries: Record<string, string>): void {
  language = code;
  catalog = entries;
}

export function currentLanguage(): string {
  return language;
}

/** `%%`, or `%@` / `%lld` / `%ld` / `%d` with an optional `n$` position: the grammar of core `i18n::format`. */
const PLACEHOLDER = /%(?:(\d+)\$)?(@|lld|ld|d)|%%/g;

/**
 * The translated template when the catalog has it, else the English key itself; substitution never emits HTML.
 * A placeholder without its argument stays as written, and an unpositional one still takes the next ordered slot.
 */
export function t(key: string, args: readonly (string | number)[] = []): string {
  const template = Object.hasOwn(catalog, key) ? catalog[key] : key;
  let ordered = 0;
  return template.replace(PLACEHOLDER, (placeholder: string, position: string | undefined) => {
    if (placeholder === '%%') {
      return '%';
    }
    const index = position === undefined ? ordered++ : Number(position) - 1;
    return index >= 0 && args[index] !== undefined ? String(args[index]) : placeholder;
  });
}
