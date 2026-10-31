// Mirrors orchestrator/view_model.rs, including frozen core serde raw values.
export type Provider = 'Claude' | 'OpenAI' | 'Gemini' | 'Grok' | 'Google (Free)' | 'Google Translate' | 'DeepL' | 'Microsoft Translator';
export type Operation = 'translate' | 'grammarFix' | 'rewrite' | 'explain' | 'screenTranslate' | 'magic';
export type RowMode = 'full' | 'mixed' | 'compact';
export interface ActionRowView { preferred: RowMode; candidates: RowMode[] }
export interface UserMessageView { key: string; args: string[] }
export interface RetryProvider { provider: Provider; name: string }
interface Base { retryProviders: RetryProvider[] }
export interface LoadingModel extends Base { phase: 'loading'; operation: Operation; preview: string }
export interface OcrModel extends Base { phase: 'ocr'; text: string; translateProviders: RetryProvider[]; activeTranslateProvider: Provider | null; actionRow: ActionRowView }
export interface ResultModel extends Base {
  phase: 'result'; text: string; operation: Operation; provider: Provider; providerName: string;
  model: string; targetLanguage: string; viaFallback: boolean;
  fallback: { primaryName: string; message: UserMessageView; details: UserMessageView[] } | null;
  truncated: boolean; showReplace: boolean; showExplain: boolean; quickLanguages: string[]; actionRow: ActionRowView;
}
export interface ErrorModel extends Base { phase: 'error'; message: UserMessageView; details: UserMessageView[] }
export type PopupModel = LoadingModel | OcrModel | ResultModel | ErrorModel;
export type PopupAction = { type: 'copy' | 'replace' | 'rewrite' | 'explain' | 'close' }
  | { type: 'retry' | 'translateOcr'; provider: Provider } | { type: 'retranslate'; language: string };
