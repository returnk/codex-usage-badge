using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using CodexBadge.Core;

namespace CodexBadge;

public partial class CapsuleWindow : Window
{
    private bool _dragging;
    private bool _dragMoved;
    private NativeMethods.POINT _startCursor;

    public event Action<bool>? HoverChanged;
    public event Action<int>? WheelAdjusted;
    public event Action? PositionResetRequested;
    public event Action<int, int>? DragStarted;
    public event Action<int, int, bool>? DragChanged;

    public CapsuleWindow()
    {
        InitializeComponent();
        SourceInitialized += (_, _) => NativeMethods.MakeNonActivating(new WindowInteropHelper(this).Handle);
        MouseEnter += (_, _) => HoverChanged?.Invoke(true);
        MouseLeave += (_, _) => HoverChanged?.Invoke(false);
        MouseWheel += (_, e) => WheelAdjusted?.Invoke(e.Delta);
        MouseLeftButtonDown += OnMouseLeftButtonDown;
        MouseLeftButtonUp += OnMouseLeftButtonUp;
        MouseMove += OnMouseMove;
        LostMouseCapture += OnLostMouseCapture;
    }

    public void SetPercent(double? remaining) =>
        PercentText.Text = remaining is null ? "--%" : $"{Math.Round(remaining.Value):0}%";

    public void ApplyTheme(CapsuleTheme theme)
    {
        var (background, foreground, border) = theme switch
        {
            CapsuleTheme.FrostLight => ("#FFF4F8FF", "#FF245AA8", "#66377DFF"),
            CapsuleTheme.GraphiteDark => ("#FF25272C", "#FFFFFFFF", "#50377DFF"),
            _ => ("#FF377DFF", "#FFFFFFFF", "#00377DFF"),
        };
        CapsuleSurface.Background = Brush(background);
        CapsuleSurface.BorderBrush = Brush(border);
        PercentText.Foreground = Brush(foreground);
    }

    private static SolidColorBrush Brush(string value) =>
        new((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(value));

    private void OnMouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount == 2)
        {
            PositionResetRequested?.Invoke();
            return;
        }

        _dragging = NativeMethods.GetCursorPos(out _startCursor);
        _dragMoved = false;
        if (_dragging) Mouse.Capture(this);
    }

    private void OnMouseMove(object sender, System.Windows.Input.MouseEventArgs e)
    {
        if (!_dragging || e.LeftButton != MouseButtonState.Pressed || !NativeMethods.GetCursorPos(out var current)) return;
        if (current.X == _startCursor.X && current.Y == _startCursor.Y) return;
        if (!_dragMoved)
        {
            _dragMoved = true;
            DragStarted?.Invoke(_startCursor.X, _startCursor.Y);
        }
        DragChanged?.Invoke(current.X, current.Y, false);
    }

    private void OnMouseLeftButtonUp(object sender, MouseButtonEventArgs e)
    {
        if (!_dragging) return;
        CompleteDrag(releaseCapture: true);
    }

    private void OnLostMouseCapture(object sender, System.Windows.Input.MouseEventArgs e) =>
        CompleteDrag(releaseCapture: false);

    private void CompleteDrag(bool releaseCapture)
    {
        if (!_dragging) return;
        _dragging = false;
        if (releaseCapture && Mouse.Captured == this) Mouse.Capture(null);
        if (_dragMoved && NativeMethods.GetCursorPos(out var current))
        {
            DragChanged?.Invoke(current.X, current.Y, true);
        }
    }
}
