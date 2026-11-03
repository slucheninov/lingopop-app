export interface HistoryRow {
  id: string; original: string; translated: string; sourceLanguage: string | null;
  targetLanguage: string; providerName: string; operation: string | null;
  operationName: string; timestampUnix: number;
}
export function filterHistory(rows: HistoryRow[], query: string): HistoryRow[] {
  const q = query.trim().toLowerCase();
  return q ? rows.filter(row => row.original.toLowerCase().includes(q) || row.translated.toLowerCase().includes(q)) : rows;
}
export function formatHistoryDate(unix: number, language: string): string {
  return new Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit', day: 'numeric', month: 'short', year: 'numeric' }).format(new Date(unix * 1000));
}
