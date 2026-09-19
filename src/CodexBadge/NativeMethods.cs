using System.Diagnostics;
using System.Runtime.InteropServices;
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
