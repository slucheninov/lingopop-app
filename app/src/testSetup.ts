import { configure } from '@testing-library/svelte';

// The Settings tabs are lazy `import()` chunks. The first test of a file pays for transforming
// that chunk, which takes more than the default 1 s on slow CI runners (macos Intel), so async
// queries wait up to 5 s. A query that succeeds returns as soon as it matches.
configure({ asyncUtilTimeout: 5000 });
