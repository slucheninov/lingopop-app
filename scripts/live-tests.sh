#!/usr/bin/env bash
# Owner-run live smoke tests against the real provider APIs. Never run in CI.
# Run it on a Mac before a release or after a provider change. It sends real requests that
# may cost a few cents; a provider whose variable is not set is skipped.
#
# Environment variables (values are read by the tests and never printed):
#   LINGOPOP_LIVE_CLAUDE_KEY, LINGOPOP_LIVE_OPENAI_KEY, LINGOPOP_LIVE_GEMINI_KEY,
#   LINGOPOP_LIVE_GROK_KEY            LLM providers: translate, grammar fix, vision
#   LINGOPOP_LIVE_GOOGLE_CLOUD_KEY    optional: LINGOPOP_LIVE_GOOGLE_CLOUD_REFERER
#   LINGOPOP_LIVE_DEEPL_KEY
#   LINGOPOP_LIVE_MICROSOFT_KEY       optional: LINGOPOP_LIVE_MICROSOFT_REGION
#   LINGOPOP_LIVE_GOOGLE_FREE=1       unofficial endpoint, no key
# Optional model override: LINGOPOP_LIVE_<P>_MODEL with <P> = CLAUDE, OPENAI, GEMINI, GROK,
# GOOGLE_CLOUD (default: the model from default_providers()).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

exec cargo test -p lingopop-core --test live -- --ignored --test-threads=1 --nocapture
