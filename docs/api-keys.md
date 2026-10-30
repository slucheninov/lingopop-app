# Getting an API key for LingoPop

LingoPop does not include any AI or translation service of its own. It sends your text to the provider you choose, using your own API key. An API key is a secret string that identifies your account at that provider.

- Enter keys in LingoPop: tray icon, then **Settings → Providers**, select a provider, paste the key, click **Save**.
- Keys are stored only on your PC, in a DPAPI-protected file (`%APPDATA%\LingoPop\secrets.dat`). They are never synced anywhere.
- Never share a key, post it in an issue, or commit it to a repository. If a key leaks, delete it in the provider's console and create a new one.
- Providers bill you directly for what you use. LingoPop does not see or charge for it. Where the provider supports it, set a spending limit before you start.

Provider consoles change often. This page was checked against the providers' official documentation on 2026-10-15. If a button or menu is named differently when you get there, follow the "Official docs" link under the provider.

## Summary

| Provider | Key needed | Cost note |
|---|---|---|
| [Claude](#claude) | Yes | Paid, billed by usage |
| [OpenAI](#openai) | Yes | Paid, billed by usage |
| [Gemini](#gemini) | Yes | Free tier with rate limits, paid tier optional |
| [Grok](#grok) | Yes | Prepaid credits |
| [Google Free](#google-free) | No | Free, unofficial, no guarantees |
| [Google Translate](#google-translate) | Yes | Needs a Cloud project with billing enabled |
| [DeepL](#deepl) | Yes | Free and paid API plans |
| [Microsoft Translator](#microsoft-translator) | Yes | Free F0 tier (2M characters per month) or paid |

What each provider can do in LingoPop:

| Provider | Translate | Fix grammar | Rewrite | Explain | Magic | Screen Translate (OCR text) | Screen Translate vision Retry |
|---|---|---|---|---|---|---|---|
| Claude | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| OpenAI | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Gemini | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Grok | Yes | Yes | Yes | Yes | Yes | Yes | Yes |
| Google Free | Yes | No | No | No | No | Yes | No |
| Google Translate | Yes | No | No | No | No | Yes | No |
| DeepL | Yes | No | No | No | No | Yes | No |
| Microsoft Translator | Yes | No | No | No | No | Yes | No |

If you only need translation, any provider works. For Fix grammar, Rewrite, Explain and Magic, use Claude, OpenAI, Gemini or Grok.

## Claude

Claude is Anthropic's model family. You use it through the Claude Console (the developer platform), which is separate from a claude.ai chat subscription. Usage is billed to your Console organization.

1. Sign in or create an account at the [Claude Console](https://platform.claude.com/).
2. Make sure billing is set up for your organization on the [Billing](https://platform.claude.com/settings/billing) page.
3. Open the [API keys](https://platform.claude.com/settings/keys) page.
4. Click **Create key**, give it a name (for example `LingoPop`), choose an expiration, and set **Linked account** to yourself. A personal key is the right choice for your own use.
5. Copy the key. It starts with `sk-ant-` and is shown only once. If you lose it, create a new one.
6. Recommended: on the Billing page, in **Spend limits**, click **Set limit** (or **Adjust limit**) and enter a monthly amount.

In LingoPop:

1. Open **Settings → Providers** and select **Claude**.
2. Paste the key into **API key**.
3. Click **Save**.
4. Click **Set as active** to use Claude for your hotkeys.

Troubleshooting:

- 401 or 403: the key is wrong, was deleted, or has expired. Create a new key and paste it again without spaces.
- 429: either a rate limit (wait and retry) or the organization's monthly spend cap for its usage tier. The spend-cap error carries no retry delay and lasts until the next month or a tier increase.
- 400 mentioning "specified API usage limits": you reached the spend limit you set yourself. Raise or remove it on the Billing page.

Official docs: [Get your Claude API key](https://platform.claude.com/docs/en/get-api-key), [Rate limits and spend limits](https://platform.claude.com/docs/en/api/rate-limits)

## OpenAI

OpenAI's API is billed separately from a ChatGPT subscription. Keys belong to a project inside your organization.

1. Sign in or create an account at the [OpenAI API platform](https://platform.openai.com/).
2. Add billing information. OpenAI's usage tiers unlock as your total credit purchases grow (the first paid tier needs $5 in credit purchases). Usage tiers are described in the rate limits guide linked below.
3. Open the [API keys](https://platform.openai.com/settings/organization/api-keys) page. To create a key for a specific project, open that project's settings and select its API keys page instead.
4. Create a new secret key (the button is labeled **Create new secret key** in OpenAI's help center and **Create an API Key** in the quickstart). Name it `LingoPop`, pick the project, and set an expiration if offered.
5. Copy the key right away. It is shown only once. OpenAI secret keys typically start with `sk-`.
6. Recommended: open [Organization limits](https://platform.openai.com/settings/organization/limits), in **Spend** select **Edit spend limit**, enter a **Monthly spend limit**, optionally turn on **Enforce a hard limit**, and click **Save**.

In LingoPop:

1. Open **Settings → Providers** and select **OpenAI**.
2. Paste the key into **API key**.
3. Click **Save**.
4. Click **Set as active**.

Troubleshooting:

- 401: the key is wrong or revoked. Create a new key. Revoked keys stop working within seconds.
- 403 or "model not found": the key's project does not have access to the selected model. Check the project's settings or pick another model in LingoPop.
- 429: rate limit, or a hard spend limit was reached (`organization_spend_limit_exceeded` or `project_spend_limit_exceeded`). A hard-limit error will not clear on retry. Raise the limit or add credit.

Official docs: [Developer quickstart](https://developers.openai.com/api/docs/quickstart), [Managing projects](https://help.openai.com/en/articles/9186755-managing-projects-in-the-api-platform), [Rate limits and usage tiers](https://developers.openai.com/api/docs/guides/rate-limits), [Spend limits](https://developers.openai.com/api/docs/guides/spend-limits)

## Gemini

Gemini keys come from Google AI Studio. New accounts start on a free tier that covers certain models with lower rate limits. The paid tier needs a linked billing account.

1. Open [Google AI Studio API keys](https://aistudio.google.com/apikey) and sign in with a Google account. Accept the terms if asked. New users get a default project and key automatically.
2. If you already use Google Cloud and your project is not listed, open the **Dashboard**, go to **Projects**, click **Import projects**, select the project and click **Import**.
3. On the API keys page, click **Create API key** and pick the project.
4. If the key is labeled **Unrestricted**, add a restriction: under **Add restrictions** choose **Restrict to Gemini API only**, then click **Restrict key**. Google's docs say unrestricted standard keys are rejected by the Gemini API. Keys created in AI Studio since 2026-05-28 are created as authorization keys by default.
5. Copy the key and keep it private.
6. Optional: for higher limits, use **Set up billing** next to your project. A project-level **Monthly spend cap** can be set under **Edit spend cap** on the AI Studio Spend page (Google marks this feature as experimental).

In LingoPop:

1. Open **Settings → Providers** and select **Gemini**.
2. Paste the key into **API key**.
3. Click **Save**.
4. Click **Set as active**.

Troubleshooting:

- 400, 401 or 403: the key is wrong, restricted to a different API, or shown as **Blocked** in AI Studio (a long-unused unrestricted key). Create a new restricted key.
- 429 (`RESOURCE_EXHAUSTED`): you exceeded the rate limit of the free or paid tier. Wait, or check your limits at [aistudio.google.com/rate-limit](https://aistudio.google.com/rate-limit). Limits apply per project, not per key.
- "You do not have permission to create a key in this project": ask a project admin for a role such as Project Editor, or create a new Google Cloud project that is not tied to an organization.

Official docs: [Using Gemini API keys](https://ai.google.dev/gemini-api/docs/api-key), [Billing](https://ai.google.dev/gemini-api/docs/billing), [Rate limits](https://ai.google.dev/gemini-api/docs/rate-limits)

## Grok

Grok is xAI's model family. The xAI API is billed separately from Grok app subscriptions. Usage is paid from prepaid credits unless your team has monthly invoicing enabled.

1. Sign up or log in at the [xAI Console](https://console.x.ai/).
2. Add credits: open [Billing](https://console.x.ai/team/default/billing) and go to **API spend management** to buy credits. Make sure you are in the right team first.
3. Open the [API Keys](https://console.x.ai/team/default/api-keys) page and create an API key. xAI's docs do not name the button, so use the create control on that page.
4. Copy the key. Treat it as shown once.
5. Recommended: use the auto top-up settings on the Billing page (trigger balance, top-up amount, monthly maximum) so that spending has a monthly ceiling. By default the invoiced billing limit is $0, so requests are rejected once prepaid credits run out.

In LingoPop:

1. Open **Settings → Providers** and select **Grok**.
2. Paste the key into **API key**.
3. Click **Save**.
4. Click **Set as active**.

Troubleshooting:

- 401 or 403: the key is wrong or from a different team than the one holding your credits. Create a new key.
- Requests rejected after working earlier: prepaid credits are depleted. Buy more credits on the Billing page.
- 429: rate limit for your team's tier. Wait and retry. Tiers rise automatically with cumulative spend.

Official docs: [xAI quickstart](https://docs.x.ai/developers/quickstart), [Billing, credits and invoices](https://docs.x.ai/console/billing), [Rate limits](https://docs.x.ai/developers/rate-limits)

## Google Free

Google Free needs no account and no key. It uses the public endpoint that powers Google Translate on the web (`translate.googleapis.com/translate_a/single`). This endpoint is unofficial and undocumented by Google.

Limits to know about:

- Translate and Screen Translate (on OCR text) only. No Fix grammar, Rewrite, Explain or Magic.
- Google can rate-limit it, block it, or change it at any time without notice.
- No service guarantee. For reliable daily use, choose a provider with a key and, if you like, set Google Free as the fallback.

In LingoPop:

1. Open **Settings → Providers** and select **Google (Free)**.
2. Leave the key empty. There is no key field.
3. Click **Set as active**.

Troubleshooting:

- 429 or "too many requests": Google is throttling your IP address. Wait a while, translate shorter texts, or switch to a keyed provider.
- Other 4xx or 5xx or unexpected output: the unofficial endpoint may be down or have changed. Switch provider, and check [Issues](https://github.com/slucheninov/lingopop-app/issues) for known problems.

Official docs: none, because this is not an official API. The official, keyed Google option is [Google Translate](#google-translate) below.

## Google Translate

This is the official Google Cloud Translation API, Basic edition (v2). You need a Google Cloud project with billing enabled. See the pricing page for the free allowance and per-character price, since Google's pages word the free allowance differently.

1. Select or create a project in the [Google Cloud project selector](https://console.cloud.google.com/projectselector2/home/dashboard). Creating one needs the Project Creator role.
2. Make sure [billing](https://console.cloud.google.com/billing) is enabled for that project.
3. Enable the API: [Enable Cloud Translation API](https://console.cloud.google.com/apis/enableflow?apiid=translate.googleapis.com). You need the Owner or Service Usage Admin role.
4. Open the [Credentials](https://console.cloud.google.com/apis/credentials) page (APIs & Services), click **Create credentials**, and select **API key**. Click **Create**.
5. Restrict the key. Click the key's name, and under **API restrictions** click **Restrict key**, select **Cloud Translation API**, and click **Save**. Google recommends restricting every key.
6. Optional, to limit where the key can be used: under **Application restrictions** select **Websites**, click **Add**, enter a referrer pattern such as `example.com/*`, click **Done**, then **Save**.
7. Optional: character usage is unlimited by default. Set quotas for the API in the Cloud console to cap spending.

In LingoPop:

1. Open **Settings → Providers** and select **Google Translate**.
2. Paste the key into **API key**.
3. If you added a website restriction in step 6, enter a URL that matches it in **Referer**, for example `https://example.com/`. Leave **Referer** empty if you did not.
4. Click **Save**, then **Set as active**.

LingoPop sends the key in the `X-goog-api-key` header, not in the URL. Google documents this header for API keys in general. The Translation v2 reference page itself only shows the `key` query parameter and does not describe how a referrer restriction is checked for a header-based request. If a website-restricted key is rejected, remove the website restriction and keep the API restriction.

Troubleshooting:

- 400, 401 or 403 "API key not valid" or "API has not been used in project": the key is wrong, restricted to a different API, does not match the referrer, or Cloud Translation API is not enabled in that project.
- 403 about billing: billing is not enabled for the project. Link a billing account.
- 429: a quota or rate limit for the project was exceeded. Check the quotas for Cloud Translation in the Cloud console.

Official docs: [Cloud Translation setup](https://docs.cloud.google.com/translate/docs/setup), [Create and restrict API keys](https://docs.cloud.google.com/docs/authentication/api-keys), [Use an API key with REST](https://docs.cloud.google.com/docs/authentication/api-keys-use), [Pricing](https://cloud.google.com/translate/pricing)

## DeepL

DeepL is a dedicated translation service. The DeepL API has a free plan and paid Pro plans. It is separate from a DeepL Translator web account. Free plan limits and availability change, so check the plans page before you sign up.

1. Go to the [DeepL API plans](https://www.deepl.com/pro-api#api-pricing) page, choose a plan, and sign up. DeepL's developer docs note that if you only have a DeepL Translator account you may need to create a separate account for the API.
2. Open your [account's API Keys page](https://www.deepl.com/your-account/keys) (the **API Keys** tab in your account).
3. Copy your authentication key.
4. Free plan keys end with `:fx`. Pro keys do not.

In LingoPop:

1. Open **Settings → Providers** and select **DeepL**.
2. Paste the key into **API key**.
3. Leave **Base URL** empty. LingoPop uses `api-free.deepl.com` for keys ending in `:fx` and `api.deepl.com` for all other keys.
4. Click **Save**, then **Set as active**.

Troubleshooting:

- 403: the key is wrong, or it is used with the wrong host. A free key must use `api-free.deepl.com` and a Pro key `api.deepl.com`. If you typed a Base URL, clear it.
- 456: your character quota or a cost-control limit is used up. It will not clear on retry until the period resets or the limit is raised in your DeepL account.
- 429: too many requests in a short time. Wait and retry.

Official docs: [Authentication](https://developers.deepl.com/docs/getting-started/auth), [Getting started](https://developers.deepl.com/docs/getting-started/quickstart), [Error handling](https://developers.deepl.com/docs/best-practices/error-handling)

## Microsoft Translator

This is Azure Translator (now listed in Microsoft's documentation as part of Foundry Tools). You need an Azure account and a Translator resource. Each Azure subscription can have one free-tier resource (F0), which includes 2 million characters per month. Paid tiers are billed by character.

1. Sign in to the [Azure portal](https://portal.azure.com/) with an [Azure account](https://azure.microsoft.com/pricing/purchase-options/azure-account).
2. Open [Create Translator resource](https://portal.azure.com/#create/Microsoft.CognitiveServicesTextTranslation) (the single-service Translator).
3. Fill in the **Basics** tab: **Subscription**, **Resource Group**, **Resource Region**, **Name**, and **Pricing tier**. Microsoft recommends **Global** for the region unless you need a specific one. Choose the free tier (F0) if you are starting out.
4. Click **Review + Create**, then **Create**. When deployment finishes, click **Go to resource**.
5. In the left pane, under **Resource Management**, select **Keys and Endpoint**.
6. Copy one of the two keys. Note the region of the resource (the one you chose in step 3).

In LingoPop:

1. Open **Settings → Providers** and select **Microsoft Translator**.
2. Paste the key into **API key**.
3. Set **Region**:
   - Global resource: leave it empty.
   - Regional resource (for example you chose `West Europe`) or a multi-service resource: enter the resource's region name, for example `westeurope`. A multi-service key only works in the region it was created in.
4. Click **Save**, then **Set as active**.

Troubleshooting:

- 401: the key is wrong, or the **Region** is missing or does not match the resource. Regional and multi-service resources need the region.
- 403 or 429: you reached the F0 monthly character quota or a rate limit. Wait, upgrade the pricing tier, or create a paid resource.
- Cannot create a second free resource: only one free-tier Translator resource is allowed per subscription. Reuse the first key.

Official docs: [Create and configure Azure resources for Translator](https://learn.microsoft.com/en-us/azure/ai-services/translator/create-translator-resource), [Authentication and authorization](https://learn.microsoft.com/en-us/azure/ai-services/translator/text-translation/reference/authentication), [Translator pricing](https://azure.microsoft.com/en-us/pricing/details/translator/)
