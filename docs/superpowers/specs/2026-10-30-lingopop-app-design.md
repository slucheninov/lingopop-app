# lingopop-app: дизайн Windows-версии LingoPop

Дата: 2026-10-30. Статус: утверждённый дизайн, код ещё не написан.

Пометка «(решение ассистента)» означает, что пункт выбран автором документа, а не владельцем проекта. Всё остальное решил владелец. Такие пункты можно пересматривать без согласования.

## 1. Зачем и что это

- **lingopop-app** - версия LingoPop для Windows: приложение в системном трее с AI-переводчиком. Пользователь выделяет текст в любом приложении, нажимает глобальный хоткей, рядом с курсором появляется popup с результатом операции.
- Отдельный репозиторий рядом с macOS-репозиторием `slucheninov/lingopop` (локально `~/Works/slava/lingo-pop`): `github.com/slucheninov/lingopop-app`, публичный.
- Стек: **Tauri v2** и OS-agnostic **Rust core**. Цель: позже перевести macOS-приложение на ту же кодовую базу.
- Swift macOS-приложение остаётся основным продуктом на macOS. Сборка Tauri под macOS не поставляется.
- Linux явно не целевая платформа. Архитектура его не запрещает, но ничего для него не делается и не тестируется.

## 2. Реальность разработки

- У владельца нет Windows-машины. Ручное тестирование делают внешние тестеры.
- Поэтому тяжёлое автоматическое тестирование здесь базовое требование, а не пожелание. Всё, что можно проверить без человека, проверяется в CI.
- Для самопроверок владельца: **Windows 11 ARM VM в UTM на его Mac** (решение ассистента). Она запускает x64-сборку через эмуляцию и arm64-сборку нативно. Глобальные хоткеи, буфер обмена и OCR в VM проверяются так же, как на железе. Мультимонитор и смешанный DPI в VM проверяются плохо, это остаётся тестерам.
- Сборка для разработки на macOS (см. раздел 5, реализация `macos`) нужна только для итераций над UI и core без VM.

## 3. Цели и не-цели

### Цели

Полный паритет с macOS v0.4.0 уже в **первой тестовой сборке**. Переносится всё, что описано в `AGENTS.md` macOS-репозитория, кроме чисто macOS-механик.

- **Операции** (`AIOperation`): Translate (auto-detect + `languagePairs`), Fix grammar, Rewrite, Explain, Screen Translate, а также Magic (кастомный промпт; на macOS это отдельная операция и вкладка Magic, см. раздел 7).
- **Screen Translate**: overlay выбора области на всех мониторах, затем фаза OCR (`.ocr`) с кнопками Copy / Translate / Retry. Retry отправляет изображение напрямую vision-провайдерам (Claude, OpenAI, Gemini, Grok) для OCR + перевода за один шаг. Текст OCR и результат перевода автоматически копируются в буфер (`autoCopyOCR`).
- **Хоткеи**: свой глобальный хоткей на каждую операцию.
- **Popup**: Copy / Replace / Retry-with-other-provider / Rewrite / Explain. Replace скрыт для результатов Screen Translate. Режимы подписей action row `full` / `mixed` / `compact` с порогом 120 символов (`ActionRowMetrics`, логика портируется как есть).
- **8 провайдеров**: Claude, OpenAI, Gemini, Grok, Google Free, Google Translate, DeepL, Microsoft Translator. Те же `supportedOperations`, `supportsVision`, `settingsFields`, `autoBaseURL`. Fallback на уровне провайдера срабатывает при 429, 5xx и сетевых ошибках, один раз.
- **История**: окно истории и последние записи в меню трея. **Usage**: вкладка с графиками, оценка стоимости (`UsagePricing`).
- **Settings**: вкладки Main / Providers / Shortcuts / Usage / Magic / About. About включает проверку обновлений (аналог `UpdateChecker`, но по релизам `slucheninov/lingopop-app`).
- **Локализация**: UI на en/pl/uk/de, выбор языка в приложении (System / en / pl / uk / de).
- Автозапуск при входе в систему, иконка трея с состояниями idle / working / error.

### Не-цели

- Linux.
- Поставка Tauri-сборки на macOS сейчас.
- Любая синхронизация: ни iCloud, ни облачной папки. История и usage хранятся только локально.
- Подпись кода в v1. Установщик без подписи, предупреждение SmartScreen принимается. Azure Trusted Signing можно добавить позже.
- Microsoft Store.

### Анти-цели (провал, даже если всё остальное работает)

- Popup забирает фокус у приложения пользователя.
- Содержимое буфера пользователя теряется после capture или Replace.
- API-ключи или настройки провайдеров уходят куда-либо, кроме самого провайдера.
- Windows-специфичный код протекает в core crate.
- Логика, которую можно проверить только вручную на Windows.

## 4. Ограничения и стек

- **Tauri v2**, Rust stable.
- Frontend: **TypeScript + Svelte 5 + Vite** (решение ассистента).
- ОС: Windows 10 1809+ и Windows 11, **x64 обязателен**; arm64-сборка желательна (решение ассистента).
- WebView2: установщик подтягивает через bootstrapper.
- Установщик: **NSIS** (решение ассистента).
- Версии независимы от macOS-приложения, старт с **0.1.0** (решение ассистента).

## 5. Архитектура

Cargo workspace:

```
lingopop-app/
  crates/
    lingopop-core/       # чистый Rust, без OS API
    lingopop-platform/   # traits + реализации windows / macos / fake
  app/
    src-tauri/           # Tauri host, команды, окна, трей
    src/                 # frontend (Svelte 5 + TS)
  scripts/               # локализация, релиз
  docs/
```

### crates/lingopop-core

Чистый Rust. Никаких OS API. Должен собираться и тестироваться на Windows и macOS.

- **Providers**: trait `AIProvider` и 8 реализаций на `reqwest` + `rustls`.
  - Один общий client (аналог `ProviderHTTP`): таймауты 30 с на запрос и 60 с суммарно.
  - Запрет cross-origin redirect (ключи не должны уходить на другой хост), только https для base URL.
  - Заголовки как в macOS-версии, например ключ Google Cloud в `X-goog-api-key` (не в URL), опциональный `Referer`.
  - Эндпоинты и форматы по `Providers/*.swift` macOS-репозитория; поведение должно совпадать.
- **Operations и fallback**: `run(operation, ...)` с валидацией провайдера (`supportedOperations`, наличие ключа) и одним fallback на `settings.fallback(for:)` при 429, 5xx, network. Ошибки без повтора: invalid URL, missing key, decoding, empty response, operation not supported, invalid configuration, provider not found. Источник: `Core/OperationRunner.swift`.
- **Prompts**: тексты из `Core/PromptBuilder.swift` переносятся дословно, вместе с `userContent` (экранирование `&`, `<`, `>`) и `sanitizeOutput`. Пока копия, без общего источника (решение ассистента).
- **Languages**: таблица из `Models/Languages.swift` (name, code, `deepLCode`, `microsoftCode`, quick list), коды из `Models/LanguageCodes.swift`.
- **Language detection**: через Rust-крейт на выбор исполнителя (например `whatlang` или `lingua`). Результат маппится на имя из таблицы `Languages`.
- **Target-language resolution**: аналог `SettingsStore.resolveTargetLanguage`: auto-detect только для `.translate`, поиск в `languagePairs`, иначе `targetLanguage`.
- **Settings model + JSON persistence**: все поля из `SettingsStore.Persisted`, кроме iCloud-полей. Миграций нет, это первая версия формата.
- **History и usage stores**: порт envelope v2 и логики `merge` из `Core/CloudMerge.swift` (tombstones, `clearedBefore`, подрезка tombstones старше 90 дней). Merge сохраняется, хотя синхронизации нет, чтобы формат остался совместимым. Лимит истории `maxHistoryCount`, retention usage 31 день.
- **Secrets**: payload (плоская `[String:String]`-карта) и trait `SecretProtector` (шифрование/расшифровка байтов). Реализация в platform-crate.
- **Action-row metrics**: порт `Core/ActionRowMetrics.swift`.
- **Errors**: отображение ошибок в тексты для пользователя (`userSummary` / `userDetail`), включая очистку тела HTTP-ответа (`sanitizedHTTPBodyDetail`, `redactedURL`).
- **Localization lookup**: поиск строки по ключу и языку, подстановка `%@` / `%lld`-аргументов.

### crates/lingopop-platform

Traits:

- `HotkeyService`
- `SelectionCapture`
- `TextInjector` (вставка обратно в приложение)
- `ClipboardSnapshot`
- `ForegroundWindow` (запомнить / восстановить)
- `CursorAndMonitors` (позиция курсора, рабочие области мониторов, DPI на монитор)
- `ScreenRegionCapture`
- `Ocr`
- `SecretProtector` (реализация)
- `Autostart`

Реализации:

- `windows`: настоящая, поставляется.
- `macos`: минимальная, только для разработки, чтобы владелец мог запускать приложение на своём Mac. Не поставляется (решение ассистента).
- `fake`: для тестов. Все сценарии оркестрации тестируются на `fake` без ОС.

### app/ (Tauri)

- Окна: popup, settings, history, capture overlay (по одному на монитор).
- Меню трея повторяет status menu macOS-приложения: недавняя история, операции, подменю target language, History / Settings / About / Quit.
- Оркестрация (hotkey → capture → resolve → run → popup) живёт в Rust, а не во frontend. Frontend только отображает состояние и шлёт действия.

## 6. Windows-специфика (заметки по реализации)

- **Хоткеи**: крейт `global-hotkey` / `RegisterHotKey`. Формат хранения хоткея свой (код клавиши + модификаторы Windows). Формат macOS (`HotkeyConfig`: Carbon keyCode + `NSEvent.ModifierFlags`) не переносится, настройки между платформами несовместимы и не должны быть.
- **Захват выделения**:
  1. снимок всех форматов буфера;
  2. дождаться отпускания модификаторов хоткея пользователя (или синтезировать key-up);
  3. `SendInput` Ctrl+C;
  4. опрос `GetClipboardSequenceNumber` до 300 мс;
  5. прочитать текст;
  6. восстановить снимок буфера.
- **UIPI**: целевое приложение с повышенными правами не получит `SendInput`. Показывать понятную ошибку для пользователя, а не молча ничего не делать.
- **Popup**: `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`. Позиция рядом с курсором, ограничение рабочей областью монитора. Закрытие по клику вне и по Esc, фокус не забирается.
- **Replace**: восстановить foreground window, положить текст в буфер, `SendInput` Ctrl+V, восстановить буфер пользователя.
- **Захват экрана**: корректен при смешанном DPI (Per-Monitor V2). Выбор API (`Windows.Graphics.Capture` или GDI `BitBlt`) за исполнителем.
- **OCR**: `Windows.Media.Ocr` через крейт `windows`. Язык берётся из профиля пользователя. Нет языкового пакета OCR: ошибка с инструкцией, как его поставить. Пустой результат: предложить vision-retry.
- **Секреты**: DPAPI `CryptProtectData` (область CurrentUser) защищает тот же плоский JSON-payload. Файл `%APPDATA%\LingoPop\secrets.dat`.
- **Файлы данных** в `%APPDATA%\LingoPop\`: `settings.json`, `history.json`, `usage.json`, `secrets.dat`.
- **Автозапуск**: HKCU Run через `tauri-plugin-autostart`.
- **Один экземпляр**: `tauri-plugin-single-instance`.
- Разделы про разрешения Accessibility и Screen Recording в Settings → Main на Windows скрываются, права не нужны.

## 7. Совместимость данных

Факты проверены по исходникам macOS-репозитория (v0.4.0, коммит `049acf3`).

### history.json

Файл: envelope v2 (`Core/CloudMerge.swift`, `MergeableEnvelope`):

```json
{ "version": 2, "items": [...], "deleted": [{"id": "<UUID>", "at": <date>}], "clearedBefore": <date> }
```

- `version`: Int (2). `items`: массив. `deleted`: массив tombstone `{id: UUID, at: Date}`. `clearedBefore`: Date, поле опускается, когда nil.
- Legacy-формат (v0.3.16 и старше): голый массив `[Item]`. Читается, при следующей записи переписывается в envelope.
- Запись истории (`Models/TranslationHistory.swift`, `TranslationHistoryEntry`):
  - `id`: UUID (строка)
  - `original`: String
  - `translated`: String
  - `sourceLanguage`: String? (поле опускается при nil)
  - `targetLanguage`: String
  - `provider`: String (raw value `ProviderType`)
  - `operation`: String (raw value `AIOperation`)
  - `timestamp`: Date
- Порядок: новые первыми, обрезка до `maxHistoryCount` (без tombstone).

### usage.json

Тот же envelope v2. Запись (`Core/UsageStore.swift`, `UsageStore.Record`):

- `id`: UUID
- `timestamp`: Date
- `provider`: String (`ProviderType`)
- `model`: String
- `operation`: String (`AIOperation`)
- `inputTokens`: Int
- `outputTokens`: Int

Хранение 31 день, порядок от старых к новым.

### Кодирование дат

В коде нет `dateEncodingStrategy` / `dateDecodingStrategy` (grep по `App`, `Core`, `Models`, `Providers`, `UI`). Используются дефолтные `JSONEncoder()` / `JSONDecoder()`. Значит, **Date кодируется числом (Double) секунд с 2001-01-01T00:00:00Z** (`timeIntervalSinceReferenceDate`, с дробной частью). Это не ISO 8601 и не Unix epoch. Для Unix epoch нужно прибавить 978307200 с. Rust-реализация обязана читать и писать именно этот формат (серде-обёртка для `f64`).

Остальные подробности дефолтного Codable: UUID кодируется строкой в верхнем регистре; ключи в `history.json` и `usage.json` не сортируются (`sortedKeys` не задан), поэтому совместимость на уровне схемы, а не побайтная. Windows-версия может писать ключи в любом порядке; читать должна любой регистр UUID.

### Raw values

- `ProviderType` (`Models/ProviderConfig.swift`), заморожены, не переименовывать: `"Claude"`, `"OpenAI"`, `"Gemini"`, `"Grok"`, `"Google (Free)"`, `"Google Translate"`, `"DeepL"`, `"Microsoft Translator"`.
- `AIOperation` (`Models/AIOperation.swift`): `"translate"`, `"grammarFix"`, `"rewrite"`, `"explain"`, `"screenTranslate"`, `"magic"`. Шестая операция `magic` есть в коде и в хоткеях (вкладка Magic); в списке паритета она учтена.
- UI показывает `displayName`, никогда `rawValue`.

### secrets.dat

- Плейнтекст (`Models/ProviderSettingsStorage.swift`, `SecretsPayload`): плоский JSON-объект `[String: String]`, ключи отсортированы при записи.
- Ключи API: `apiKey.<ProviderType.rawValue>`.
- Настройки провайдеров: одна запись `providerSettings.v1` со значением-строкой `{"version":1,"providers":{"<rawValue>":{"baseURL":...,"model":...,"referer":...,"region":...}}}`.
- Неизвестные записи при перезаписи сохраняются. Форму контейнера не менять.
- На macOS файл зашифрован AES-GCM (`Core/SecretStorage.swift`), на Windows будет DPAPI. Шифрованные файлы между платформами несовместимы и не должны копироваться; совместим только формат payload.

### Промпты и локализации

- Тексты промптов: `Core/PromptBuilder.swift` (включая `defaultMagicPrompt` и токен `{{targetLanguage}}`). Остаются английскими.
- Локализации: `App/Localizable.xcstrings`, 4 языка: en (источник), pl, uk, de. Правила проверки: `scripts/check-localizations.py`.

### Итог

- `history.json` и `usage.json` совместимы по схеме с macOS-приложением: те же поля, то же кодирование дат, те же raw values, envelope v2. Хранятся только локально.
- Слова владельца: «да, та же история и статистика, только без iCloud Drive, хранить просто локально». Интерпретация: те же функции и совместимый формат, без синхронизации. Совместимый формат нужен, чтобы синк можно было добавить позже (решение ассистента).
- Persisted-идентификаторы (raw values) не переименовываются никогда.

## 8. Локализация

- Строки импортируются скриптом из `App/Localizable.xcstrings` macOS-репозитория в JSON для frontend и отдельно для Rust (строки трея и нативных окон).
- CI-проверка с теми же правилами, что `scripts/check-localizations.py`: все ключи без `shouldTranslate: false` переведены на pl/uk/de; мультимножество спецификаторов формата (`%@`, `%lld`, `%1$@`, `%%`), переводы строк и спецсимволы (`…`, `→`, `⌘`, `{{targetLanguage}}`) совпадают с ключом.
- Нелокализуемое: `rawValue`, названия языков из таблицы `Languages` (это данные), тексты промптов, логи, HTTP/JSON-поля, бренды провайдеров.
- Переключение языка применяется сразу, без перезапуска (решение ассистента; на macOS нужен перезапуск, на Windows это дешевле).
- Строки, которые есть только на macOS (⌘, Accessibility, Screen Recording), на Windows не показываются; ключи, неприменимые к Windows, помечаются в скрипте импорта.

## 9. Тестирование

Четыре слоя.

### a) Core unit tests (CI: `windows-latest` и `macos-latest`)

- Провайдеры против HTTP mock-сервера с записанными fixtures ответов, включая ошибки 429 и 5xx для проверки fallback.
- Merge истории и usage (union, tombstones, `clearedBefore`, подрезка 90 дней, legacy bare array).
- Секреты (payload, `SecretProtector` через fake), загрузка settings (без миграций), разрешение target language, action-row metrics, кодирование дат в формате 2001-эпохи (тесты на фикстурах, сохранённых macOS-приложением).
- Оркестрация целиком на `fake` platform.

### b) Platform smoke tests (`windows-latest`)

- Регистрация и снятие хоткея, снимок и восстановление буфера (roundtrip), `SendInput` копирование в тестовое окно, захват известного окна, OCR фикстурного PNG.
- Исполнитель должен проверить, даёт ли GitHub Windows runner интерактивный рабочий стол, и пометить тесты, которым он нужен (`#[ignore]` / отдельная feature), чтобы не получить ложные падения.

### c) Frontend unit tests (Vitest)

Компоненты popup, режимы action row, формы Settings, переключение языка.

### d) Ручной чек-лист для тестеров

- `docs/testing/manual-checklist.md`: сценарии для Notepad, Word, Chrome, Telegram, приложения с повышенными правами, мультимонитора и смешанного DPI, каждого провайдера.
- Установщик из CI-артефакта на каждый PR.

E2E через `tauri-driver` по желанию, на усмотрение исполнителя.

## 10. Репозиторий

Зеркалит структуру macOS-репозитория. Каждый файл ниже берётся из соответствующего файла macOS-репозитория и адаптируется под Windows/Tauri:

- `LICENSE` (MIT), `README.md`, `SECURITY.md`, `PRIVACY.md` (описать Windows-хранение: DPAPI, `%APPDATA%`, отсутствие синхронизации), `CONTRIBUTING.md`, `CHANGELOG.md`, `RELEASING.md`.
- `AGENTS.md` как единственный источник инструкций для агентов и `CLAUDE.md`, указывающий на него (тот же паттерн, что в macOS-репозитории: `CLAUDE.md` короткий и импортирует `AGENTS.md`).
- `.github/CODEOWNERS` (`* @slucheninov`), issue templates (`bug_report.yml`, `feature_request.yml`, `config.yml`), `pull_request_template.md`.
- `.github/dependabot.yml`: экосистемы cargo, npm, github-actions (в macOS-репозитории сейчас только github-actions, это расширение).
- `.coderabbit.yaml`.
- `docs/api-keys.md` (адаптировать, якоря разделов сохранить: claude, openai, gemini, grok, google-free, google-translate, deepl, microsoft-translator) и иконка из `docs/icon.png` macOS-репозитория.

### CI

- `ci.yml`, обязательная проверка `build-and-test`: `cargo fmt --check`, `clippy -D warnings`, `cargo test` (core на windows и macos, остальное на windows), lint и тесты frontend, проверка локализации, сборка NSIS-установщика как артефакта. Действия GitHub закрепляются по SHA, как в macOS-репозитории.
- `release.yml`: тег `v*` → сборка на `windows-latest` → GitHub Release с установщиком и `SHA256SUMS` через `GITHUB_TOKEN`.
- Аналог `scripts/publish-release.sh`: `prepare <version>` (ветка + PR с бампом версии) и `tag <version>` (тег на `origin/main`, ожидание workflow, проверка артефактов).

### Настройки репозитория (как в macOS-репозитории)

- `main` только через PR; обязательны проверка `build-and-test` и approval CodeRabbit.
- Secret scanning + push protection, Dependabot security updates.

### Bootstrap

Репозиторий пуст, поэтому самые первые коммиты идут прямо в `main`. После этого включается защита ветки, и всё остальное идёт через PR (решение ассистента).

## 11. Оставлено исполнителю

- Точный выбор крейтов: определение языка (`whatlang` / `lingua` / иное), HTTP-мокирование, API захвата экрана.
- Детали стиля UI: следовать раскладке и поведению macOS-приложения, адаптируя под вид Windows 11 (решение ассистента).
- Инструмент E2E.
- arm64-сборка.
- Дефолтные хоткеи на Windows (на macOS это Option+T для Translate); выбрать так, чтобы не конфликтовать с типичными системными сочетаниями.
