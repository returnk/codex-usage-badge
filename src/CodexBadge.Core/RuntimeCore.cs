using System.Text.Json;

namespace CodexBadge.Core;

public enum QuotaFreshness
{
    Fresh,
    Stale,
    Unavailable,
}

public sealed record QuotaView(QuotaSnapshot? Snapshot, QuotaFreshness Freshness);

public sealed class QuotaCache
{
    public static readonly TimeSpan MaximumStaleAge = TimeSpan.FromMinutes(30);
    private QuotaSnapshot? _lastSuccess;
    private bool _lastReadFailed;

    public void SetSuccess(QuotaSnapshot snapshot)
    {
        _lastSuccess = snapshot with { Freshness = QuotaFreshness.Fresh };
        _lastReadFailed = false;
    }

    public void SetFailure() => _lastReadFailed = true;

    public QuotaView Get(DateTimeOffset now)
    {
        if (_lastSuccess is null || now - _lastSuccess.FetchedAt > MaximumStaleAge)
        {
            return new QuotaView(null, QuotaFreshness.Unavailable);
        }

        var freshness = _lastReadFailed ? QuotaFreshness.Stale : QuotaFreshness.Fresh;
        return new QuotaView(_lastSuccess with { Freshness = freshness }, freshness);
    }
}

public static class AppServerProtocol
{
    public const string RateLimitUpdatedMethod = "account/rateLimits/updated";

    public static bool IsRateLimitUpdate(string line)
    {
        try
        {
            using var document = JsonDocument.Parse(line);
            return document.RootElement.TryGetProperty("method", out var method) &&
                   method.ValueKind == JsonValueKind.String &&
                   string.Equals(method.GetString(), RateLimitUpdatedMethod, StringComparison.Ordinal);
        }
        catch (JsonException)
        {
            return false;
        }
    }
}

public static class CodexCliLocator
{
    public const string OverrideEnvironmentVariable = "CODEX_BADGE_CLI";
    public const string LegacyOverrideEnvironmentVariable = "CODEX_CAPSULE_CLI";

    public static string? Find(IReadOnlyDictionary<string, string?>? environment = null)
    {
        environment ??= Environment.GetEnvironmentVariables()
            .Cast<System.Collections.DictionaryEntry>()
            .ToDictionary(entry => (string)entry.Key, entry => entry.Value?.ToString(), StringComparer.OrdinalIgnoreCase);

        foreach (var variable in new[] { OverrideEnvironmentVariable, LegacyOverrideEnvironmentVariable })
        {
            if (environment.TryGetValue(variable, out var explicitPath) && IsExecutable(explicitPath))
            {
                return Path.GetFullPath(explicitPath!);
            }
        }

        if (environment.TryGetValue("PATH", out var pathValue))
        {
            foreach (var directory in (pathValue ?? string.Empty).Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
            {
                var candidate = Path.Combine(directory.Trim('"'), "codex.exe");
                if (IsExecutable(candidate)) return Path.GetFullPath(candidate);
            }
        }

        if (environment.TryGetValue("LOCALAPPDATA", out var localAppData) && !string.IsNullOrWhiteSpace(localAppData))
        {
            var bin = Path.Combine(localAppData, "OpenAI", "Codex", "bin");
            try
            {
                return Directory.EnumerateFiles(bin, "codex.exe", SearchOption.AllDirectories)
                    .Where(IsExecutable)
                    .OrderByDescending(File.GetLastWriteTimeUtc)
                    .FirstOrDefault();
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        return null;
    }

    private static bool IsExecutable(string? path) => !string.IsNullOrWhiteSpace(path) && File.Exists(path);
}
