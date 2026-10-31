<script lang="ts">
  import type { LoadingModel } from '../lib/types';
  let {model}: {model:LoadingModel} = $props();
  const labels = {translate:'Translating…',grammarFix:'Fixing grammar…',rewrite:'Rewriting…',explain:'Explaining…',screenTranslate:'Recognizing text…',magic:'Working…'};
</script>
<header role="status"><span class="spinner" aria-hidden="true"></span><strong>{labels[model.operation]}</strong></header>
<p class="preview">{model.preview}</p>
<style>
  header { display:flex; align-items:center; gap:8px; }
  .preview { color:GrayText; white-space:pre-wrap; overflow-wrap:anywhere; }
  .spinner { width:12px; height:12px; border:2px solid #bbb; border-top-color:#3478cf; border-radius:50%; animation:spin 1s linear infinite; }
  @keyframes spin { to { transform:rotate(360deg); } }
  @media (prefers-reduced-motion:reduce) { .spinner { animation:none; } }
</style>
