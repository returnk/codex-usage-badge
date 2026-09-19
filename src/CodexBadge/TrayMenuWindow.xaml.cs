using System.Windows;
using System.Windows.Interop;
using System.Windows.Threading;
using Drawing = System.Drawing;
using Forms = System.Windows.Forms;

namespace CodexBadge;

public partial class TrayMenuWindow : Window
{
    private readonly DispatcherTimer _closeTimer;

    public event Action? StartupToggleRequested;
    public event Action? RelocateRequested;
    public event Action? ExitRequested;

    public TrayMenuWindow()
    {
        InitializeComponent();
        _closeTimer = new DispatcherTimer(TimeSpan.FromMilliseconds(300), DispatcherPriority.Normal,
            (_, _) => HideMenu(), Dispatcher) { IsEnabled = false };

        SourceInitialized += (_, _) => NativeMethods.MakeNonActivating(new WindowInteropHelper(this).Handle);
        MouseEnter += (_, _) => _closeTimer.Stop();
        MouseLeave += (_, _) => _closeTimer.Start();
        StartupButton.Click += (_, _) => RunAndHide(StartupToggleRequested);
        RelocateButton.Click += (_, _) => RunAndHide(RelocateRequested);
        ExitButton.Click += (_, _) => RunAndHide(ExitRequested);
    }

    public void ShowAtCursor(bool startupEnabled)
    {
        StartupCheck.Visibility = startupEnabled ? Visibility.Visible : Visibility.Hidden;
        _closeTimer.Stop();
        if (!IsVisible) Show();
        UpdateLayout();

        if (!NativeMethods.GetCursorPos(out var cursor)) return;
        var screen = Forms.Screen.FromPoint(new Drawing.Point(cursor.X, cursor.Y)).WorkingArea;
        var hwnd = new WindowInteropHelper(this).Handle;
        var scale = Math.Max(1, NativeMethods.GetDpiForWindow(hwnd)) / 96d;
        var width = (int)Math.Round(ActualWidth * scale);
        var height = (int)Math.Round(ActualHeight * scale);
        var x = Math.Clamp(cursor.X - width + (int)Math.Round(16 * scale), screen.Left, screen.Right - width);
        var y = Math.Clamp(cursor.Y - height + (int)Math.Round(8 * scale), screen.Top, screen.Bottom - height);

        NativeMethods.SetWindowPos(hwnd, NativeMethods.HwndTopMost, x, y, width, height, NativeMethods.SwpNoActivate);
        NativeMethods.ShowWindow(hwnd, NativeMethods.SwShowNoActivate);
    }

    private void RunAndHide(Action? action)
    {
        HideMenu();
        action?.Invoke();
    }

    private void HideMenu()
    {
        _closeTimer.Stop();
        Hide();
    }
}
