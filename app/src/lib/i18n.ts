/** English catalog keys are their own fallback translations. Substitution never emits HTML. */
export function t(key: string, args: readonly (string | number)[] = []): string {
  let ordered = 0;
  return key.replace(/%(?:(\d+)\$)?(@|lld)/g, (placeholder, position: string | undefined) => {
    const index = position ? Number(position) - 1 : ordered++;
    return args[index] === undefined ? placeholder : String(args[index]);
  });
}
