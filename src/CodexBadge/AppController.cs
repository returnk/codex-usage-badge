using System.IO;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Threading;
using CodexBadge.Core;
using Microsoft.Win32;
using Forms = System.Windows.Forms;

namespace CodexBadge;

internal sealed class AppController : IDisposable
{
    private const double CapsuleWidth = 63;
    private const double CapsuleHeight = 24;
    private const double DetailWidth = 270;
    private const double DetailHeight = 148;
    private const double DetailGap = 6;

    private readonly Dispatcher _dispatcher;
    private readonly CapsuleWindow _capsule = new();
    private readonly DetailWindow _detail = new();
    private readonly QuotaCoordinator _quota = new();
    private readonly SettingsStore _settingsStore;
    private readonly CapsuleSettings _settings;
    private readonly DispatcherTimer _trackingTimer;
    private readonly DispatcherTimer _hoverCloseTimer;
    private readonly DispatcherTimer _postDragHoverTimer;
    private readonly DispatcherTimer _saveTimer;
    private readonly TrayMenuWindow _trayMenu = new();
    private readonly Forms.NotifyIcon _tray;
    private IReadOnlyList<nint> _candidates = [];
    private nint _owner;
    private CapsulePoint _dragCursorStart;
    private CapsulePoint _dragWindowStart;
    private int _ticks;
    private bool _capsuleVisible;
    private bool _detailVisible;
    private bool _capsuleHovered;
    private bool _detailHovered;
    private bool _wasMinimized;
    private bool _wasForeground;
    private bool _hadCodex;
    private bool _dragging;
    private bool _disposed;

    internal AppController(Dispatcher dispatcher)
    {
        _dispatcher = dispatcher;
        var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        var settingsPath = Path.Combine(localAppData, "CodexBadge", "settings.json");
        SettingsStore.MigrateLegacyFile(
            Path.Combine(localAppData, "CodexCapsule", "settings.json"),
            settingsPath);
        _settingsStore = new SettingsStore(settingsPath);
        _settings = _settingsStore.Load();

        _trackingTimer = new DispatcherTimer(TimeSpan.FromMilliseconds(200), DispatcherPriority.Background, OnTrackingTick, dispatcher);
        _hoverCloseTimer = new DispatcherTimer(TimeSpan.FromMilliseconds(300), DispatcherPriority.Normal, OnHoverClose, dispatcher) { IsEnabled = false };
        _postDragHoverTimer = new DispatcherTimer(TimeSpan.FromMilliseconds(300), DispatcherPriority.Normal, OnPostDragHover, dispatcher) { IsEnabled = false };
        _saveTimer = new DispatcherTimer(TimeSpan.FromMilliseconds(300), DispatcherPriority.Background, (_, _) => SaveNow(), dispatcher) { IsEnabled = false };

        _capsule.HoverChanged += hovering => OnHoverChanged(isCapsule: true, hovering);
        _detail.HoverChanged += hovering => OnHoverChanged(isCapsule: false, hovering);
        _capsule.WheelAdjusted += CycleTheme;
        _capsule.PositionResetRequested += ResetPosition;
        _capsule.DragStarted += OnDragStarted;
        _capsule.DragChanged += OnDragChanged;
        _quota.Changed += view => _dispatcher.BeginInvoke(() => UpdateQuota(view));

        _trayMenu.StartupToggleRequested += ToggleStartup;
        _trayMenu.RelocateRequested += ResetPosition;
        _trayMenu.ExitRequested += () => System.Windows.Application.Current.Shutdown();
        _tray = BuildTrayIcon();
    }

    internal void Start()
    {
        ApplyTheme();
        _capsule.Show();
        _detail.Show();
        _capsule.Hide();
        _detail.Hide();
        _capsuleVisible = false;
        _detailVisible = false;
        UpdateQuota(_quota.Current);

        StartupRegistration.SetEnabled(_settings.StartWithWindows);
        _settingsStore.Save(_settings);
        SystemEvents.PowerModeChanged += OnPowerModeChanged;
        SystemEvents.UserPreferenceChanged += OnUserPreferenceChanged;
        _tray.Visible = true;
        _trackingTimer.Start();
    }

    private Forms.NotifyIcon BuildTrayIcon()
    {
        var tray = new Forms.NotifyIcon
        {
            Text = "Codex Badge",
            Icon = AppIcon.LoadDrawingIcon(),
        };
        tray.MouseUp += (_, e) =>
        {
            if (e.Button == Forms.MouseButtons.Right)
                _dispatcher.BeginInvoke(() => _trayMenu.ShowAtCursor(_settings.StartWithWindows));
        };
        return tray;
    }

    private void OnTrackingTick(object? sender, EventArgs e)
    {
        if (_dragging) return;
        _ticks++;
        if (_ticks % 5 == 1) _candidates = NativeMethods.FindCodexWindows();

        var foreground = NativeMethods.GetForegroundWindow();
        var foregroundCodex = _candidates.Contains(foreground);
        nint nextOwner = foregroundCodex
            ? foreground
            : _candidates.Contains(_owner) ? _owner : _candidates.FirstOrDefault();

        if (nextOwner == 0)
        {
            HideWindows();
            if (_hadCodex)
            {
                _hadCodex = false;
                _ = _quota.SetCodexPresentAsync(false);
            }
            _owner = 0;
            return;
        }

        var newBinding = nextOwner != _owner;
        _owner = nextOwner;
        if (!_hadCodex)
        {
            _hadCodex = true;
            _ = _quota.SetCodexPresentAsync(true, immediateRefresh: true);
        }
        else if (newBinding)
        {
            _ = _quota.RefreshAsync();
        }

        var minimized = NativeMethods.IsIconic(_owner) || !NativeMethods.IsWindowVisible(_owner);
        if (minimized)
        {
            HideWindows();
            _wasMinimized = true;
            return;
        }

        if (_wasMinimized)
        {
            _wasMinimized = false;
            _ = _quota.RefreshAsync();
        }

        if (newBinding)
        {
            AssignOwner();
        }
        if (!_capsuleVisible)
        {
            _capsule.Show();
            _capsuleVisible = true;
        }

        var activated = foregroundCodex && !_wasForeground;
        _wasForeground = foregroundCodex;
        PositionWindows(activated || newBinding);
        if (_ticks % 5 == 0) UpdateQuota(_quota.Current);
    }

    private void AssignOwner()
    {
        NativeMethods.SetOwner(new WindowInteropHelper(_capsule).Handle, _owner);
        NativeMethods.SetOwner(new WindowInteropHelper(_detail).Handle, _owner);
    }

    private void PositionWindows(bool bringForward)
    {
        if (_owner == 0 || !NativeMethods.TryGetFrameBounds(_owner, out var frame)) return;
        var scale = Math.Max(1, NativeMethods.GetDpiForWindow(_owner)) / 96d;
        var width = (int)Math.Round(CapsuleWidth * scale);
        var height = (int)Math.Round(CapsuleHeight * scale);
        var point = CapsulePlacement.Calculate(
            frame.Left, frame.Right, frame.Bottom, scale,
            CapsuleWidth, CapsuleHeight, _settings.OffsetX, _settings.OffsetY);
        var x = point.X;
        var y = point.Y;
        var capsuleHwnd = new WindowInteropHelper(_capsule).Handle;
        NativeMethods.SetWindowPos(capsuleHwnd, NativeMethods.HwndTop, x, y, width, height,
            NativeMethods.SwpNoActivate | (bringForward ? 0 : NativeMethods.SwpNoZOrder));

        if (!_detailVisible) return;
        var detailWidth = (int)Math.Round(DetailWidth * scale);
        var detailHeight = (int)Math.Round(DetailHeight * scale);
        var detailX = x + width - detailWidth;
        var detailY = y - detailHeight - (int)Math.Round(DetailGap * scale);
        NativeMethods.SetWindowPos(new WindowInteropHelper(_detail).Handle, NativeMethods.HwndTop,
            detailX, detailY, detailWidth, detailHeight, NativeMethods.SwpNoActivate | (bringForward ? 0 : NativeMethods.SwpNoZOrder));
    }

    private void HideWindows()
    {
        if (_detailVisible)
        {
            _detail.Hide();
            _detailVisible = false;
        }
        if (_capsuleVisible)
        {
            _capsule.Hide();
            _capsuleVisible = false;
        }
        _wasForeground = false;
    }

    private void OnHoverChanged(bool isCapsule, bool hovering)
    {
        if (isCapsule) _capsuleHovered = hovering;
        else _detailHovered = hovering;

        if (_dragging) return;

        if (hovering)
        {
            _hoverCloseTimer.Stop();
            ShowDetail();
            _ = _quota.RefreshOnHoverIfNeededAsync();
        }
        else if (!_capsuleHovered && !_detailHovered)
        {
            _hoverCloseTimer.Stop();
            _hoverCloseTimer.Start();
        }
    }

    private void OnHoverClose(object? sender, EventArgs e)
    {
        _hoverCloseTimer.Stop();
        if (_capsuleHovered || _detailHovered || !_detailVisible) return;
        _detail.Hide();
        _detailVisible = false;
    }

    private void ShowDetail()
    {
        if (_detailVisible || _dragging || _owner == 0 || NativeMethods.IsIconic(_owner)) return;
        _detail.ApplyTheme(_settings.Theme);
        _detail.Show();
        _detailVisible = true;
        PositionWindows(bringForward: _wasForeground);
    }

    private void OnPostDragHover(object? sender, EventArgs e)
    {
        _postDragHoverTimer.Stop();
        if (_capsuleHovered) ShowDetail();
    }

    private void CycleTheme(int wheelDelta)
    {
        _settings.Theme = ThemeMath.Cycle(_settings.Theme, wheelDelta);
        ApplyTheme();
        ScheduleSave();
    }

    private void ApplyTheme()
    {
        _capsule.ApplyTheme(_settings.Theme);
        _detail.ApplyTheme(_settings.Theme);
    }

    private void ResetPosition()
    {
        _settings.ResetPosition();
        PositionWindows(bringForward: false);
        ScheduleSave();
    }

    private void ToggleStartup()
    {
        _settings.StartWithWindows = !_settings.StartWithWindows;
        StartupRegistration.SetEnabled(_settings.StartWithWindows);
        ScheduleSave();
    }

    private void OnDragStarted(int cursorX, int cursorY)
    {
        var capsuleHwnd = new WindowInteropHelper(_capsule).Handle;
        if (!NativeMethods.TryGetWindowBounds(capsuleHwnd, out var bounds)) return;
        _dragging = true;
        _trackingTimer.Stop();
        _dragCursorStart = new CapsulePoint(cursorX, cursorY);
        _dragWindowStart = new CapsulePoint(bounds.Left, bounds.Top);
        _hoverCloseTimer.Stop();
        _postDragHoverTimer.Stop();
        if (_detailVisible)
        {
            _detail.Hide();
            _detailVisible = false;
        }
    }

    private void OnDragChanged(int cursorX, int cursorY, bool completed)
    {
        if (!_dragging) return;
        var position = DragMath.CalculatePosition(
            _dragWindowStart,
            _dragCursorStart,
            new CapsulePoint(cursorX, cursorY));
        NativeMethods.SetWindowPos(
            new WindowInteropHelper(_capsule).Handle,
            NativeMethods.HwndTop,
            position.X,
            position.Y,
            0,
            0,
            NativeMethods.SwpNoActivate | NativeMethods.SwpNoZOrder | NativeMethods.SwpNoSize);
        if (!completed) return;

        if (_owner != 0)
        {
            var scale = Math.Max(1, NativeMethods.GetDpiForWindow(_owner)) / 96d;
            _settings.OffsetX += (position.X - _dragWindowStart.X) / scale;
            _settings.OffsetY += (position.Y - _dragWindowStart.Y) / scale;
            ScheduleSave();
        }

        _dragging = false;
        PositionWindows(bringForward: false);
        _trackingTimer.Start();
        if (_capsuleHovered)
        {
            _postDragHoverTimer.Stop();
            _postDragHoverTimer.Start();
        }
    }

    private void ScheduleSave()
    {
        _saveTimer.Stop();
        _saveTimer.Start();
    }

    private void SaveNow()
    {
        _saveTimer.Stop();
        _settingsStore.Save(_settings);
    }

    private void UpdateQuota(QuotaView view)
    {
        _capsule.SetPercent(view.Snapshot?.DisplayRemainingPercent);
        _detail.SetQuota(view);
    }

    private void OnPowerModeChanged(object sender, PowerModeChangedEventArgs e)
    {
        if (e.Mode == PowerModes.Resume && _hadCodex) _ = _quota.RefreshAsync();
    }

    private void OnUserPreferenceChanged(object sender, UserPreferenceChangedEventArgs e) =>
        _dispatcher.BeginInvoke(ApplyTheme);

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        _trackingTimer.Stop();
        _hoverCloseTimer.Stop();
        _postDragHoverTimer.Stop();
        _saveTimer.Stop();
        SaveNow();
        SystemEvents.PowerModeChanged -= OnPowerModeChanged;
        SystemEvents.UserPreferenceChanged -= OnUserPreferenceChanged;
        _tray.Visible = false;
        _tray.Dispose();
        _trayMenu.Close();
        _quota.DisposeAsync().AsTask().GetAwaiter().GetResult();
        _detail.Close();
        _capsule.Close();
    }
}

internal static class StartupRegistration
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "CodexBadge";
    private const string LegacyValueName = "CodexCapsule";

    internal static void SetEnabled(bool enabled)
    {
        try
        {
            using var key = Registry.CurrentUser.CreateSubKey(RunKey);
            key.DeleteValue(LegacyValueName, throwOnMissingValue: false);
            if (enabled && Environment.ProcessPath is { } path) key.SetValue(ValueName, $"\"{path}\"");
            else key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
        catch (Exception error) when (error is UnauthorizedAccessException or System.Security.SecurityException or IOException)
        {
            try
            {
                var directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "CodexBadge");
                Directory.CreateDirectory(directory);
                File.WriteAllText(Path.Combine(directory, "startup-error.log"), $"{DateTimeOffset.Now:O} {error}");
            }
            catch { }
        }
    }
}
