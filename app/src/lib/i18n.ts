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

/** The translated template when the catalog has it, else the English key itself; substitution never emits HTML. */
export function t(key: string, args: readonly (string | number)[] = []): string {
  const template = Object.hasOwn(catalog, key) ? catalog[key] : key;
  let ordered = 0;
  return template.replace(/%(?:(\d+)\$)?(@|lld)/g, (placeholder, position: string | undefined) => {
    const index = position ? Number(position) - 1 : ordered++;
    return args[index] === undefined ? placeholder : String(args[index]);
  });
}
