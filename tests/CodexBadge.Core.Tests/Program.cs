using System.Text;
using System.Text.Json;
using CodexBadge.Core;

var tests = new (string Name, Action Run)[]
{
    ("Quota mapper prefers the exact five-hour window", QuotaMapperPrefersFiveHour),
    ("Quota mapper falls back to the exact weekly window", QuotaMapperFallsBackToWeekly),
    ("Quota mapper does not relabel unknown windows", QuotaMapperRejectsUnknownWindow),
    ("Quota mapper reads bounded reset credits", QuotaMapperReadsResetCredits),
    ("Weekly quota blocks the capsule only when exhausted", WeeklyQuotaBlocksOnlyAtZero),
    ("Weekly reset text includes local date and time", WeeklyResetTextIncludesDateAndTime),
    ("Settings persist visual preferences", SettingsPersistVisualPreferences),
    ("JSONL framing preserves partial lines", JsonLineBufferFramesPartialLines),
    ("Retry backoff follows the planned sequence", RetryBackoffFollowsSequence),
    ("Quota cache keeps failures for thirty minutes", QuotaCacheExpiresAfterThirtyMinutes),
    ("Protocol recognizes rate-limit notifications", ProtocolRecognizesRateLimitNotifications),
    ("CLI locator honors explicit environment override", CliLocatorHonorsOverride),
    ("Capsule placement anchors to the left sidebar", CapsulePlacementAnchorsToSidebar),
    ("Capsule placement does not scale the physical sidebar anchor", CapsulePlacementDoesNotScaleSidebarAnchor),
    ("Codex window classifier rejects popup windows", CodexWindowClassifierRejectsPopups),
    ("Absolute drag position follows the cursor without accumulated drift", AbsoluteDragPositionFollowsCursor),
    ("New settings default to the light theme", NewSettingsDefaultToLightTheme),
    ("Position reset preserves startup preference and theme", PositionResetPreservesPreferences),
    ("Theme wheel includes a privacy mode", ThemeWheelCycles),
    ("Privacy mode masks quota until hover", PrivacyModeMasksQuotaUntilHover),
    ("Live quota anchor overrides the fixed sidebar", LiveQuotaAnchorOverridesSidebar),
    ("Quota anchor rejects this application's controls", QuotaAnchorRejectsOwnProcess),
    ("Sidebar anchor rejects the conversation composer", SidebarAnchorRejectsConversationComposer),
    ("Sidebar container bottom controls vertical placement", SidebarContainerBottomControlsVerticalPlacement),
    ("Progress color bands honor twenty and forty percent boundaries", ProgressColorBandsHonorBoundaries),
    ("Previous layout migrates to the new default position", LegacySettingsMigratePosition),
    ("Legacy product settings migrate without overwriting the new file", LegacyProductSettingsMigrateOnce),
};

var failed = 0;
foreach (var test in tests)
{
    try
    {
        test.Run();
        Console.WriteLine($"PASS {test.Name}");
    }
    catch (Exception error)
    {
        failed++;
        Console.Error.WriteLine($"FAIL {test.Name}: {error.Message}");
    }
}

Console.WriteLine($"{tests.Length - failed}/{tests.Length} tests passed");
return failed == 0 ? 0 : 1;

static void QuotaMapperPrefersFiveHour()
{
    using var json = JsonDocument.Parse("""
    {
      "rateLimits": {
        "planType": "plus",
        "primary": { "usedPercent": 57, "windowDurationMins": 10080, "resetsAt": 1789776000 },
        "secondary": { "usedPercent": 21, "windowDurationMins": 300, "resetsAt": 1789609740 }
      },
      "rateLimitResetCredits": { "availableCount": 0 }
    }
    """);

    var snapshot = QuotaMapper.Map(json.RootElement, DateTimeOffset.UnixEpoch);

    Equal(79d, snapshot.DisplayRemainingPercent);
    Equal(79d, snapshot.FiveHour?.RemainingPercent);
    Equal(43d, snapshot.Weekly?.RemainingPercent);
}

static void QuotaMapperFallsBackToWeekly()
{
    using var json = JsonDocument.Parse("""
    {
      "rateLimitsByLimitId": {
        "codex": {
          "planType": "plus",
          "primary": { "usedPercent": 57, "windowDurationMins": 10080, "resetsAt": 1789776000 }
        }
      },
      "rateLimits": {}
    }
    """);

    var snapshot = QuotaMapper.Map(json.RootElement, DateTimeOffset.UnixEpoch);

    Equal(43d, snapshot.DisplayRemainingPercent);
    True(snapshot.FiveHour is null, "Five-hour window must stay missing.");
}

static void QuotaMapperRejectsUnknownWindow()
{
    using var json = JsonDocument.Parse("""
    {
      "rateLimits": {
        "primary": { "usedPercent": 10, "windowDurationMins": 60, "resetsAt": 1789776000 }
      }
    }
    """);

    var snapshot = QuotaMapper.Map(json.RootElement, DateTimeOffset.UnixEpoch);

    True(snapshot.DisplayRemainingPercent is null, "An unknown window must not become the display value.");
    True(snapshot.Weekly is null, "An unknown window must not become weekly.");
}

static void QuotaMapperReadsResetCredits()
{
    using var valid = JsonDocument.Parse("""{ "rateLimits": {}, "rateLimitResetCredits": { "availableCount": 2 } }""");
    using var invalid = JsonDocument.Parse("""{ "rateLimits": {}, "rateLimitResetCredits": { "availableCount": 101 } }""");

    Equal(2, QuotaMapper.Map(valid.RootElement, DateTimeOffset.UnixEpoch).ResetCreditCount);
    True(QuotaMapper.Map(invalid.RootElement, DateTimeOffset.UnixEpoch).ResetCreditCount is null,
        "Out-of-range credit counts must be ignored.");
}

static void WeeklyQuotaBlocksOnlyAtZero()
{
    var fetchedAt = DateTimeOffset.UnixEpoch;
    var fiveHour = new QuotaWindow(83, 300, fetchedAt.AddHours(2));
    var exhausted = new QuotaSnapshot(
        fiveHour, new QuotaWindow(0, 10_080, fetchedAt.AddDays(3)), 0, fetchedAt);
    var remaining = new QuotaSnapshot(
        fiveHour, new QuotaWindow(0.1, 10_080, fetchedAt.AddDays(3)), 0, fetchedAt);
    var unknown = new QuotaSnapshot(fiveHour, null, 0, fetchedAt);

    Equal(0d, exhausted.DisplayRemainingPercent);
    True(exhausted.IsWeeklyExhausted, "Zero weekly quota must block five-hour availability.");
    Equal(83d, remaining.DisplayRemainingPercent);
    True(!remaining.IsWeeklyExhausted, "Any positive weekly remainder must stay usable.");
    Equal(83d, unknown.DisplayRemainingPercent);
    True(!unknown.IsWeeklyExhausted, "Missing weekly data must not be treated as exhausted.");
}

static void WeeklyResetTextIncludesDateAndTime()
{
    var chinaTime = TimeZoneInfo.CreateCustomTimeZone(
        "UTC+8-test", TimeSpan.FromHours(8), "UTC+8-test", "UTC+8-test");
    var reset = new DateTimeOffset(2026, 9, 19, 8, 33, 0, TimeSpan.Zero);

    Equal("9/19日 16:33", QuotaDisplayText.FormatWeeklyReset(reset, chinaTime));
}

static void SettingsPersistVisualPreferences()
{
    var directory = Path.Combine(Path.GetTempPath(), "CodexBadgeTests", Guid.NewGuid().ToString("N"));
    var path = Path.Combine(directory, "settings.json");
    try
    {
        var store = new SettingsStore(path);
        store.Save(new CapsuleSettings
        {
            OffsetX = 9,
            OffsetY = -4,
            StartWithWindows = false,
            Theme = CapsuleTheme.FrostLight,
        });
        var loaded = store.Load();

        Equal(9d, loaded.OffsetX);
        Equal(-4d, loaded.OffsetY);
        True(!loaded.StartWithWindows, "The startup preference must round-trip.");
        Equal(CapsuleTheme.FrostLight, loaded.Theme);
    }
    finally
    {
        if (Directory.Exists(directory)) Directory.Delete(directory, recursive: true);
    }
}

static void JsonLineBufferFramesPartialLines()
{
    var buffer = new JsonLineBuffer();

    var first = buffer.Append(Encoding.UTF8.GetBytes("one\ntw"));
    var second = buffer.Append(Encoding.UTF8.GetBytes("o\nthree\n"));

    Equal(1, first.Count);
    Equal("one", first[0]);
    Equal(2, second.Count);
    Equal("two", second[0]);
    Equal("three", second[1]);
}

static void RetryBackoffFollowsSequence()
{
    var backoff = new RetryBackoff();

    Equal(TimeSpan.FromSeconds(15), backoff.RegisterFailure());
    Equal(TimeSpan.FromSeconds(30), backoff.RegisterFailure());
    Equal(TimeSpan.FromMinutes(1), backoff.RegisterFailure());
    Equal(TimeSpan.FromMinutes(5), backoff.RegisterFailure());
    Equal(TimeSpan.FromMinutes(5), backoff.RegisterFailure());
    backoff.RegisterSuccess();
    Equal(TimeSpan.FromSeconds(15), backoff.RegisterFailure());
}

static void QuotaCacheExpiresAfterThirtyMinutes()
{
    var fetchedAt = new DateTimeOffset(2026, 9, 16, 1, 0, 0, TimeSpan.Zero);
    var snapshot = new QuotaSnapshot(new QuotaWindow(79, 300, fetchedAt.AddHours(4)), null, 0, fetchedAt);
    var cache = new QuotaCache();
    cache.SetSuccess(snapshot);
    cache.SetFailure();

    Equal(QuotaFreshness.Stale, cache.Get(fetchedAt.AddMinutes(29)).Freshness);
    True(cache.Get(fetchedAt.AddMinutes(29)).Snapshot is not null, "Recent stale quota must remain visible.");
    Equal(QuotaFreshness.Stale, cache.Get(fetchedAt.AddMinutes(29)).Snapshot?.Freshness);
    Equal(QuotaFreshness.Unavailable, cache.Get(fetchedAt.AddMinutes(31)).Freshness);
    True(cache.Get(fetchedAt.AddMinutes(31)).Snapshot is null, "Expired quota must not remain visible.");
}

static void ProtocolRecognizesRateLimitNotifications()
{
    True(AppServerProtocol.IsRateLimitUpdate("""{"method":"account/rateLimits/updated","params":{}}"""),
        "The update notification must be recognized.");
    True(!AppServerProtocol.IsRateLimitUpdate("""{"id":1,"result":{}}"""),
        "A response is not an update notification.");
    True(!AppServerProtocol.IsRateLimitUpdate("not-json"),
        "Malformed protocol input must be ignored.");
}

static void CliLocatorHonorsOverride()
{
    var directory = Path.Combine(Path.GetTempPath(), "CodexBadgeTests", Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(directory);
    var executable = Path.Combine(directory, "codex.exe");
    File.WriteAllBytes(executable, []);
    try
    {
        Equal(executable, CodexCliLocator.Find(new Dictionary<string, string?>
        {
            [CodexCliLocator.OverrideEnvironmentVariable] = executable,
            ["PATH"] = string.Empty,
            ["LOCALAPPDATA"] = directory,
        }));
    }
    finally
    {
        Directory.Delete(directory, recursive: true);
    }
}

static void CapsulePlacementAnchorsToSidebar()
{
    var normal = CapsulePlacement.Calculate(
        frameLeft: 100, frameRight: 1300, frameBottom: 900, scale: 1,
        capsuleWidth: 63, capsuleHeight: 24, offsetX: 0, offsetY: 0);
    Equal(379, normal.X);
    Equal(866, normal.Y);

    var narrow = CapsulePlacement.Calculate(
        frameLeft: 100, frameRight: 350, frameBottom: 900, scale: 1,
        capsuleWidth: 63, capsuleHeight: 24, offsetX: 0, offsetY: 0);
    Equal(285, narrow.X);
}

static void CapsulePlacementDoesNotScaleSidebarAnchor()
{
    var point = CapsulePlacement.Calculate(
        frameLeft: 12, frameRight: 1200, frameBottom: 900, scale: 1.25,
        capsuleWidth: 63, capsuleHeight: 24, offsetX: 0, offsetY: 0);

    Equal(275, point.X);
    Equal(858, point.Y);
}

static void CodexWindowClassifierRejectsPopups()
{
    True(CodexWindowClassifier.IsMainWindow(
        visible: true, toolWindow: false, hasOwner: false, width: 350, height: 600),
        "A compact Codex main window must remain eligible.");
    True(!CodexWindowClassifier.IsMainWindow(
        visible: true, toolWindow: true, hasOwner: false, width: 800, height: 600),
        "A tool window must never replace the Codex main window.");
    True(!CodexWindowClassifier.IsMainWindow(
        visible: true, toolWindow: false, hasOwner: true, width: 800, height: 600),
        "An owned popup must never replace the Codex main window.");
    True(!CodexWindowClassifier.IsMainWindow(
        visible: true, toolWindow: false, hasOwner: false, width: 240, height: 180),
        "A small popup must never replace the Codex main window.");
}

static void AbsoluteDragPositionFollowsCursor()
{
    var first = DragMath.CalculatePosition(
        windowStart: new CapsulePoint(282, 873),
        cursorStart: new CapsulePoint(300, 885),
        cursorCurrent: new CapsulePoint(337, 901));
    var later = DragMath.CalculatePosition(
        windowStart: new CapsulePoint(282, 873),
        cursorStart: new CapsulePoint(300, 885),
        cursorCurrent: new CapsulePoint(410, 840));

    Equal(new CapsulePoint(319, 889), first);
    Equal(new CapsulePoint(392, 828), later);
}

static void NewSettingsDefaultToLightTheme()
{
    Equal(CapsuleTheme.FrostLight, new CapsuleSettings().Theme);
}

static void PositionResetPreservesPreferences()
{
    var settings = new CapsuleSettings
    {
        OffsetX = 80,
        OffsetY = -30,
        StartWithWindows = false,
        Theme = CapsuleTheme.GraphiteDark,
    };

    settings.ResetPosition();

    Equal(0d, settings.OffsetX);
    Equal(0d, settings.OffsetY);
    True(!settings.StartWithWindows, "Position reset must not change startup preference.");
    Equal(CapsuleTheme.GraphiteDark, settings.Theme);
}

static void ThemeWheelCycles()
{
    Equal(CapsuleTheme.FrostLight, ThemeMath.Cycle(CapsuleTheme.CodexBlue, 120));
    Equal(CapsuleTheme.GraphiteDark, ThemeMath.Cycle(CapsuleTheme.FrostLight, 120));
    var privacy = ThemeMath.Cycle(CapsuleTheme.GraphiteDark, 120);
    Equal("Privacy", privacy.ToString());
    Equal(CapsuleTheme.CodexBlue, ThemeMath.Cycle(privacy, 120));
    Equal("Privacy", ThemeMath.Cycle(CapsuleTheme.CodexBlue, -120).ToString());
}

static void PrivacyModeMasksQuotaUntilHover()
{
    var type = typeof(QuotaVisuals).Assembly.GetType("CodexBadge.Core.CapsuleDisplay");
    True(type is not null, "CapsuleDisplay must provide privacy-safe text formatting.");
    var format = type!.GetMethod("FormatPercent");
    True(format is not null, "CapsuleDisplay.FormatPercent must exist.");

    Equal("••%", format!.Invoke(null, [83d, true, false]) as string);
    Equal("83%", format.Invoke(null, [83d, true, true]) as string);
    Equal("83%", format.Invoke(null, [83d, false, false]) as string);
}

static void LiveQuotaAnchorOverridesSidebar()
{
    var method = typeof(CapsulePlacement).GetMethod("CalculateFromAnchor");
    True(method is not null, "CapsulePlacement.CalculateFromAnchor must exist.");
    var point = (CapsulePoint)method!.Invoke(null, [
        512, 900, 1d, 63d, 24d, 0d, 0d
    ])!;

    Equal(447, point.X);
    Equal(866, point.Y);
}

static void QuotaAnchorRejectsOwnProcess()
{
    True(!QuotaAnchorCandidate.IsExternalProcess(42, 42), "The capsule must not anchor to its own percentage text.");
    True(QuotaAnchorCandidate.IsExternalProcess(43, 42), "A Codex control from another process is valid.");
}

static void SidebarAnchorRejectsConversationComposer()
{
    True(SidebarAnchorCandidate.IsContainer(0, 1200, 900, 1d, 0, 504, 900), "The left sidebar container is valid.");
    True(SidebarAnchorCandidate.IsContainer(0, 1200, 900, 1d, 0, 344, 900), "The compact sidebar container is valid.");
    True(!SidebarAnchorCandidate.IsContainer(0, 900, 900, 1d, 397, 900, 900), "The conversation composer must be rejected.");
    True(!SidebarAnchorCandidate.IsContainer(0, 900, 900, 1d, 0, 900, 900), "The full Codex window must be rejected.");
    Equal(343, SidebarAnchorCandidate.ChooseOuterRight(332, 343));
    True(
        SidebarAnchorCandidate.IsContainer(-9, 2569, 1389, 1.25d, -1, 343, 1382),
        "The real maximized 125% DPI sidebar bounds must be accepted without scaling UIA width twice.");
}

static void SidebarContainerBottomControlsVerticalPlacement()
{
    var point = CapsulePlacement.CalculateFromContainer(
        344, 860, 1d, 63d, 24d, 0d, 0d);
    Equal(279, point.X);
    Equal(826, point.Y);
}

static void ProgressColorBandsHonorBoundaries()
{
    Equal(QuotaProgressBand.Orange, QuotaVisuals.ProgressBand(0));
    Equal(QuotaProgressBand.Orange, QuotaVisuals.ProgressBand(19.99));
    Equal(QuotaProgressBand.Yellow, QuotaVisuals.ProgressBand(20));
    Equal(QuotaProgressBand.Yellow, QuotaVisuals.ProgressBand(39.99));
    Equal(QuotaProgressBand.Green, QuotaVisuals.ProgressBand(40));
    Equal(QuotaProgressBand.Green, QuotaVisuals.ProgressBand(100));
}

static void LegacySettingsMigratePosition()
{
    var directory = Path.Combine(Path.GetTempPath(), "CodexBadgeTests", Guid.NewGuid().ToString("N"));
    var path = Path.Combine(directory, "settings.json");
    Directory.CreateDirectory(directory);
    File.WriteAllText(path, """{"LayoutVersion":5,"OffsetX":-32.8,"OffsetY":19.2,"StartWithWindows":true,"Theme":"FrostLight"}""");
    try
    {
        var loaded = new SettingsStore(path).Load();
        Equal(0d, loaded.OffsetX);
        Equal(0d, loaded.OffsetY);
        Equal(CapsuleTheme.FrostLight, loaded.Theme);
        Equal(CapsuleSettings.CurrentLayoutVersion, loaded.LayoutVersion);
    }
    finally
    {
        Directory.Delete(directory, recursive: true);
    }
}

static void LegacyProductSettingsMigrateOnce()
{
    var directory = Path.Combine(Path.GetTempPath(), "CodexBadgeTests", Guid.NewGuid().ToString("N"));
    var legacyPath = Path.Combine(directory, "CodexCapsule", "settings.json");
    var currentPath = Path.Combine(directory, "CodexBadge", "settings.json");
    Directory.CreateDirectory(Path.GetDirectoryName(legacyPath)!);
    File.WriteAllText(legacyPath, """{"LayoutVersion":6,"OffsetX":12,"OffsetY":-5,"Theme":"GraphiteDark"}""");
    try
    {
        SettingsStore.MigrateLegacyFile(legacyPath, currentPath);
        var migrated = new SettingsStore(currentPath).Load();
        Equal(12d, migrated.OffsetX);
        Equal(-5d, migrated.OffsetY);
        Equal(CapsuleTheme.GraphiteDark, migrated.Theme);

        File.WriteAllText(currentPath, """{"LayoutVersion":6,"OffsetX":99}""");
        SettingsStore.MigrateLegacyFile(legacyPath, currentPath);
        Equal(99d, new SettingsStore(currentPath).Load().OffsetX);
    }
    finally
    {
        if (Directory.Exists(directory)) Directory.Delete(directory, recursive: true);
    }
}

static void Equal<T>(T expected, T actual)
{
    if (!EqualityComparer<T>.Default.Equals(expected, actual))
    {
        throw new InvalidOperationException($"Expected {expected}, got {actual}.");
    }
}

static void True(bool condition, string message)
{
    if (!condition) throw new InvalidOperationException(message);
}
