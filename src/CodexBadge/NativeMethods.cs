using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows.Automation;
using System.Windows.Interop;
using CodexBadge.Core;
using Microsoft.Win32;

namespace CodexBadge;

internal static class NativeMethods
{
    internal const int GwlExStyle = -20;
    internal const int GwlHwndParent = -8;
    private const uint GwOwner = 4;
    internal const long WsExToolWindow = 0x00000080L;
    internal const long WsExNoActivate = 0x08000000L;
    internal const uint SwpNoActivate = 0x0010;
    internal const uint SwpNoZOrder = 0x0004;
    internal const uint SwpNoSize = 0x0001;
    internal const int SwShowNoActivate = 4;
    private const int DwmwaExtendedFrameBounds = 9;
    private static readonly object AnchorGate = new();
    private static nint _anchorOwner;
    private static AutomationElement? _toolbarAnchor;
    private static int _toolbarAnchorRight;
    private static int _toolbarAnchorBottom;
    private static long _nextAnchorProbe;
    private static int _anchorProbeRunning;

    internal static readonly nint HwndTop = 0;
    internal static readonly nint HwndTopMost = -1;

    [StructLayout(LayoutKind.Sequential)]
    internal struct RECT
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
        public int Width => Right - Left;
        public int Height => Bottom - Top;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct POINT
    {
        public int X;
        public int Y;
    }

    internal delegate bool EnumWindowsProc(nint hwnd, nint lParam);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, nint lParam);

    [DllImport("user32.dll")]
    internal static extern bool IsWindowVisible(nint hwnd);

    [DllImport("user32.dll")]
    internal static extern bool IsIconic(nint hwnd);

    [DllImport("user32.dll")]
    internal static extern nint GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern nint GetWindow(nint hwnd, uint command);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(nint hwnd, out uint processId);

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static extern nint GetWindowLongPtr(nint hwnd, int index);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern nint SetWindowLongPtr(nint hwnd, int index, nint value);

    [DllImport("user32.dll")]
    internal static extern bool SetWindowPos(nint hwnd, nint insertAfter, int x, int y, int width, int height, uint flags);

    [DllImport("user32.dll")]
    internal static extern bool ShowWindow(nint hwnd, int command);

    [DllImport("user32.dll")]
    internal static extern bool GetCursorPos(out POINT point);

    [DllImport("user32.dll")]
    internal static extern uint GetDpiForWindow(nint hwnd);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(nint hwnd, out RECT rect);

    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(nint hwnd, int attribute, out RECT value, int size);

    internal static void MakeNonActivating(nint hwnd)
    {
        var style = GetWindowLongPtr(hwnd, GwlExStyle).ToInt64();
        SetWindowLongPtr(hwnd, GwlExStyle, new nint(style | WsExToolWindow | WsExNoActivate));
    }

    internal static void SetOwner(nint child, nint owner) => SetWindowLongPtr(child, GwlHwndParent, owner);

    internal static bool TryGetFrameBounds(nint hwnd, out RECT rect)
    {
        if (DwmGetWindowAttribute(hwnd, DwmwaExtendedFrameBounds, out rect, Marshal.SizeOf<RECT>()) == 0) return true;
        return GetWindowRect(hwnd, out rect);
    }

    internal static bool TryGetWindowBounds(nint hwnd, out RECT rect) => GetWindowRect(hwnd, out rect);

    internal static bool TryGetToolbarAnchor(
        nint hwnd, RECT frame, double scale,
        out int right, out int bottom)
    {
        lock (AnchorGate)
        {
            if (_anchorOwner != hwnd)
            {
                _anchorOwner = hwnd;
                _toolbarAnchor = null;
                _toolbarAnchorRight = 0;
                _toolbarAnchorBottom = 0;
                _nextAnchorProbe = 0;
            }
            right = _toolbarAnchorRight;
            bottom = _toolbarAnchorBottom;
        }

        if (Environment.TickCount64 >= Interlocked.Read(ref _nextAnchorProbe) &&
            Interlocked.CompareExchange(ref _anchorProbeRunning, 1, 0) == 0)
        {
            Interlocked.Exchange(ref _nextAnchorProbe, Environment.TickCount64 + 750);
            _ = Task.Run(() => ProbeToolbarAnchor(hwnd, frame, scale));
        }
        return right > 0 && bottom > 0;
    }

    private static void ProbeToolbarAnchor(nint hwnd, RECT frame, double scale)
    {
        try
        {
            AutomationElement? cached;
            lock (AnchorGate) cached = _anchorOwner == hwnd ? _toolbarAnchor : null;
            if (TryReadToolbarAnchor(cached, frame, scale, out var cachedRight, out var cachedBottom))
            {
                StoreToolbarAnchor(hwnd, cached, cachedRight, cachedBottom);
                return;
            }

            var root = AutomationElement.FromHandle(hwnd);
            var controls = root.FindAll(TreeScope.Descendants, new OrCondition(
                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Button),
                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Text)));
            AutomationElement? best = null;
            foreach (AutomationElement element in controls)
            {
                if (!QuotaAnchorCandidate.IsExternalProcess(element.Current.ProcessId, Environment.ProcessId)) continue;
                if (!IsVoiceControl(element.Current.Name)) continue;
                var parent = element;
                var outerRight = 0;
                var outerBottom = 0;
                for (var depth = 0; depth < 8 && parent is not null; depth++)
                {
                    var bounds = parent.Current.BoundingRectangle;
                    if (IsSidebarContainer(bounds, frame, scale))
                    {
                        best = parent;
                        outerRight = SidebarAnchorCandidate.ChooseOuterRight(
                            outerRight, (int)Math.Round(bounds.Right));
                        outerBottom = (int)Math.Round(bounds.Bottom);
                    }
                    parent = TreeWalker.RawViewWalker.GetParent(parent);
                }
                if (best is not null)
                {
                    StoreToolbarAnchor(hwnd, best, outerRight, outerBottom);
                    return;
                }
            }

            StoreToolbarAnchor(hwnd, null, 0, 0);
        }
        catch (ElementNotAvailableException) { }
        catch (InvalidOperationException) { }
        catch (COMException) { }
        finally { Interlocked.Exchange(ref _anchorProbeRunning, 0); }
    }

    private static void StoreToolbarAnchor(
        nint hwnd, AutomationElement? element, int right, int bottom)
    {
        lock (AnchorGate)
        {
            if (_anchorOwner != hwnd) return;
            _toolbarAnchor = element;
            _toolbarAnchorRight = right;
            _toolbarAnchorBottom = bottom;
        }
    }

    private static bool TryReadToolbarAnchor(
        AutomationElement? element, RECT frame, double scale,
        out int right, out int bottom)
    {
        right = 0;
        bottom = 0;
        if (element is null) return false;
        try
        {
            if (!QuotaAnchorCandidate.IsExternalProcess(element.Current.ProcessId, Environment.ProcessId)) return false;
            var bounds = element.Current.BoundingRectangle;
            if (!IsSidebarContainer(bounds, frame, scale)) return false;
            right = (int)Math.Round(bounds.Right);
            bottom = (int)Math.Round(bounds.Bottom);
            return true;
        }
        catch (ElementNotAvailableException) { return false; }
        catch (InvalidOperationException) { return false; }
        catch (COMException) { return false; }
    }

    private static bool IsVoiceControl(string? name) =>
        !string.IsNullOrWhiteSpace(name) &&
        (name.Contains("语音", StringComparison.OrdinalIgnoreCase) ||
         name.Contains("Voice", StringComparison.OrdinalIgnoreCase));

    private static bool IsSidebarContainer(System.Windows.Rect bounds, RECT frame, double scale) =>
        !bounds.IsEmpty && SidebarAnchorCandidate.IsContainer(
            frame.Left, frame.Right, frame.Bottom, scale,
            bounds.Left, bounds.Right, bounds.Bottom);

    internal static IReadOnlyList<nint> FindCodexWindows()
    {
        var ownProcessId = Environment.ProcessId;
        var result = new List<nint>();
        EnumWindows((hwnd, _) =>
        {
            var visible = IsWindowVisible(hwnd);
            if (!visible) return true;
            GetWindowThreadProcessId(hwnd, out var processId);
            if (processId == ownProcessId || processId == 0) return true;
            try
            {
                using var process = Process.GetProcessById((int)processId);
                if (!IsCodexDesktopProcess(process) || !TryGetFrameBounds(hwnd, out var bounds)) return true;
                var toolWindow = (GetWindowLongPtr(hwnd, GwlExStyle).ToInt64() & WsExToolWindow) != 0;
                var hasOwner = GetWindow(hwnd, GwOwner) != 0;
                if (CodexWindowClassifier.IsMainWindow(
                        visible, toolWindow, hasOwner, bounds.Width, bounds.Height))
                {
                    result.Add(hwnd);
                }
            }
            catch (ArgumentException) { }
            catch (InvalidOperationException) { }
            catch (System.ComponentModel.Win32Exception) { }
            return true;
        }, 0);
        return result;
    }

    private static bool IsCodexDesktopProcess(Process process)
    {
        if (string.Equals(process.ProcessName, "Codex", StringComparison.OrdinalIgnoreCase)) return true;
        if (!string.Equals(process.ProcessName, "ChatGPT", StringComparison.OrdinalIgnoreCase)) return false;
        try
        {
            var path = process.MainModule?.FileName;
            return path is not null &&
                   (path.Contains("OpenAI.Codex_", StringComparison.OrdinalIgnoreCase) ||
                    path.Contains(@"\OpenAI\Codex\", StringComparison.OrdinalIgnoreCase));
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return false;
        }
    }
}

internal static class ThemeReader
{
    internal static bool IsDarkMode()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return key?.GetValue("AppsUseLightTheme") is int value && value == 0;
        }
        catch
        {
            return false;
        }
    }
}
