using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;
using System.Text.Json;

namespace CodexBadge;

internal sealed class AppServerClient : IAsyncDisposable
{
    private readonly string _executable;
    private readonly ConcurrentDictionary<long, TaskCompletionSource<JsonElement>> _pending = new();
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly CancellationTokenSource _lifetime = new();
    private Process? _process;
    private long _nextId;

    internal event Action? RateLimitsUpdated;
    internal event Action? Exited;

    internal AppServerClient(string executable) => _executable = executable;

    internal async Task StartAsync(CancellationToken cancellationToken)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = _executable,
            Arguments = "app-server --stdio",
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };

        _process = new Process { StartInfo = startInfo, EnableRaisingEvents = true };
        _process.Exited += (_, _) =>
        {
            FailPending(new IOException("Codex app-server exited."));
            Exited?.Invoke();
        };
        if (!_process.Start()) throw new IOException("Unable to start Codex app-server.");

        _ = ReadLoopAsync(_lifetime.Token);
        _ = DrainErrorsAsync(_lifetime.Token);

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(TimeSpan.FromSeconds(10));
        await RequestAsync("initialize", new
        {
            clientInfo = new { name = "codex_badge", title = "Codex Badge", version = "1.0.0" },
        }, timeout.Token);
        await SendAsync(new { method = "initialized", @params = new { } }, timeout.Token);
    }

    internal Task<JsonElement> ReadRateLimitsAsync(CancellationToken cancellationToken) =>
        RequestAsync("account/rateLimits/read", new { }, cancellationToken);

    private async Task<JsonElement> RequestAsync(string method, object parameters, CancellationToken cancellationToken)
    {
        var id = Interlocked.Increment(ref _nextId);
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!_pending.TryAdd(id, completion)) throw new InvalidOperationException("Duplicate request identifier.");
        try
        {
            await SendAsync(new { id, method, @params = parameters }, cancellationToken);
            return await completion.Task.WaitAsync(cancellationToken);
        }
        finally
        {
            _pending.TryRemove(id, out _);
        }
    }

    private async Task SendAsync(object message, CancellationToken cancellationToken)
    {
        var process = _process ?? throw new InvalidOperationException("App-server is not running.");
        var line = JsonSerializer.Serialize(message);
        await _writeLock.WaitAsync(cancellationToken);
        try
        {
            await process.StandardInput.WriteLineAsync(line.AsMemory(), cancellationToken);
            await process.StandardInput.FlushAsync(cancellationToken);
        }
        finally
        {
            _writeLock.Release();
        }
    }

    private async Task ReadLoopAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (_process is { } process && await process.StandardOutput.ReadLineAsync(cancellationToken) is { } line)
            {
                HandleLine(line);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { }
        catch (Exception error)
        {
            FailPending(error);
        }
    }

    private void HandleLine(string line)
    {
        try
        {
            using var document = JsonDocument.Parse(line);
            var root = document.RootElement;
            if (root.TryGetProperty("id", out var idElement) && idElement.TryGetInt64(out var id) && _pending.TryGetValue(id, out var completion))
            {
                if (root.TryGetProperty("error", out var error))
                {
                    completion.TrySetException(new IOException(error.ToString()));
                }
                else if (root.TryGetProperty("result", out var result))
                {
                    completion.TrySetResult(result.Clone());
                }
                return;
            }

            if (root.TryGetProperty("method", out var method) && method.GetString() == Core.AppServerProtocol.RateLimitUpdatedMethod)
            {
                RateLimitsUpdated?.Invoke();
            }
        }
        catch (JsonException) { }
    }

    private async Task DrainErrorsAsync(CancellationToken cancellationToken)
    {
        try
        {
            while (_process is { } process && await process.StandardError.ReadLineAsync(cancellationToken) is not null) { }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested) { }
        catch (ObjectDisposedException) { }
    }

    private void FailPending(Exception error)
    {
        foreach (var completion in _pending.Values) completion.TrySetException(error);
    }

    public async ValueTask DisposeAsync()
    {
        _lifetime.Cancel();
        var process = _process;
        _process = null;
        if (process is not null)
        {
            try
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
                await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(2));
            }
            catch (InvalidOperationException) { }
            catch (System.ComponentModel.Win32Exception) { }
            catch (TimeoutException) { }
            finally { process.Dispose(); }
        }
        _writeLock.Dispose();
        _lifetime.Dispose();
    }
}
