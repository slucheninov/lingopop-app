import Foundation

// Fixture generator / checker built against the REAL macOS model files (see run.sh).
// `generate <outdir>` writes deterministic fixtures and goldens; `check <dir>` decodes files with the Swift models.

typealias HistoryEnvelope = MergeableEnvelope<TranslationHistoryEntry>
typealias UsageEnvelope = MergeableEnvelope<UsageStore.Record>

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func date(_ seconds: Double) -> Date { Date(timeIntervalSinceReferenceDate: seconds) }

func uuid(_ string: String) -> UUID { UUID(uuidString: string)! }

func write(_ data: Data, named name: String, in dir: String) {
    let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
    do { try data.write(to: url) } catch { fail("cannot write \(url.path): \(error)") }
}

func prettyJSON(_ object: Any) -> Data {
    do {
        var data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        data.append(0x0A)
        return data
    } catch { fail("cannot serialize golden: \(error)") }
}

// MARK: - generate

func historyEntries() -> [TranslationHistoryEntry] {
    [
        TranslationHistoryEntry(
            id: uuid("16990D16-7E15-47C3-A299-EF2AEC4400D1"),
            original: "Hello, world",
            translated: "Hallo, Welt",
            sourceLanguage: "English",
            targetLanguage: "German",
            provider: .claude,
            operation: .translate,
            timestamp: date(815059756.035193)
        ),
        TranslationHistoryEntry(
            id: uuid("9B1F5C3A-2D44-4E8A-8F0B-6A7C1E2D3F40"),
            original: "Привіт 👋\n\"світ\" <b>&</b> a/b",
            translated: "Hello 👋\n\"world\" <b>&</b> a/b",
            sourceLanguage: nil,
            targetLanguage: "English",
            provider: .googleFree,
            operation: .screenTranslate,
            timestamp: date(815059700.5)
        ),
        TranslationHistoryEntry(
            id: uuid("0A2B4C6D-8E0F-4A1B-9C3D-5E7F90A1B2C3"),
            original: "make it sound friendlier",
            translated: "Could you please make it sound a little friendlier?",
            sourceLanguage: "English",
            targetLanguage: "English",
            provider: .microsoftTranslator,
            operation: .magic,
            timestamp: date(815059000)
        ),
        TranslationHistoryEntry(
            id: uuid("D4E5F6A7-B8C9-4D0E-A1B2-C3D4E5F60718"),
            original: "He go to school yesterday.",
            translated: "He went to school yesterday.",
            sourceLanguage: "English",
            targetLanguage: "English",
            provider: .deepl,
            operation: .grammarFix,
            timestamp: date(815058000.25)
        ),
    ]
}

func usageRecords() -> [UsageStore.Record] {
    [
        UsageStore.Record(
            id: uuid("432F3827-40A1-430B-A818-67B2B6A36970"),
            timestamp: date(815059756.035515),
            provider: .claude,
            model: "claude-haiku-5-5",
            operation: .translate,
            inputTokens: 120,
            outputTokens: 45
        ),
        UsageStore.Record(
            id: uuid("7C8D9E0F-1A2B-4C3D-8E4F-5A6B7C8D9E0F"),
            timestamp: date(815059800.75),
            provider: .googleCloud,
            model: "nmt",
            operation: .screenTranslate,
            inputTokens: 0,
            outputTokens: 0
        ),
        UsageStore.Record(
            id: uuid("E1F2A3B4-C5D6-4E7F-9A8B-0C1D2E3F4A5B"),
            timestamp: date(815060000),
            provider: .openai,
            model: "gpt-6-luna",
            operation: .magic,
            inputTokens: 1500,
            outputTokens: 800
        ),
    ]
}

func generatePayloads(into dir: String) {
    let entries = historyEntries()

    var history = HistoryEnvelope(items: entries)
    history.deleted = [
        CloudTombstone(id: uuid("AAAAAAAA-1111-4222-8333-444444444444"), at: date(815059111.125)),
        CloudTombstone(id: uuid("BBBBBBBB-5555-4666-8777-888888888888"), at: date(815058222.5)),
    ]
    history.clearedBefore = date(800000000.5)
    guard let historyData = history.encoded() else { fail("history encode failed") }
    write(historyData, named: "history.json", in: dir)

    guard let legacy = try? JSONEncoder().encode(entries) else { fail("legacy history encode failed") }
    write(legacy, named: "history.legacy.json", in: dir)

    var usage = UsageEnvelope(items: usageRecords())
    usage.deleted = [CloudTombstone(id: uuid("CCCCCCCC-9999-4AAA-8BBB-CCCCCCCCCCCC"), at: date(815059333.875))]
    usage.clearedBefore = date(810000000.25)
    guard let usageData = usage.encoded() else { fail("usage encode failed") }
    write(usageData, named: "usage.json", in: dir)

    var secrets: [String: String] = [
        "apiKey.Claude": "sk-ant-FIXTURE",
        "apiKey.Google Translate": "AIza-FIXTURE",
        "futureEntry.v9": "{\"x\":1}",
    ]
    do {
        try SecretsPayload.setProviderSettings([
            .claude: ProviderConnectionSettings(baseURL: "https://api.anthropic.com", model: "claude-haiku-5-5", referer: nil, region: nil),
            .googleCloud: ProviderConnectionSettings(baseURL: "https://translation.googleapis.com", model: "nmt", referer: "https://example.com/app", region: nil),
            .microsoftTranslator: ProviderConnectionSettings(baseURL: "https://api.cognitive.microsofttranslator.com", model: "", referer: nil, region: "westeurope"),
        ], in: &secrets)
        write(try SecretsPayload.encode(secrets), named: "secrets.payload.json", in: dir)
    } catch { fail("secrets encode failed: \(error)") }
}

func generateLanguagesGolden(into dir: String) {
    func orNull(_ value: String?) -> Any { value.map { $0 as Any } ?? NSNull() }
    let rows: [[String: Any]] = Languages.all.map {
        [
            "name": $0.name,
            "code": $0.code,
            "deepLCode": orNull($0.deepLCode),
            "microsoftCode": orNull($0.microsoftCode),
            "inQuickList": $0.inQuickList,
        ]
    }
    let names = Languages.all.map(\.name) + ["Klingon"]
    let helpers: [[String: Any]] = names.map {
        [
            "name": $0,
            "code": LanguageCodes.code(for: $0),
            "deepLCode": LanguageCodes.deepLCode(for: $0),
            "microsoftCode": LanguageCodes.microsoftCode(for: $0),
        ]
    }
    write(prettyJSON(["languages": rows, "helpers": helpers]), named: "languages.golden.json", in: dir)
}

func generatePromptsGolden(into dir: String) {
    let defaultMagic = PromptBuilder.defaultMagicPrompt
    var system: [[String: Any]] = AIOperation.allCases.map {
        [
            "operation": $0.rawValue,
            "targetLanguage": "German",
            "magicPrompt": defaultMagic,
            "output": PromptBuilder.systemPrompt(for: $0, targetLanguage: "German"),
        ]
    }
    system.append([
        "operation": AIOperation.magic.rawValue,
        "targetLanguage": "Polish",
        "magicPrompt": "Answer in {{targetLanguage}}",
        "output": PromptBuilder.systemPrompt(for: .magic, targetLanguage: "Polish", magicPrompt: "Answer in {{targetLanguage}}"),
    ])
    system.append([
        "operation": AIOperation.magic.rawValue,
        "targetLanguage": "German",
        "magicPrompt": "",
        "output": PromptBuilder.systemPrompt(for: .magic, targetLanguage: "German", magicPrompt: ""),
    ])

    let userInputs = ["a<b>&c", "line1\nline2 &amp; <x>"]
    let sanitizeInputs = ["  <TEXT>hi</text>\n", "<text><text>x</text></text>", "x &lt; y &amp;&amp; y &gt; z", "<text>unclosed", "plain"]
    let golden: [String: Any] = [
        "defaultMagicPrompt": defaultMagic,
        "systemPrompt": system,
        "userContent": userInputs.map { ["input": $0, "output": PromptBuilder.userContent($0)] },
        "sanitizeOutput": sanitizeInputs.map { ["input": $0, "output": PromptBuilder.sanitizeOutput($0)] },
        "visionPrompt": [["targetLanguage": "German", "output": PromptBuilder.visionPrompt(targetLanguage: "German")]],
    ]
    write(prettyJSON(golden), named: "prompts.golden.json", in: dir)
}

// MARK: - check

func check(dir: String) {
    var seen = 0

    func load(_ name: String) -> Data? {
        let url = URL(fileURLWithPath: dir).appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: url.path) else {
            print("skipped \(name)")
            return nil
        }
        do { return try Data(contentsOf: url) } catch { fail("\(name): cannot read: \(error)") }
    }

    if let data = load("history.json") {
        guard let decoded = HistoryEnvelope.decode(data) else { fail("history.json: not decodable as MergeableEnvelope<TranslationHistoryEntry>") }
        print("history.json: \(decoded.envelope.items.count) items, \(decoded.envelope.deleted.count) tombstones, legacy=\(decoded.isLegacy)")
        seen += 1
    }
    if let data = load("usage.json") {
        guard let decoded = UsageEnvelope.decode(data) else { fail("usage.json: not decodable as MergeableEnvelope<UsageStore.Record>") }
        print("usage.json: \(decoded.envelope.items.count) items, \(decoded.envelope.deleted.count) tombstones, legacy=\(decoded.isLegacy)")
        seen += 1
    }
    if let data = load("secrets.payload.json") {
        guard let entries = SecretsPayload.decode(data) else { fail("secrets.payload.json: not a flat string map") }
        guard case .loaded(let providers) = SecretsPayload.providerSettings(in: entries) else {
            fail("secrets.payload.json: providerSettings.v1 is not .loaded")
        }
        print("secrets.payload.json: \(entries.count) entries, \(providers.count) provider settings")
        seen += 1
    }
    if seen == 0 { fail("none of history.json, usage.json, secrets.payload.json found in \(dir)") }
}

// MARK: - main

let args = CommandLine.arguments
guard args.count >= 3 else { fail("usage: gen generate <outdir> | gen check <dir>") }
switch args[1] {
case "generate":
    generatePayloads(into: args[2])
    generateLanguagesGolden(into: args[2])
    generatePromptsGolden(into: args[2])
case "check":
    check(dir: args[2])
default:
    fail("unknown mode \(args[1])")
}
