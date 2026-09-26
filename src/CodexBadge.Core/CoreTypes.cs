using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace CodexBadge.Core;

public sealed record QuotaWindow(double RemainingPercent, int WindowDurationMinutes, DateTimeOffset? ResetsAt);

public sealed record QuotaSnapshot(
    QuotaWindow? FiveHour,
    QuotaWindow? Weekly,
    int? ResetCreditCount,
    DateTimeOffset FetchedAt,
    QuotaFreshness Freshness = QuotaFreshness.Fresh)
{
    public bool IsWeeklyExhausted => Weekly is { RemainingPercent: <= 0 };

    public double? DisplayRemainingPercent =>
        IsWeeklyExhausted ? 0 : FiveHour?.RemainingPercent ?? Weekly?.RemainingPercent;
}

public static class QuotaDisplayText
{
    public static string FormatWeeklyReset(DateTimeOffset? reset, TimeZoneInfo timeZone) =>
        reset is { } value
            ? TimeZoneInfo.ConvertTime(value, timeZone).ToString("M/d'日 'HH:mm", CultureInfo.InvariantCulture)
            : "重置时间未知";
}

public static class QuotaMapper
{
    private const int FiveHourMinutes = 300;
    private const int WeeklyMinutes = 10_080;

    public static QuotaSnapshot Map(JsonElement result, DateTimeOffset fetchedAt)
    {
        var limits = SelectRateLimits(result);
        QuotaWindow? fiveHour = null;
        QuotaWindow? weekly = null;

        if (limits.ValueKind == JsonValueKind.Object)
        {
            foreach (var name in new[] { "primary", "secondary" })
            {
                if (!limits.TryGetProperty(name, out var element) || element.ValueKind != JsonValueKind.Object)
                {
                    continue;
                }

                var window = ParseWindow(element);
                if (window?.WindowDurationMinutes == FiveHourMinutes) fiveHour ??= window;
                if (window?.WindowDurationMinutes == WeeklyMinutes) weekly ??= window;
            }
        }

        return new QuotaSnapshot(fiveHour, weekly, ParseResetCredits(result), fetchedAt);
    }

    private static JsonElement SelectRateLimits(JsonElement result)
    {
        if (result.TryGetProperty("rateLimitsByLimitId", out var byId) && byId.ValueKind == JsonValueKind.Object)
        {
            if (byId.TryGetProperty("codex", out var codex) && codex.ValueKind == JsonValueKind.Object)
            {
                return codex;
            }

            foreach (var property in byId.EnumerateObject())
            {
                if (property.Value.ValueKind == JsonValueKind.Object) return property.Value;
            }
        }

        return result.TryGetProperty("rateLimits", out var fallback) ? fallback : default;
    }

    private static QuotaWindow? ParseWindow(JsonElement element)
    {
        if (!TryReadDouble(element, "usedPercent", out var used) ||
            !TryReadInt(element, "windowDurationMins", out var minutes))
        {
            return null;
        }

        DateTimeOffset? resetsAt = null;
        if (TryReadLong(element, "resetsAt", out var timestamp))
        {
            try { resetsAt = DateTimeOffset.FromUnixTimeSeconds(timestamp); }
            catch (ArgumentOutOfRangeException) { }
        }

        return new QuotaWindow(Math.Clamp(100 - used, 0, 100), minutes, resetsAt);
    }

    private static int? ParseResetCredits(JsonElement result)
    {
        if (!result.TryGetProperty("rateLimitResetCredits", out var credits) ||
            credits.ValueKind != JsonValueKind.Object ||
            !TryReadInt(credits, "availableCount", out var count) ||
            count is < 0 or > 100)
        {
            return null;
        }

        return count;
    }

    private static bool TryReadDouble(JsonElement element, string name, out double value)
    {
        value = default;
        return element.TryGetProperty(name, out var property) && property.TryGetDouble(out value) && double.IsFinite(value);
    }

    private static bool TryReadInt(JsonElement element, string name, out int value)
    {
        value = default;
        return element.TryGetProperty(name, out var property) && property.TryGetInt32(out value);
    }

    private static bool TryReadLong(JsonElement element, string name, out long value)
    {
        value = default;
        return element.TryGetProperty(name, out var property) && property.TryGetInt64(out value);
    }
}

[JsonConverter(typeof(JsonStringEnumConverter<CapsuleTheme>))]
public enum CapsuleTheme
{
    CodexBlue,
    FrostLight,
    GraphiteDark,
    Privacy,
}

public static class ThemeMath
{
    private const int ThemeCount = 4;

    public static CapsuleTheme Cycle(CapsuleTheme current, int wheelDelta)
    {
        if (wheelDelta == 0) return current;
        var direction = Math.Sign(wheelDelta);
        return (CapsuleTheme)(((int)current + direction + ThemeCount) % ThemeCount);
    }
}

public static class CapsuleDisplay
{
    public static string FormatPercent(double? remaining, bool privacyMode, bool reveal) =>
        privacyMode && !reveal
            ? "••%"
            : remaining is null ? "--%" : $"{Math.Round(remaining.Value):0}%";
}

public enum QuotaProgressBand
{
    Orange,
    Yellow,
    Green,
}

public static class QuotaVisuals
{
    public static QuotaProgressBand ProgressBand(double remainingPercent) => remainingPercent switch
    {
        < 20 => QuotaProgressBand.Orange,
        < 40 => QuotaProgressBand.Yellow,
        _ => QuotaProgressBand.Green,
    };
}

public sealed class CapsuleSettings
{
    public const int CurrentLayoutVersion = 6;

    public int LayoutVersion { get; set; }
    public double OffsetX { get; set; }
    public double OffsetY { get; set; }
    public bool StartWithWindows { get; set; } = true;
    public CapsuleTheme Theme { get; set; } = CapsuleTheme.FrostLight;

    public void ResetPosition()
    {
        OffsetX = 0;
        OffsetY = 0;
    }
}

public readonly record struct CapsulePoint(int X, int Y);

public static class CodexWindowClassifier
{
    public static bool IsMainWindow(bool visible, bool toolWindow, bool hasOwner, int width, int height) =>
        visible && !toolWindow && !hasOwner && width >= 280 && height >= 200;
}

public static class DragMath
{
    public static CapsulePoint CalculatePosition(
        CapsulePoint windowStart,
        CapsulePoint cursorStart,
        CapsulePoint cursorCurrent) =>
        new(
            windowStart.X + cursorCurrent.X - cursorStart.X,
            windowStart.Y + cursorCurrent.Y - cursorStart.Y);
}

public static class CapsulePlacement
{
    public const double SidebarWidth = 344;
    public const double RightInset = 2;
    public const double BottomInsetToCapsuleBottom = 10;

    public static CapsulePoint Calculate(
        int frameLeft,
        int frameRight,
        int frameBottom,
        double scale,
        double capsuleWidth,
        double capsuleHeight,
        double offsetX,
        double offsetY)
    {
        scale = double.IsFinite(scale) && scale > 0 ? scale : 1;
        var sidebarRight = Math.Min(frameRight, frameLeft + (int)Math.Round(SidebarWidth));
        var x = sidebarRight - (int)Math.Round((RightInset + capsuleWidth - offsetX) * scale);
        var y = frameBottom - (int)Math.Round((BottomInsetToCapsuleBottom + capsuleHeight - offsetY) * scale);
        return new CapsulePoint(x, y);
    }

    public static CapsulePoint CalculateFromAnchor(
        int anchorRight,
        int frameBottom,
        double scale,
        double capsuleWidth,
        double capsuleHeight,
        double offsetX,
        double offsetY)
    {
        scale = double.IsFinite(scale) && scale > 0 ? scale : 1;
        var x = anchorRight - (int)Math.Round((RightInset + capsuleWidth - offsetX) * scale);
        var y = frameBottom - (int)Math.Round((BottomInsetToCapsuleBottom + capsuleHeight - offsetY) * scale);
        return new CapsulePoint(x, y);
    }

    public static CapsulePoint CalculateFromContainer(
        int containerRight,
        int containerBottom,
        double scale,
        double capsuleWidth,
        double capsuleHeight,
        double offsetX,
        double offsetY) =>
        CalculateFromAnchor(
            containerRight, containerBottom, scale,
            capsuleWidth, capsuleHeight, offsetX, offsetY);

}

public static class QuotaAnchorCandidate
{
    public static bool IsExternalProcess(int candidateProcessId, int ownProcessId) =>
        candidateProcessId > 0 && candidateProcessId != ownProcessId;
}

public static class SidebarAnchorCandidate
{
    public static bool IsContainer(
        int frameLeft, int frameRight, int frameBottom, double scale,
        double left, double right, double bottom)
    {
        scale = double.IsFinite(scale) && scale > 0 ? scale : 1;
        var width = right - left;
        return Math.Abs(left - frameLeft) <= 8 * scale &&
               width >= 280 && width <= 800 &&
               right <= frameRight - 260 * scale &&
               bottom >= frameBottom - 8 * scale && bottom <= frameBottom + 4 * scale;
    }

    public static int ChooseOuterRight(int currentRight, int candidateRight) =>
        Math.Max(currentRight, candidateRight);
}

public sealed class SettingsStore
{
    private static readonly JsonSerializerOptions JsonOptions = new() { WriteIndented = true };
    private readonly string _path;

    public SettingsStore(string path) => _path = path;

    public static void MigrateLegacyFile(string legacyPath, string currentPath)
    {
        try
        {
            if (File.Exists(currentPath) || !File.Exists(legacyPath)) return;
            var directory = Path.GetDirectoryName(currentPath);
            if (!string.IsNullOrEmpty(directory)) Directory.CreateDirectory(directory);
            File.Copy(legacyPath, currentPath, overwrite: false);
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }

    public CapsuleSettings Load()
    {
        try
        {
            if (!File.Exists(_path)) return Normalize(new CapsuleSettings());
            var settings = JsonSerializer.Deserialize<CapsuleSettings>(File.ReadAllText(_path), JsonOptions)
                ?? new CapsuleSettings();
            return Normalize(settings);
        }
        catch (IOException) { return Normalize(new CapsuleSettings()); }
        catch (UnauthorizedAccessException) { return Normalize(new CapsuleSettings()); }
        catch (JsonException) { return Normalize(new CapsuleSettings()); }
    }

    public void Save(CapsuleSettings settings)
    {
        settings.LayoutVersion = CapsuleSettings.CurrentLayoutVersion;
        var directory = Path.GetDirectoryName(_path);
        if (!string.IsNullOrEmpty(directory)) Directory.CreateDirectory(directory);
        File.WriteAllText(_path, JsonSerializer.Serialize(settings, JsonOptions));
    }

    private static CapsuleSettings Normalize(CapsuleSettings settings)
    {
        if (!Enum.IsDefined(settings.Theme)) settings.Theme = CapsuleTheme.FrostLight;
        if (settings.LayoutVersion < CapsuleSettings.CurrentLayoutVersion)
        {
            settings.OffsetX = 0;
            settings.OffsetY = 0;
            settings.LayoutVersion = CapsuleSettings.CurrentLayoutVersion;
        }
        return settings;
    }
}

public sealed class JsonLineBuffer
{
    private readonly Decoder _decoder = Encoding.UTF8.GetDecoder();
    private readonly StringBuilder _pending = new();

    public IReadOnlyList<string> Append(ReadOnlySpan<byte> bytes)
    {
        var chars = new char[Encoding.UTF8.GetMaxCharCount(bytes.Length)];
        _decoder.Convert(bytes, chars, flush: false, out _, out var charsUsed, out _);
        _pending.Append(chars, 0, charsUsed);

        var lines = new List<string>();
        while (true)
        {
            var text = _pending.ToString();
            var newline = text.IndexOf('\n');
            if (newline < 0) break;
            lines.Add(text[..newline].TrimEnd('\r'));
            _pending.Remove(0, newline + 1);
        }
        return lines;
    }
}

public sealed class RetryBackoff
{
    private static readonly TimeSpan[] Delays =
    [
        TimeSpan.FromSeconds(15),
        TimeSpan.FromSeconds(30),
        TimeSpan.FromMinutes(1),
        TimeSpan.FromMinutes(5),
    ];

    private int _failureCount;

    public TimeSpan RegisterFailure()
    {
        var delay = Delays[Math.Min(_failureCount, Delays.Length - 1)];
        _failureCount++;
        return delay;
    }

    public void RegisterSuccess() => _failureCount = 0;
}
