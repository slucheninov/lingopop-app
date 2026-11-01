# Фаза 4b: окна Settings и History, меню трея, overlay Screen Translate, локализация, usage, автозапуск, проверка обновлений

Дата: 2026-11-01. Статус: проект дизайна, код ещё не написан. Продолжает общий дизайн [2026-10-30-lingopop-app-design.md](2026-10-30-lingopop-app-design.md) (раздел «Разбиение», Фаза 4b) и опирается на Фазу 4a ([plans/2026-10-31-phase4a-orchestration-popup.md](../plans/2026-10-31-phase4a-orchestration-popup.md)).

Пометки: **«решение владельца»** - принято владельцем проекта; **«решение ассистента»** - выбрано автором документа и может быть пересмотрено без согласования. Пункты без пометки следуют из принципа владельца (ниже) или из существующего кода.

Пути вида `UI/SettingsView.swift:22` относятся к macOS-репозиторию `~/Works/slava/lingo-pop` (только чтение). Пути без префикса - этот репозиторий.

## 1. Зачем и принцип

После Фазы 4a приложение умеет только хоткей → popup. Всё остальное, что у macOS-версии доступно из меню в строке меню, на Windows отсутствует: нет настроек (ключи API можно положить только руками в `secrets.dat`, что невозможно), нет истории, трей имеет один пункт Quit, Screen Translate не имеет overlay и не имеет хоткея, usage не виден, язык интерфейса только английский. Без Фазы 4b продукт нельзя отдать тестерам.

**Принцип владельца: macOS-приложение - это спецификация.** Копируем проверенное поведение, раскладку, тексты и порядок элементов; отступаем только там, где этого требует Windows. Каждое такое отступление в этом документе названо «отступление» и обосновано.

## 2. Решения владельца и ассистента (сводка)

Решения владельца:

1. **Окно Settings полное, как на macOS, с первого релиза** (все шесть вкладок, все контролы), а не минимум.
2. **UI macOS - база UI Windows**: раскладка, контролы, тексты, порядок.
3. **Проверка обновлений** в About: релизы GitHub `slucheninov/lingopop-app`; пре-релизы игнорируются. `GET https://api.github.com/repos/slucheninov/lingopop-app/releases/latest` по определению не возвращает пре-релизы и черновики; 404 / нет релиза = ничего не показываем; ошибки молча, как на macOS. Кнопка Download открывает `https://github.com/slucheninov/lingopop-app/releases/latest`.

Решения ассистента (каждое повторено в своём разделе):

- a. Из macOS-UI убираются: секция iCloud, секции Accessibility / Screen Recording и `PermissionAlert`, скрытое app-меню (§3.2).
- b. Автозапуск: значение в `HKCU\...\Run` (§11).
- c. Ошибка регистрации хоткея показывается в строке Shortcuts; проверка дубликатов между операциями; хоткей у Screen Translate (§6.4).
- d. Размер шрифта popup и язык интерфейса хранятся не в `settings.json`, а в отдельном `ui.json` (§13).
- e. Локализация en/pl/uk/de, генерируемые каталоги, смена языка с перезапуском (§9).
- f. Overlay: нативный Win32 в `lingopop-platform` (§8).
- g. Секреты: ключ никогда не попадает в webview (§6.3).
- h. Usage: портируются таблица цен и форматирование (§10).
- i. Состояния иконки трея и строки истории (§7).
- j. Окна Settings и History - дополнительные webview-окна Tauri с тем же Svelte-приложением (§3.1).

## 3. Цели, не-цели, анти-цели

### 3.1 Цели

- Окно **Settings**: вкладки Main / Providers / Shortcuts / Usage / Magic / About, полный набор контролов macOS (§6).
- Окно **History**: поиск, список, Copy original / Copy translation / Delete, Clear (§6.8).
- Полное меню трея по составу status menu macOS (§7) и три состояния иконки.
- **Overlay Screen Translate** + хоткей Screen Translate (§8); убирается заглушка `NoPicker`.
- **Локализация** en / pl / uk / de + выбор языка приложения (§9).
- **Usage**: сводка, график, разбивка по провайдерам, оценка стоимости (§10).
- **Автозапуск** (§11), **проверка обновлений** (§12).
- Изменения настроек применяются на лету: `ConfigSnapshot` читается заново на каждый запуск операции; хоткеи перерегистрируются без перезапуска.

Реализация окон (решение ассистента, пункт j): `settings` и `history` - обычные (фокусируемые, с рамкой Windows) окна Tauri с тем же Svelte-приложением, что и popup. Один frontend-бандл, выбор экрана по метке окна (`popup` / `settings` / `history`); код Settings и History подгружается лениво, чтобы бандл popup не рос. Окна создаются при первом открытии и закрываются по x (в отличие от popup, который создаётся скрытым на старте); повторное открытие фокусирует существующее окно. Размеры как на macOS: Settings 720x560, не растягивается (`UI/SettingsView.swift:42`, `UI/SettingsWindowController.swift:36`); History 540x480, растягивается, минимум 480x360 (`UI/HistoryWindowController.swift:23-24`, `UI/HistoryView.swift:65`). Заголовки окон: «LingoPop Settings», «Translation History».

### 3.2 Не-цели

- Синхронизация любого вида: секция «iCloud Drive sync» и ключи `iCloudSync` / `syncHistoryToiCloud` не переносятся (`UI/Settings/MainSettingsTab.swift:110-127`). Решение ассистента, пункт a; согласуется с не-целями общего дизайна.
- Секции «Accessibility» / «Screen Recording», `PermissionsManager`, `PermissionAlert`, пункт меню «⚠ Re-check Accessibility…» (`MainSettingsTab.swift:129-175`, `StatusMenuController.swift:133-137`). На Windows этих прав нет. Решение ассистента, пункт a; общий дизайн (раздел 6) уже требовал скрыть их.
- Скрытое app-меню macOS (`installMainMenu`, Cmd-шорткаты). Решение ассистента, пункт a.
- Установка обновлений (авто-апдейт). Только уведомление и ссылка на страницу релизов.
- Установщик, подпись, Store (Фаза 5).
- Мультиязычное переключение «на лету» без перезапуска (см. отступление в §7).
- Языки, кроме en / pl / uk / de.

### 3.3 Анти-цели (провал, даже если остальное работает)

- **API-ключ попадает в webview** (в IPC-ответ, событие, DOM, devtools, localStorage) или в лог. Webview получает только булево «ключ есть».
- **Форма `settings.json` расходится с macOS** `SettingsStore.Persisted`: новые поля, переименования, другое кодирование. Любая Windows-only настройка живёт в отдельном файле (§10).
- **UI блокирует главный поток**, который обслуживает межпоточный `SetWindowPos` popup. Все команды `async`, никакого блокирующего ввода-вывода или ожидания окон на главном потоке (правило из `app/src-tauri/src/commands.rs`).
- **Фокус popup**: открытие Settings / History или overlay не должно ломать правила popup Фазы 4a: popup не получает фокус, dismiss-хуки работают как прежде.
- Потеря пользовательских данных при сохранении: запись `settings.json` только через `settings::save` (атомарно), `secrets.dat` только через `SecretStore`; повреждённый файл не перезаписывается «пустым» (поведение 4a сохраняется).
- Логика, проверяемая только вручную на Windows, там, где её можно вынести в чистую функцию с тестом.

## 4. Ограничения

Из `AGENTS.md`, без изменений:

- `lingopop-core` остаётся OS-agnostic (`forbid(unsafe_code)`, без `windows`, `tauri*`, `objc*`, без `tokio` как прямой зависимости); `scripts/check-core-deps.sh` это проверяет. Всё новое, что можно, - чистая логика в core или orchestrator (разбор тега версии, цены, форматирование, валидация хоткеев/провайдеров, поиск по истории, агрегации usage).
- `unsafe` только в `crates/lingopop-platform/src/windows/` и `src/macos/` с `// SAFETY:`. Реестр (автозапуск) и overlay - там же.
- `lingopop-orchestrator` не зависит от `tauri*` / `windows*`.
- Все команды Tauri `async`; главный поток свободен.
- Правило потока хоткеев: `GlobalHotkeyService` `!Send`, живёт в `thread_local!` потока event loop; перерегистрация идёт через `AppHandle::run_on_main_thread`.
- Windows проверяется только CI (и ручным чек-листом); dev-машина - Mac. Поэтому Windows-код держится тонким, а решения - в чистых функциях.
- Тесты без внешней сети; loopback-серверы допустимы.
- Секреты и ключи не попадают в логи, фикстуры и issue.
- Персистентные raw values и кодирование дат заморожены (`ProviderType`, `AIOperation`, даты, UUID). Тексты UI используют `display_name()`, а не raw value.
- Версия, зашитая в сборку (для сравнения при проверке обновлений): `lingopop_core::VERSION` / версия в `tauri.conf.json` (совпадают по правилам релиза).

## 5. Модель данных и состояние оболочки

### 5.1 Что хранится

| Файл (`%APPDATA%\LingoPop\`) | Содержимое | Изменения в 4b |
|---|---|---|
| `settings.json` | 12 полей `SettingsStore.Persisted` | запись из UI через `settings::save`; **форма не меняется** |
| `secrets.dat` | ключи API + `providerSettings.v1` (DPAPI) | запись из UI через `SecretStore` (в 4a не писалось) |
| `history.json`, `usage.json` | envelope v2 | чтение из окон; `remove` / `clear` / `reset` |
| `ui.json` (**новый**, только Windows) | `popupFontSize`, `appLanguage`, кэш проверки обновлений | см. §10 |

### 5.2 Изменения оболочки (`app/src-tauri`)

- `ShellConfig` получает методы записи: обновить `Settings` (с сохранением на диск, ошибка сохранения возвращается в UI и логируется без пользовательского текста), записать API-ключ, записать `ProviderConnectionSettings` одного провайдера. После записи следующий `snapshot()` видит новые значения (так работает 4a: снимок строится заново на каждый запуск).
- `ShellStores` получает чтение истории (`entries()`), `remove(id)`, `clear()`, `trim(max)`; чтение usage (`records()`), `reset()`. Как в 4a: store `None` (не открылся) = окно показывает пустое состояние, записи пропускаются, файл не перезаписывается.
- Если `SecretStore` не открылся (в 4a это `None`), вкладка Providers показывает ошибку на месте поля ключа и не даёт сохранить ключ, остальные поля сохраняются отдельно только если `ProviderSettingsState` не `Corrupt` (см. §6.3); решение ассистента, чтобы не затереть нечитаемый файл.
- Реестр результатов регистрации хоткеев (§6.4): оболочка хранит `operation -> {Registered | Conflict | Invalid}` и отдаёт его в Settings.
- Состояние трея (idle / working / error): оркестратору нужен способ сообщить о начале и конце работы. Предлагается опциональный приёмник статуса в `Deps` (по образцу `log`), вызываемый из `start`/`finish`/`present_error`; точная форма на усмотрение исполнителя, требование - не ломать существующие тесты и не тянуть `tauri` в orchestrator.

## 6. Области

### 6.1 Settings: общая структура

Эталон: `UI/SettingsView.swift:14-44`. Вкладки и порядок: **Main, Providers, Shortcuts, Usage, Magic, About**; иконки вкладок - инлайн-SVG, аналоги SF Symbols (`slider.horizontal.3`, `cpu`, `command`, `chart.bar.xaxis`, `sparkles`, `info.circle`); «Shortcuts» остаётся названием вкладки. Закрытие окна возвращает выбранную вкладку на Main (`SettingsWindowController.swift:45-48`). Пункт трея «About LingoPop» открывает Settings сразу на вкладке About, в том числе когда окно уже открыто (`:20-21`).

Поведение сохранения как на macOS: контролы Main, Shortcuts, Fallback применяются сразу при изменении (там `@Published` + автосохранение); форма провайдера и промпт Magic сохраняются кнопкой Save. Отступление: нет Combine-подписок; UI шлёт команду `settings_update` с изменённым полем, оболочка применяет и сохраняет (§5.2).

Все видимые строки идут через `t()` (каталог §9). Названия языков и провайдеров не локализуются.

### 6.2 Settings → Main

Эталон: `UI/Settings/MainSettingsTab.swift:32-176`. Секции и контролы в порядке macOS; справа - куда пишется значение.

| Секция | Контрол (английский текст) | Поле | Эталон |
|---|---|---|---|
| Target language | Toggle «Auto-detect input language» | `settings.json: useAutoDetect` | `:35` |
| | Picker «Default target» (при auto-detect) / «Target»: `quick_list()` + текущий, если его нет в списке | `targetLanguage` | `:36-43` |
| | Редактор «Language pairs» (виден при auto-detect): строки «from → to» с кнопкой удаления, ряд из двух Picker + кнопка добавления; значения по умолчанию нового ряда Russian → Ukrainian; подпись «If detected source has no pair, the default target is used.» | `languagePairs` (словарь) | `:193-237` |
| Popup | Slider «Result font size» 11..20 шаг 1 с подписью «N pt», иконки меньше/больше | **`ui.json: popupFontSize`** (по умолчанию 13) | `:49-64`, `UI/PopupSupport.swift:6-7` |
| Language | Picker «Language»: «System» (локализовано), English, Polski, Українська, Deutsch (родные названия, не локализуются) | **`ui.json: appLanguage`** | `:66-84`, `Core/AppLanguage.swift:6-33` |
| | Строка «Restart LingoPop to apply the new language.» + кнопка «Relaunch», если выбор даёт другой язык, чем у запущенного UI | - | `:76-83` |
| Limits | «Max input characters» (число, 0 = без лимита) | `maxInputChars` | `:87-92` |
| | «History size» (степпер 0..100) | `maxHistoryCount` | `:93-97` |
| | «Display» (степпер 0..100: строк истории в меню) | `menuHistoryCount` | `:98-102` |
| Startup | Toggle «Launch at login» | `launchAtLogin` (+ реестр, §11) | `:106` |
| | Toggle «Auto-copy OCR/translation results to clipboard» | `autoCopyOCR` | `:107` |

Правила:

- Уменьшение «History size» немедленно обрезает историю (`HistoryStore::trim`), как в `AppDelegate.swift:84-88`.
- Меню трея показывает `min(menuHistoryCount, maxHistoryCount)` строк (`StatusMenuController.swift:66`).
- Размер шрифта popup меняет раскладку popup (на macOS `PopupLayout(fontSize:)`, `UI/PopupLayout.swift:47-56`); текущий popup на Windows шрифт не параметризует. 4b добавляет параметр в frontend popup (шрифт результата и производные метрики по образцу `PopupLayout`), значение читается при показе и обновляется событием при изменении в Settings. Измерение ширины (`panelWidth`) использует тот же размер, чтобы режимы action row считались верно.
- Отступление (удалено): секции iCloud и две секции прав (см. §3.2).
- Значение «Language» применяется только после перезапуска (§9).

### 6.3 Settings → Providers

Эталон: `UI/Settings/ProviderSettingsTab.swift`, `ProviderSidebar.swift`, `ProviderDraft.swift`, `ProviderDetailSections.swift`. Раскладка: слева сайдбар шириной 220 со списком всех восьми провайдеров (`ProviderType::ALL`), справа форма выбранного; при открытии выбран активный провайдер (`ProviderSettingsTab.swift:19`); смена выбранного провайдера отбрасывает несохранённые правки (`:14`).

Строка сайдбара: иконка, `display_name`, звезда у активного, статус: «No key needed» (провайдер без ключа), «Key set», «No key»; плюс «· ↳ <fallback>» если задан fallback (`ProviderSidebar.swift:40-70`). Иконки/цвета провайдеров - инлайн-SVG и цвета, соответствующие `sfSymbol` / `iconColor` macOS (на усмотрение исполнителя).

Форма (набор секций определяется `ProviderCapabilities::settings_fields`, как `impl.settingsFields`):

| Секция | Контрол | Поле / действие | Эталон |
|---|---|---|---|
| Заголовок | иконка, имя, строка поддерживаемых операций через « · » (`supported_operations`), метка «Active» или кнопка «Set as active» | `activeProvider` | `ProviderSettingsTab.swift:90-120` |
| API key (если `requires_api_key`) | индикатор «A key is stored» / «No key set», кнопка «Clear»; поле пароля «Paste your API key» / «Enter a new key to replace»; подпись про хранение; ссылка «How to get an API key» | **`secrets.dat: apiKey.<raw>`** | `ProviderDetailSections.swift:6-37` |
| API key (без ключа) | «No key needed» + ссылка «About this provider (no key needed)» | - | `:38-47` |
| Endpoint (если есть `BaseUrl`) | поле «Base URL» (для DeepL «Auto (free/pro chosen by key)»), кнопка справки с дефолтом, кнопка сброса к дефолту | `providerSettings.v1[provider].baseURL` | `:62-105` |
| Model (если есть пресеты) | поле «Model name» + меню пресетов `available_models` | `...model` | `:107-131` |
| HTTP Referer (optional) | только Google Translate | `...referer` | `:141-150` |
| Region (optional) | только Microsoft Translator | `...region` | `:152-161` |
| Fallback | Picker «If <provider> fails»: «Don't fall back» или любой другой провайдер; подпись «Triggered on 429 (rate limit), 5xx (server error), and network errors.» | `settings.json: fallbacks[provider]` (применяется сразу) | `:174-196` |
| Save | кнопка «Save» (неактивна при ошибках валидации), метка «Saved» на 1,5 с | запись драфта | `ProviderSettingsTab.swift:68-82, 133-143` |

Валидация драфта - перенос `ProviderDraft.validate` (`ProviderDraft.swift:36-56`): все поля тримятся; Base URL пустой или начинается с `https://` («Base URL must start with https://»); Referer без управляющих символов; Region только ASCII-буквы/цифры, `-`, `_`. Пустой Referer / Region = `None`. Сохранение ключа: непустой введённый ключ записывается, поле очищается, после сохранения никогда не заполняется значением (`:8`, `:63-78`).

Валидацию нужно реализовать **один раз, в Rust** (чистая функция в core или orchestrator, возвращающая ошибки по полям, с тестами), а frontend вызывает её командой для мгновенной подсказки и сервер повторяет её при сохранении. Решение ассистента: так валидация не расходится между фронтом и записью.

Отступления Windows:

- Секреты (решение ассистента, пункт g): webview **не получает ключ**. Команды: `provider_state(provider) -> {hasKey, baseUrl, model, referer, region, ...}`; `provider_save(provider, draft)` где `draft.apiKey` - единственное место, где ключ идёт из webview в Rust (один раз, при сохранении), оболочка передаёт его в `SecretStore::set_api_key`; `provider_clear_key(provider)`. Ключ не логируется, не эхо-возвращается, не сохраняется в состоянии компонента после отправки (поле очищается), `Debug` команд не печатает драфт.
- Подпись про хранение: macOS «Stored locally in an encrypted file (AES-GCM, machine-bound).» (`:34`) заменяется Windows-only строкой про DPAPI и учётную запись Windows.
- Ссылка «How to get an API key» ведёт на `docs/api-keys.md` этого репозитория, на якорь с именем провайдера (заголовки закреплены в `AGENTS.md`); ссылка открывается через системный браузер (не в webview), через команду оболочки с белым списком URL (только домены этого репозитория и сам разрешённый шаблон).
- Если запись `secrets.dat` не удалась, ошибка показывается под полем ключа (как `store.apiKeySaveError`, `:29-33`), а не молча.
- Ключи `providerSettings.v1` и ключи API пишутся одним `SecretStore`; при `ProviderSettingsState::Corrupt` сохранение ConnectionSettings блокируется с сообщением (иначе нечитаемый файл был бы перезаписан).

### 6.4 Settings → Shortcuts

Эталон: `UI/Settings/HotkeySettingsTab.swift`, `UI/HotkeyRecorderField.swift`.

- Строки на **все** операции `AIOperation::ALL` в порядке macOS: иконка + `display_name`, справа поле-рекордер (ширина 160, высота 24) и кнопка ✕ очистки, видимая только когда хоткей задан (`HotkeySettingsTab.swift:10-20, 45-51`). Подпись под списком: «Click a shortcut field, then press a key combination. ...» (Windows-вариант текста, см. §9).
- Поле: клик включает запись, заголовок «Press a key combo…»; повторный клик выключает (`HotkeyRecorderField.swift:52-55, 79`).
- Правила записи (зеркало macOS, `:59-72`): нужна минимум одна клавиша-модификатор (Ctrl / Alt / Shift; Win принимается, если webview его получил); нажатие без модификаторов не записывается, при этом Esc без модификаторов отменяет запись; нажатие одних модификаторов игнорируется (`flagsChanged` ничего не делает, `:74-76`); после записи режим записи выключается. Ключ определяется по `KeyboardEvent.code` (так хранит `Hotkey.key`), список модификаторов - в `Hotkey.modifiers`; сборка хоткея и `is_valid()` - в Rust (frontend не принимает решений о валидности, кроме правил записи выше). Показ: `Hotkey::display_string` (`Ctrl+Alt+T`).
- Значения по умолчанию: как сейчас у core - только Translate = `Ctrl+Alt+T` (по умолчанию у macOS тоже только translate, `Core/SettingsStore.swift:95-100`); Screen Translate и остальные без хоткея.

Решения ассистента для Windows:

- **Ошибка регистрации показывается в строке** (отступление, вызвано Windows): `RegisterHotKey` отказывает, если сочетание занято другим приложением или некорректно. Раньше (4a) это только логировалось. Теперь оболочка хранит результат (`Conflict` / `Invalid`) на операцию и отдаёт его в Settings: красная строка под полем, например, «This shortcut is already used by another app.» (Windows-only ключи каталога). Результат обновляется при каждой перерегистрации.
- **Дубликаты между операциями** (решение ассистента; на macOS проверки нет): при записи сочетания, уже назначенного другой операции, UI не сохраняет его и показывает в строке «Already used by <operation>.». Это отступление от принципа «копируй macOS»; обосновано тем, что регистрация второго идентичного сочетания гарантированно падает (`AlreadyRegistered`), так что пользователь получил бы ту же ошибку позже и менее понятную. Если владельцу это мешает, проверку можно убрать, регистрационная ошибка останется.
- **Запись при зарегистрированных хоткеях**: пока включён режим записи, все глобальные хоткеи приостанавливаются (иначе нажатие текущего сочетания сработало бы как операция, а не как запись, потому что `RegisterHotKey` перехватывает его раньше webview). По окончанию записи или потере фокуса окна хоткеи возвращаются. Решение ассистента; на macOS Carbon-хоткеи ведут себя так же, но проблема там не отмечена, поэтому поведение нужно проверить вручную.
- **Применение**: изменение хоткея пересоздаёт регистрации на потоке event loop через `run_on_main_thread` (правило потока). Меняется только набор регистраций, сам `GlobalHotkeyService` остаётся в `thread_local!`.
- **Screen Translate получает хоткей**: из `hotkeys::registered_operations` удаляется исключение Screen Translate, тест `screen_translate_is_not_registered` меняется на обратный. Обработчик остаётся прежним (`Orchestrator::trigger`).

### 6.5 Settings → Usage

Раздел §10.

### 6.6 Settings → Magic

Эталон: `UI/Settings/MagicSettingsTab.swift`.

- Описание «Magic runs a single AI call with your custom system prompt. Use {{targetLanguage}} to insert your configured target language automatically.»
- Моноширинный многострочный редактор (минимум высоты 120), счётчик «N / 2000» (в Unicode scalar values, красным от 1900), обрезка на лету до 2000 scalar (`:20-27`, `:30-32`).
- «Reset to default» с подтверждением «Reset prompt?» / «This will replace your prompt with the default. Continue?» (кнопки «Reset», «Cancel») → `DEFAULT_MAGIC_PROMPT` (`:34-36, 53-61`).
- «Save» (неактивна, пока черновик равен сохранённому): значение пропускается через `settings::sanitize_magic_prompt` и пишется в `magicPrompt`; после сохранения черновик заменяется очищенным значением (`:39-42`).
- Подсчёт и обрезка выполняются по тем же правилам, что в core (подсчёт scalar через команду или совпадающая реализация с тестом на паритет, чтобы эмодзи и суррогаты считались одинаково).

### 6.7 Settings → About

Эталон: `UI/Settings/AboutTab.swift`.

- Иконка 120x120 (`docs/icon.png`), «LingoPop», «Version <X>», индикатор проверки, кнопка «Check for updates» (`:19-35`).
- Плашка «Version <X> available» + «Download», если найдена версия новее (`:37-62`); логика §12.
- Описательный текст и ссылка «Privacy Policy» (`:64-76`).

Отступления: два описательных текста macOS упоминают «on your Mac» и iCloud-синхронизацию (`:65, :68`) - вместо них Windows-only строки каталога без синхронизации и без «Mac»; ссылка Privacy Policy ведёт на `PRIVACY.md` этого репозитория, а не macOS-репозитория (`:73`).

### 6.8 Окно History

Эталон: `UI/HistoryView.swift`, `UI/HistoryWindowController.swift`.

- Шапка: поле «Search history» (поиск по оригиналу и переводу, регистронезависимо, по обрезанному запросу), кнопка очистки (корзина) видима только при непустой истории (`HistoryView.swift:23-37, 13-19`).
- Состояния: пустая история - значок, «No translations yet.», «Press your hotkey on selected text to translate.»; нет совпадений - «No matches for “<query>”.»; иначе список (`:41-63`).
- Строка записи: значок и короткое имя операции, для Translate - бейдж «<source> → <target>» либо только target; бейдж провайдера (`display_name`), дата-время `hour minute day month year` в локали, меню «Copy original» / «Copy translation» / «Delete»; оригинал (серый, до 3 строк) и перевод (до 6 строк), текст выделяемый (`:69-112`).
- Очистка истории: на macOS без подтверждения (`:30-32`); для Windows тоже без подтверждения (паритет). `clear` ставит watermark, `Delete` ставит tombstone (логика уже в `HistoryStore`).

Отступления Windows:

- Копирование: через системный буфер из Rust (команда оболочки) или `navigator.clipboard`; текст ставится как обычный (в Win+V и облачный буфер разрешено, так как это явное действие пользователя). Пометка формата исключения (`CanIncludeInClipboardHistory = 0`) нужна только для временных записей Replace/capture; для явного Copy из History она не нужна (решение ассистента).
- Окно получает записи командой чтения (весь список, поиск и фильтрация на frontend; поиск вынесен в чистую функцию с тестом). Обновление при новой записи: событие `history://changed` (оболочка эмитит после `add_history` / `remove` / `clear` / `trim`).
- Дата форматируется по языку приложения (§9), не по системному.

## 7. Трей

Эталон: `UI/StatusMenuController.swift`, `Core/StatusItemController.swift`.

### 7.1 Меню (порядок как на macOS, `StatusMenuController.swift:62-144`)

1. Заголовок (неактивный) «Select the clip you want to add to your clipboard».
2. До `min(menuHistoryCount, maxHistoryCount)` строк недавней истории (если пусто - неактивная «No recent translations»). Текст строки: `оригинал → перевод`, обрезанные по краям, переводы строки заменены пробелами (`:194-198`); клик копирует **перевод** в буфер (`:188-192`).
3. Разделитель.
4. Пункты операций по `AIOperation::ALL`, `display_name`, справа текущий хоткей (`:109-127`). Клик запускает операцию (`statusMenuDidSelect`).
5. Разделитель; подменю «Target: <язык>» со списком `quick_list()` (+ текущий в начало, если его нет), галочка у текущего; выбор пишет `targetLanguage` и сохраняет (`:129, :146-174`).
6. «History…», «Settings…», «About LingoPop».
7. Разделитель, «Quit LingoPop».

Отступления Windows:

- Меню нативное (muda/Tauri), кастомных view-строк нет (`HistoryMenuItemView`, расчёт ширины `statusMenuRowWidth` не переносятся). Длинные строки истории усекаются до фиксированной длины с «…» (решение ассистента; исходный общий дизайн это уже требовал); полный текст доступен в окне History.
- Пункт «⚠ Re-check Accessibility…» убирается (§3.2).
- **Клавиши Cmd+1..0 у строк истории** (решение ассистента, пункт i): в контекстном меню трея Windows нет аналога Cmd-эквивалентов. Предлагается показывать мнемоники `&1`..`&9`, `&0` (нажатие цифры при открытом меню выбирает строку); точный механизм (мнемоники muda или ничего) оставлен исполнителю после проверки на Windows. Cmd+H / Cmd+, / Cmd+Q у остальных пунктов не воспроизводятся.
- Меню перестраивается при каждом открытии (`menuNeedsUpdate`, `:52-54`) либо при каждом изменении истории/настроек (проще для muda): обновление пунктов по событиям `history://changed` и `settings://changed`.
- **Запуск текстовой операции из меню** (решение владельца, вариант (a)): клик по меню трея переносит foreground в оболочку, поэтому платформа получает новый компонент - **трекер последнего внешнего окна**: помнит последнее foreground-окно, которое НЕ принадлежит LingoPop. Windows: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` (out-of-context) на потоке с message loop, события фильтруются по собственному process id; trait + fake + Windows-реализация в `lingopop-platform`; macOS dev: no-op либо текущее frontmost-приложение. Живёт ли трекер в потоке message loop dismiss-бэкенда или в собственном потоке - решает исполнитель. Правило устаревания: никакого, кроме `IsWindow` (решение ассистента).
  - Текстовая операция, запущенная из трея, берёт у трекера запомненное окно, восстанавливает его существующим путём `ForegroundWindow::restore` (с короткой паузой до фактического перехода окна в foreground) и затем выполняет обычный захват выделения, так что поведение совпадает с macOS.
  - Нет запомненного окна, оно уже закрыто (`IsWindow` false) или `restore` не удался: показывается ошибка в стиле nothing-selected / foreground-lost; используется существующий ключ `KEY_FOREGROUND_LOST` (при отсутствии запомненного окна допустим `KEY_NOTHING_SELECTED`; исполнитель берёт тот, что точнее по тексту).
  - Операции по хоткею не меняются: они используют текущее foreground-окно. Elevated-цель по-прежнему даёт `InputBlocked`, как сейчас.
  - `Orchestrator::trigger` получает параметр источника/цели запуска (хоткей = текущее foreground, трей = окно трекера); форму выбирает исполнитель.
  - Screen Translate из меню работает без выделения и трекера.
- Меню нужно локализовать (каталог §9), поэтому тексты меню идут из Rust-каталога (отдельный JSON/таблица, генерируемые тем же скриптом), а не из frontend.
- Двойной клик/клик по иконке: открывает Settings? На macOS такого нет; не добавляем (паритет). Повторный запуск exe (single-instance) открывает Settings (решение ассистента; комментарий в `lib.rs` уже предвидел это).

### 7.2 Иконка

macOS: idle - текстовый шаблон «AЇ»; working - два SF-символа, переключаются каждые 0,45 с; error - красный треугольник на 3 с, затем idle (`StatusItemController.swift:26-45, 66-78`). Windows (решение ассистента):

- idle - иконка приложения из `icons/` (`default_window_icon`);
- working - два кадра, смена каждые 450 мс;
- error - красный индикатор поверх иконки на 3 с, затем idle (если за это время не пришло другое состояние: отмена таймера как на `:28-29`).
- Кадры - PNG/ICO, сгенерированные скриптом из исходника (в `app/src-tauri/icons/`), коммитятся готовыми; таймеры и смена иконки выполняются на потоке, допустимом Tauri (через `run_on_main_thread` или API трея, не блокируя); детали и размеры (16/32 px, светлая/тёмная панель задач) на усмотрение исполнителя.

Состояния меняются приёмником из §5.2: working между `trigger`/`start` и появлением результата (для Screen Translate - с момента Loading), error при `present_error`, idle при результате или закрытии. Как на macOS (`ScreenTranslateFlow.swift:71, 102`): working включается после выбора области, а не во время выбора.

## 8. Overlay Screen Translate

Эталон: `UI/CaptureOverlayWindow.swift`, `App/ScreenTranslateFlow.swift`. Реализует существующий trait `RegionPicker` (`pick -> Result<Option<Rect>, PlatformError>`, `Ok(None)` = отмена, ничего не показано) и заменяет `NoPicker`. Остальной поток (захват, OCR, popup, vision-retry) уже сделан в 4a (`crates/lingopop-orchestrator/src/screen.rs`).

Поведение macOS, которое переносится:

- затемнение 30% чёрного поверх экрана (`:171`);
- выделение «вырезается» (прозрачное), белая рамка 1,5 pt (`:188-194`);
- подпись «<w> × <h>» (11 pt, medium, белая с тенью) в правом нижнем углу выделения, не выходит за край (`:196-211`);
- курсор-перекрестие на время выбора (`:90`);
- один жест: нажатие, перетаскивание, отпускание; без подтверждения (`:103-127`);
- отпускание с выделением < 10x10 - тихая отмена (`:121-125`; в `ScreenTranslateFlow.swift:107-108` отмена не показывает ничего);
- Esc отменяет (`:128-130`);
- повторный вызов хоткея во время выбора отменяет предыдущий выбор (отмена future = drop, `CaptureOverlayWindow.swift:52-60`; в Rust - drop future `pick`);
- на macOS окна перекрывают все мониторы (`:51`); на Windows тоже.

Решения ассистента (пункт f):

- **Нативный Win32 в `lingopop-platform`**, не webview: быстрое открытие, нет проблем с фокусом WebView2, нет мерцания. Модуль `src/windows/overlay.rs` (`unsafe` только здесь), реализация `RegionPicker` для платформы; fake `FakeOverlay`/скриптованный picker для тестов оркестратора уже есть (`FakePicker`).
- Окно(а): либо по одному layered topmost окну на монитор, либо одно на весь виртуальный экран - на усмотрение исполнителя. Требования: Per-Monitor V2 DPI (поток под `DpiGuard`, как у `WindowsCapture`), координаты в физических пикселях виртуального экрана (результат `Rect` совместим с `validate_region`/`capture_region`), перекрывает панель задач, не крадёт активацию там, где этого можно избежать (но должно получать мышь и Esc; решается захватом мыши и низкоуровневым хуком клавиатуры либо `WS_EX_NOACTIVATE` + хук Esc - выбор исполнителя; popup-правила «Esc поглощается только пока активен» сохраняются).
- Размер порога «10x10» считается в физических пикселях; толщина рамки 1,5 pt масштабируется на DPI монитора, где рисуется кромка; шрифт подписи тоже.
- Окружение не Windows (macOS dev): `pick` возвращает `Ok(None)` либо `Unsupported`, как `MacCapture`/`MacOcr` (решение ассистента); оболочка при `Unsupported` ведёт себя как отмена с записью в лог.
- Завершение: окна уничтожаются на любом пути (drop, Esc, отпускание, ошибка), мышь освобождается, хук снимается, курсор восстанавливается (RAII-обёртки, как везде в `windows/`).
- Отличие от macOS в потоке: на macOS перед захватом приложение-предшественник активируется с задержкой 150 мс (`ScreenTranslateFlow.swift:57-60`) - на Windows оболочка не активирует себя во время выбора, значит, и восстановление не нужно; убедиться, что захват не содержит самого overlay (окна скрыты до `capture_region`; ручная проверка).
- Хоткей Screen Translate включается (§6.4); пункт меню трея «Screen Translate» тоже.

## 9. Локализация

Эталон: `App/Localizable.xcstrings` (206 ключей; en - исходный язык; pl/uk/de покрывают 197; 9 ключей `shouldTranslate: false` - ключи вроде `%lld`, `—`, `LingoPop`), `scripts/check-localizations.py` (правила: все переводимые ключи переведены, мультимножество спецификаторов `%@ %lld %1$@ %%`, число `\n` и спецсимволы `… → ⌘ {{targetLanguage}}` совпадают с ключом), `Core/AppLanguage.swift`.

Решения ассистента (пункт e):

- **Языки**: en, pl, uk, de. Приложение: «System» / English / Polski / Українська / Deutsch; родные названия - данные, не переводятся.
- **System** определяется по языкам интерфейса ОС (список предпочтений Windows), первое совпадение с `[en, pl, uk, de]` по коду основного языка, иначе en (порт `AppLanguage.resolve`, `Core/AppLanguage.swift:106-109`, `supportedCodes` `:96`). Логика сопоставления - чистая функция в core с тестами.
- **Смена языка применяется после перезапуска**; рядом с Picker появляется строка «Restart LingoPop to apply the new language.» и кнопка «Relaunch» (`MainSettingsTab.swift:76-83`, `Core/AppLanguage.swift:116-120` `needsRelaunch`: сравнение «эффективный язык выбора» и «язык, с которым запущен UI»). Перезапуск: запуск нового экземпляра с ожиданием выхода старого (на macOS `App/AppRelauncher.swift:9-34`); на Windows нужна отложенная команда-помощник, не допускающая одновременной работы двух экземпляров (конфликт `RegisterHotKey` и single-instance); механизм на усмотрение исполнителя. Если запуск помощника не удался - сообщение «Could not relaunch LingoPop» (ключ есть в macOS-каталоге).
  - **Это отменяет** строку общего дизайна (раздел 8) «Переключение языка применяется сразу, без перезапуска». Владелец в этой фазе выбрал поведение macOS; общий дизайн при коммите этого документа нужно привести в соответствие.
- **Каталог**: скрипт `scripts/` (Python, по образцу `scripts/mac-fixtures`) читает `App/Localizable.xcstrings` macOS-репозитория и **генерирует и коммитит** JSON на язык (`en.json` как отображение «ключ → ключ» не нужен: английский ключ - собственный fallback, как в `app/src/lib/i18n.ts`) для frontend, а также таблицу строк для Rust (текст меню трея и ключи ошибок, уже используемые popup). Файлы никогда не правятся вручную; как фикстуры macOS: перегенерация только при изменении macOS-каталога; в `SOURCE.txt`-подобный файл пишется коммит macOS-репозитория.
  - **Это отменяет** строку общего дизайна (раздел 8): «повторных импортов нет, источник истины - каталог lingopop-app». Теперь macOS-каталог остаётся источником для общих ключей (повторный импорт возможен), а Windows-only ключи живут отдельно.
- **Windows-only каталог**: небольшой вручную поддерживаемый файл (все четыре языка) для строк, которых нет на macOS: Windows-вариант справки по хоткеям (вместо `⌃ ⌥ ⇧ ⌘`), подпись хранения ключей (DPAPI), два текста About без «Mac»/iCloud, ошибки регистрации хоткея и дубликата, тексты Windows-only popup-ключей (`KEY_INPUT_BLOCKED` и др., уже существующие и пока только английские), подписи Relaunch и окон. Те же проверки, что в `check-localizations.py`, запускаются по обоим каталогам (CI, шаг `npm test` или отдельный).
- **Ключи macOS, неприменимые к Windows**, помечаются в скрипте и не попадают в сгенерированные каталоги (список из 26 ключей: Accessibility, Screen Recording, iCloud, ⌘-хоткеи, «Select text anywhere on your Mac...», «Quit LingoPop from the menu bar...» и т. п.; точный список вычисляется скриптом по явному списку исключений, не по регулярному выражению).
- **Не локализуются**: raw values, названия языков (`languages::ALL`), названия провайдеров, тексты промптов, логи, HTTP/JSON, бренды.
- Применение: язык выбирается при старте (читается `ui.json`, §13); frontend получает свой каталог при загрузке окна (команда или встроенный JSON), Rust-часть (меню трея) - при построении меню. Даты в History форматируются по этому языку.
- Попап уже вызывает `t()` для всех строк; 4b заменяет pass-through реальным поиском в каталоге с тем же интерфейсом (`t(key, args)`), форматные подстановки `%@`/`%lld`/`%1$@` остаются.

## 10. Usage

Эталон: `UI/Settings/UsageSettingsTab.swift`, `UI/Settings/UsageFormatting.swift`, `UI/Settings/TokenBarChartView.swift`, `Core/UsagePricing.swift`, `Core/UsageStore.swift`. Данные - `usage.json` (envelope v2, уже читается и пишется `UsageStore` из core, хранит сырые `UsageRecord`).

- Период: сегментный переключатель «1 day» / «7 days» / «30 days» (по умолчанию 30), «Period», справа «Reset stats…» с подтверждением «Delete all usage history?» (кнопки «Delete», «Cancel») → `UsageStore::reset` (watermark) (`UsageSettingsTab.swift:19-37, 99-106`).
- Три карточки: «Est. Cost» (красным; «~$?», если цена ни для одной записи неизвестна), «Tokens» (input+output; подпись «in X · out Y»), «Calls» (подпись «avg X/call» или «no activity») (`:40-59`); подпись периода «today» / «last 7 days» / «last 30 days» (`:180-187`).
- График токенов по провайдерам (столбцы стопкой, порядок Claude, OpenAI, Gemini, Grok), заголовок «Hourly tokens · today» / «Daily tokens · 7 days» / «Daily tokens · 30 days»; слоты: 24 часа для «1 day», 7 или 30 календарных дней (`Core/UsageStore.swift:90-122`, `TokenBarChartView.swift`). Подписи оси: час каждые 6, день недели (локализованный) для 7 дней, число месяца каждые 5 для 30 дней (`TokenBarChartView.swift:12-22`).
- Таблица по провайдерам: Provider / Input / Output / Calls / Est. Cost; прочерки «—» для провайдера без данных (`UsageSettingsTab.swift:74-95, 140-178`).
- **Отслеживаемые провайдеры - четыре LLM** как на macOS: Claude, OpenAI, Gemini, Grok (`UsageFormatting.swift:7`); у переводческих сервисов usage нет. Имена в таблице - `usageDisplayName` (`Google Free`, `Google Cloud`, `Microsoft` - для остальных, в таблицу не попадают), цвета: красный, зелёный, синий, фиолетовый (`:21-43`).
- **Форматирование** (порт `UsageFormatting`, `:9-18`): токены `≥1000` → «X.YK» с одним знаком, иначе целое; стоимость `≥0.01` → «$X.XX», `≥0.0001` → «$X.XXXX», `>0` → «<$0.0001», иначе «$0.00».
- **Цены** (порт `UsagePricing`, `Core/UsagePricing.swift:4-58`): таблица «модель → (цена за 1M input, цена за 1M output)», стоимость = `(input*pi + output*po) / 1e6`; неизвестная модель - стоимость отсутствует, а сумма считается только по известным записям (`Core/UsageStore.swift:70-76`). Таблица копируется **как есть** (41 модель); изменение таблицы - отдельное решение. Рекомендуется держать таблицу и форматирование в core (чистая логика, тесты на паритет с macOS-значениями), а не во frontend.
- Агрегации (`totals`, `totalsByProvider`, `chartBuckets`) тоже чистые функции в core; границы дня считаются в локальной календарной зоне пользователя. Отступление: `Calendar.current` macOS заменяется локальной зоной Windows; зона передаётся в функцию параметром (для детерминированных тестов), реальное значение даёт оболочка.
- Окно получает записи за период командой и подписывается на событие `usage://changed`, чтобы открытая вкладка обновлялась после каждой операции.
- Отступление: график рисуется в frontend (SVG или canvas) с теми же цветами и порядком стопки; доступность: у графика есть текстовая альтернатива/таблица (aria).

## 11. Автозапуск

Эталон: `Core/LoginItemService.swift:9-23` (включает/выключает через `SMAppService`, ошибки только в лог), `App/AppDelegate.swift:76-80` (применяется при каждом изменении `launchAtLogin` и один раз при старте подпиской с исходным значением).

Решение ассистента (пункт b), отступление от общего дизайна (раздел 6 называл `tauri-plugin-autostart`; плагин не используется, чтобы не тащить зависимость и держать `unsafe` в одном месте):

- Значение `LingoPop` в `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, данные - путь к текущему exe **в кавычках** (`"C:\...\lingopop.exe"`). Включено = значение есть и совпадает; выключено = значение удалено.
- Применяется при изменении переключателя и **один раз при старте** (как macOS; привести реестр в соответствие с `launchAtLogin`, в том числе чинит путь после перемещения exe).
- Ошибка записи в реестр: только лог с кодом, без UI (паритет с macOS). Переключатель в UI не откатывается.
- Реализация: `LoginItem`-trait в `lingopop-platform` (`set_enabled(bool)`), Windows-реализация в `src/windows/` (реестр, `unsafe` с `// SAFETY:`), fake для тестов и no-op для macOS dev (решение ассистента).
- Портативный exe: путь меняется при перемещении файла; этот случай покрывается применением при старте.

## 12. Проверка обновлений

Эталон: `Core/UpdateChecker.swift:6-80`, `UI/Settings/AboutTab.swift:26-62`.

Решение владельца: источник - релизы `slucheninov/lingopop-app` (GET `.../releases/latest`, он не возвращает пре-релизы и черновики), пре-релизы игнорируются, 404 / нет релиза = нет плашки, ошибки молча; Download открывает `https://github.com/slucheninov/lingopop-app/releases/latest`.

Что переносится из macOS:

- Запрос с заголовком `Accept: application/vnd.github+json`, таймаут 10 с, только статус 2xx, из JSON берётся `tag_name`, ведущая `v` отбрасывается (`:50-63`).
- Периодичность: проверка при старте, если прошло меньше 3 часов - используется кэш (`latestVersion`, время последней проверки), далее таймер каждые 3 часа; кнопка «Check for updates» проверяет немедленно (`:13, :21-39`). Кэш хранится в `ui.json` (§13), а не в `settings.json`.
- Сравнение версий: три числовые компоненты `major.minor.patch`, суффикс после `-` отбрасывается (`:69-80`). Вынести в core чистой функцией с тестами. Плашка «Version <X> available» показывается, если `latest` новее текущей (`:41-44`).

Отступления и уточнения Windows:

- Репозиторий и URL другие (выше). Заголовок `User-Agent` обязателен для GitHub API: используется `LingoPop/<VERSION>` из `ReqwestTransport` (в `Transport` он уже выставляется).
- Запрос идёт через `Transport` из core (`ReqwestTransport` в продакшене, `FakeTransport` в тестах) - единый HTTP-стек, без второго клиента; редиректы и TLS по правилам core (rustls, без платформенного TLS).
- **Текущая версия может быть пре-релизом** (сейчас `0.2.0-alpha.N`), а macOS-сравнение отбрасывает суффикс. Решение ассистента: версия `X.Y.Z-pre` считается **старее** стабильной `X.Y.Z` (иначе тестер на `0.2.0-alpha.5` никогда не получит `0.2.0`). Это маленькое отступление; остальное сравнение как на macOS. Если владельцу нужен точный паритет, достаточно убрать это правило.
- До первого стабильного релиза `latest` вернёт 404: UI ничего не показывает, кнопка «Check for updates» тоже молчит (паритет «молча»; индикатор проверки останавливается).
- Явно: пока нет первого стабильного релиза, `/releases/latest` возвращает 404, и вкладка About ничего не показывает; это принято (решение владельца: пре-релизы игнорируются).
- Тестирование: только через `FakeTransport` - стабильная версия новее, равна, старее; правило пре-релиз против стабильной; 404; сетевая ошибка. Отладочной подмены URL или версии нет (решение ассистента).
- Проверка не отправляет никаких данных пользователя, кроме стандартных заголовков HTTP; в лог пишутся только фиксированные строки.
- Место кода: разбор ответа, сравнение версий, правила кэша - core (чистые функции); таймер и открытие страницы - оболочка (URL открывается системным браузером через команду с белым списком, без шелл-интерполяции).

## 13. Хранилище `ui.json` (решение ассистента, пункт d)

Файл `%APPDATA%\LingoPop\ui.json`, только Windows (в macOS-репозитории эти значения лежат в `UserDefaults`: `LingoPop.popupFontSize`, `LingoPop.appLanguage`, `LingoPop.lastUpdateCheck`, `LingoPop.latestKnownVersion`). Причина: `settings.json` должен оставаться идентичным по форме `SettingsStore.Persisted` (анти-цель). Поля:

- `popupFontSize` (число, 11..20, по умолчанию 13; значение вне диапазона - ограничивается);
- `appLanguage` (`"system" | "en" | "pl" | "uk" | "de"`, по умолчанию `system`);
- кэш обновлений: `latestVersion`, `lastUpdateCheck` (секунды Unix) - необязательные.

Правила: **мягкое декодирование** по полям (отсутствует или некорректно = значение по умолчанию, остальные поля сохраняются, как в `Settings::from_json_bytes`), запись атомарная через `fsio::atomic_write`, повреждённый файл переименовывается (`quarantine`) и заменяется значениями по умолчанию; BOM допускается. Путь добавляется в `DataPaths` (`ui_json()`). Модуль живёт в core (чистое чтение/запись и валидация, тесты), так что macOS-тесты его проверяют. Файл не содержит секретов и пользовательского текста.

## 14. Тестирование

Принцип общего дизайна сохраняется: всё, что проверяется без человека, проверяется в CI.

- **core (чистая логика)**: валидация драфта провайдера, ограничение/подсчёт промпта (паритет с `sanitize_magic_prompt`), цены и форматирование usage (значения сверены с macOS на фикстурах), агрегации и слоты графика с заданной зоной, разбор тега версии и сравнение (включая правило пре-релиза), кэш обновлений, `ui.json`, определение языка System, проверка дубликатов хоткеев, усечение строк меню.
- **orchestrator / оболочка (fake-платформа)**: статус трея; перерегистрация хоткеев через `FakeHotkeyService` (регистрация, конфликт, обновление реестра результатов, приостановка на время записи); запись настроек и ключей через fake `SecretProtector` (ключ не появляется в ответах команд и логах); `LoginItem` fake (применение при старте и по изменению); picker через `FakePicker` (уже есть). Трекер последнего внешнего окна: fake-трекер + `FakeForeground` в тестах оркестратора (запуск из трея восстанавливает запомненное окно до захвата; нет окна / `IsWindow` false / сбой `restore` дают ошибку; запуск по хоткею трекер не использует).
- **Каталоги локализации**: проверка скриптом (правила `check-localizations.py`) по обоим каталогам; тест, что сгенерированные файлы совпадают с выводом скрипта; тест полноты ключей, используемых в frontend (`t('...')` литералы имеют перевод).
- **Frontend (Vitest, jsdom, моки Tauri)**: формы Settings (каждый контрол → команда с нужным полем), рекордер хоткеев (правила записи: без модификатора, Esc, только модификаторы, очистка ✕, приостановка), валидация Providers, переключение вкладок и открытие на About, поиск в History, график Usage (слоты, суммы), переключение языка (строка Relaunch), отсутствие ключа в состоянии после Save.
- **Платформа Windows (`#[ignore = "needs interactive desktop"]` в `tests/windows_smoke.rs`, шаг CI «Windows desktop tests»)**: overlay (создание и уничтожение окон, отмена Esc, порог 10x10 через синтетический ввод там, где это возможно); реестр автозапуска (запись/чтение/удаление в тестовом ключе, путь в кавычках; тест не должен оставлять значение `LingoPop` в реальном `Run`). трекер последнего внешнего окна (смена foreground между внешними окнами запоминается, окна своего процесса игнорируются). Тесты, которым рабочий стол не нужен (реестр HKCU), запускаются по умолчанию.
- **Ручной чек-лист** (`docs/testing/manual-checklist.md`, обновляется в этой фазе): открытие Settings/History из трея и повторным запуском exe; сохранение ключа и «Key set»; хоткей: запись, конфликт с занятым сочетанием, дубликат; Screen Translate: overlay на нескольких мониторах и смешанном DPI, Esc, < 10x10, подпись размера; смена языка и Relaunch; автозапуск (перезагрузка/повторный вход); строки истории в трее и мнемоники; запуск текстовой операции из меню трея (трекер возвращает прежнее окно, захват после закрытия меню; нет запомненного окна - сообщение); состояния иконки; обновление (ручной вызов с подменой версии).
- Не проверяется автоматически: внешний вид (DPI, тема Windows), поведение WebView2 за пределами popup, `RegisterHotKey` с чужими приложениями.

## 15. Что остаётся исполнителю

- Раскладка и выбор между одним окном на виртуальный экран и окном на монитор для overlay; способ получения Esc/мыши без кражи активации.
- Форма приёмника статуса трея в orchestrator; расписание кадров иконок и их формат.
- Размещение трекера последнего внешнего окна (поток dismiss-бэкенда или свой) и форма параметра источника у `Orchestrator::trigger`.
- Мнемоники для строк истории (или отказ), усечение строк (длина).
- Механизм Relaunch на Windows.
- Структура каталога Windows-only строк и точное имя скрипта генерации; список исключаемых macOS-ключей фиксируется явным списком.
- Мини-детали UI: иконки провайдеров и вкладок, отступы, анимации, дизайн карточек Usage; следование теме Windows (светлая/тёмная) в рамках macOS-раскладки.
- Разбиение на задачи и порядок (план пишется отдельно после утверждения этого документа). Разумная опора: сначала хранилища и чистая логика (core), затем команды оболочки и окна, затем overlay и трей, в конце локализация и чек-лист.
- Разрешения Tauri: добавить окна `settings` и `history` в `capabilities` (минимальный набор: события и собственные команды), не расширять CSP (`connect-src` остаётся `ipc:`; запросы идут из Rust).

## 16. Расхождения с общим дизайном (приведены в соответствие в общем дизайне в том же коммите)

1. §8 общего дизайна: «язык применяется сразу» → теперь с перезапуском (§9).
2. §8 общего дизайна: «после импорта источник истины - каталог lingopop-app, повторных импортов нет» → теперь общие ключи генерируются из macOS-каталога скриптом и не правятся вручную, Windows-only ключи отдельно (§9).
3. §6 общего дизайна: автозапуск через `tauri-plugin-autostart` → значение реестра напрямую (§11).
4. §6/§5 общего дизайна: меню трея с «About» уже описано; «Cmd+1..0» отсутствуют на Windows (§7).
5. Хоткей Screen Translate в Фазе 4a не регистрировался; в 4b регистрируется (§6.4).
