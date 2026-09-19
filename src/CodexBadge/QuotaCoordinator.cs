using CodexBadge.Core;
using System.IO;

namespace CodexBadge;

internal sealed class QuotaCoordinator : IAsyncDisposable
{
    private readonly QuotaCache _cache = new();
    private readonly RetryBackoff _backoff = new();
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly System.Threading.Timer _minuteTimer;
    private readonly System.Threading.Timer _retryTimer;
    private readonly object _stateLock = new();
    private AppServerClient? _client;
    private CancellationTokenSource? _notificationDebounce;
    private DateTimeOffset _nextStart = DateTimeOffset.MinValue;
    private bool _codexPresent;
    private bool _disposed;

    internal event Action<QuotaView>? Changed;

    internal QuotaCoordinator()
    {
        _minuteTimer = new System.Threading.Timer(_ => _ = RefreshAsync(), null, Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
        _retryTimer = new System.Threading.Timer(_ => _ = RefreshAsync(), null, Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
    }

    internal QuotaView Current => _cache.Get(DateTimeOffset.Now);

    internal async Task SetCodexPresentAsync(bool present, bool immediateRefresh = false)
    {
        _codexPresent = present;
        if (!present)
        {
            _minuteTimer.Change(Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
            _retryTimer.Change(Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
            await StopClientAsync();
            return;
        }

        _minuteTimer.Change(TimeSpan.FromMinutes(1), TimeSpan.FromMinutes(1));
        await EnsureClientAsync();
        if (immediateRefresh) await RefreshAsync();
    }

    internal Task RefreshOnHoverIfNeededAsync()
    {
        var current = Current;
        return current.Freshness != QuotaFreshness.Fresh ||
               current.Snapshot is null ||
               DateTimeOffset.Now - current.Snapshot.FetchedAt >= TimeSpan.FromMinutes(1)
            ? RefreshAsync()
            : Task.CompletedTask;
    }

    internal async Task RefreshAsync()
    {
        if (!_codexPresent || _disposed || !await _gate.WaitAsync(0)) return;
        try
        {
            await EnsureClientCoreAsync();
            AppServerClient? client;
            lock (_stateLock) client = _client;
            if (client is null)
            {
                _cache.SetFailure();
                RaiseChanged();
                return;
            }

            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            var result = await client.ReadRateLimitsAsync(timeout.Token);
            _cache.SetSuccess(QuotaMapper.Map(result, DateTimeOffset.Now));
            _backoff.RegisterSuccess();
            RaiseChanged();
        }
        catch (Exception error) when (error is IOException or InvalidOperationException or OperationCanceledException or System.ComponentModel.Win32Exception)
        {
            _cache.SetFailure();
            RaiseChanged();
        }
        finally
        {
            _gate.Release();
        }
    }

    private async Task EnsureClientAsync()
    {
        await _gate.WaitAsync();
        try { await EnsureClientCoreAsync(); }
        finally { _gate.Release(); }
    }

    private async Task EnsureClientCoreAsync()
    {
        lock (_stateLock)
        {
            if (_client is not null || DateTimeOffset.Now < _nextStart) return;
        }

        var executable = CodexCliLocator.Find();
        if (executable is null)
        {
            ScheduleRetry();
            return;
        }

        var client = new AppServerClient(executable);
        client.RateLimitsUpdated += ScheduleNotificationRefresh;
        client.Exited += OnClientExited;
        try
        {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(12));
            await client.StartAsync(timeout.Token);
            lock (_stateLock) _client = client;
            _backoff.RegisterSuccess();
            _retryTimer.Change(Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
        }
        catch
        {
            await client.DisposeAsync();
            ScheduleRetry();
        }
    }

    private void ScheduleNotificationRefresh()
    {
        var next = new CancellationTokenSource();
        var old = Interlocked.Exchange(ref _notificationDebounce, next);
        old?.Cancel();
        old?.Dispose();
        _ = Task.Run(async () =>
        {
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(1), next.Token);
                await RefreshAsync();
            }
            catch (OperationCanceledException) { }
        });
    }

    private void OnClientExited()
    {
        AppServerClient? client;
        lock (_stateLock)
        {
            client = _client;
            _client = null;
        }
        if (client is not null) _ = client.DisposeAsync().AsTask();
        _cache.SetFailure();
        ScheduleRetry();
        RaiseChanged();
    }

    private void ScheduleRetry()
    {
        var delay = _backoff.RegisterFailure();
        _nextStart = DateTimeOffset.Now + delay;
        if (_codexPresent && !_disposed) _retryTimer.Change(delay, Timeout.InfiniteTimeSpan);
    }

    private void RaiseChanged() => Changed?.Invoke(Current);

    private async Task StopClientAsync()
    {
        AppServerClient? client;
        lock (_stateLock)
        {
            client = _client;
            _client = null;
        }
        if (client is not null)
        {
            client.RateLimitsUpdated -= ScheduleNotificationRefresh;
            client.Exited -= OnClientExited;
            await client.DisposeAsync();
        }
        _backoff.RegisterSuccess();
        _nextStart = DateTimeOffset.MinValue;
        _retryTimer.Change(Timeout.InfiniteTimeSpan, Timeout.InfiniteTimeSpan);
    }

    public async ValueTask DisposeAsync()
    {
        _disposed = true;
        _codexPresent = false;
        _minuteTimer.Dispose();
        _retryTimer.Dispose();
        _notificationDebounce?.Cancel();
        _notificationDebounce?.Dispose();
        await StopClientAsync();
        _gate.Dispose();
    }
}
