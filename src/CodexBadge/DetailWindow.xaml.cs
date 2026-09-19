using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using CodexBadge.Core;

namespace CodexBadge;

public partial class DetailWindow : Window
{
    private int? _creditCount;
    private bool _weeklyExhausted;
    private System.Windows.Media.Brush _foreground = System.Windows.Media.Brushes.Black;
    private System.Windows.Media.Brush _muted = System.Windows.Media.Brushes.Gray;
    private System.Windows.Media.Brush _warning = System.Windows.Media.Brushes.DarkOrange;
    private System.Windows.Media.Brush _blockedProgress = System.Windows.Media.Brushes.Gray;

    public event Action<bool>? HoverChanged;

    public DetailWindow()
    {
        InitializeComponent();
        SourceInitialized += (_, _) => NativeMethods.MakeNonActivating(new WindowInteropHelper(this).Handle);
        MouseEnter += (_, _) => HoverChanged?.Invoke(true);
        MouseLeave += (_, _) => HoverChanged?.Invoke(false);
        ApplyTheme(CapsuleTheme.CodexBlue);
    }

    public void SetQuota(QuotaView view)
    {
        var snapshot = view.Snapshot;
        if (snapshot is null)
        {
            _weeklyExhausted = false;
            FiveHourUnavailableRun.Text = string.Empty;
            FiveHourResetText.Text = "暂时无法读取额度";
            FiveHourResetText.Foreground = _muted;
            FiveHourPercentText.Text = "--%";
            FiveHourProgress.Value = 0;
            WeeklyText.Text = "本周剩余";
            WeeklyPercentText.Text = "--%";
            WeeklyResetText.Text = "重置时间未知";
            _creditCount = null;
            UpdateCreditText();
            StatusText.Text = string.Empty;
            return;
        }

        _weeklyExhausted = snapshot.IsWeeklyExhausted;
        FiveHourUnavailableRun.Text = _weeklyExhausted ? "（暂不可用）" : string.Empty;
        FiveHourResetText.Text = _weeklyExhausted
            ? "本周额度已用完，5小时额度暂不可用"
            : FormatReset(snapshot.FiveHour);
        FiveHourResetText.Foreground = _weeklyExhausted ? _warning : _muted;
        FiveHourPercentText.Text = FormatPercent(snapshot.FiveHour?.RemainingPercent);
        FiveHourProgress.Value = snapshot.FiveHour?.RemainingPercent ?? 0;
        SetProgressBrush(snapshot.FiveHour?.RemainingPercent, _weeklyExhausted);
        WeeklyText.Text = "本周剩余";
        WeeklyPercentText.Text = FormatPercent(snapshot.Weekly?.RemainingPercent);
        WeeklyResetText.Text = QuotaDisplayText.FormatWeeklyReset(
            snapshot.Weekly?.ResetsAt, TimeZoneInfo.Local);
        _creditCount = snapshot.ResetCreditCount;
        UpdateCreditText();
        StatusText.Text = view.Freshness == QuotaFreshness.Stale
            ? $"更新于 {snapshot.FetchedAt.ToLocalTime():HH:mm}"
            : string.Empty;
    }

    public void ApplyTheme(CapsuleTheme theme)
    {
        var dark = theme == CapsuleTheme.GraphiteDark ||
                   (theme == CapsuleTheme.CodexBlue && ThemeReader.IsDarkMode());
        var background = theme == CapsuleTheme.FrostLight ? "#FFF4F8FF" : dark ? "#FF25272C" : "#FFFFFFFF";
        Card.Background = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(background));
        Card.BorderBrush = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#3BFFFFFF" : "#24000000"));
        _foreground = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#F5FFFFFF" : "#E6000000"));
        _muted = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#A8FFFFFF" : "#8F000000"));
        _warning = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#FFF6B73C" : "#FFD97706"));
        _blockedProgress = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#FF737A86" : "#FF9CA3AF"));
        var track = new SolidColorBrush((System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(dark ? "#24FFFFFF" : "#12000000"));
        FiveHourLabel.Foreground = _muted;
        FiveHourUnavailableRun.Foreground = _warning;
        FiveHourPercentText.Foreground = _foreground;
        FiveHourResetText.Foreground = _weeklyExhausted ? _warning : _muted;
        WeeklyText.Foreground = _muted;
        WeeklyPercentText.Foreground = _foreground;
        WeeklyResetText.Foreground = _muted;
        StatusText.Foreground = _muted;
        Divider.Background = track;
        FiveHourProgress.Background = track;
        SetProgressBrush(FiveHourProgress.Value, _weeklyExhausted);
        UpdateCreditText();
        CardShadow.Color = dark ? Colors.Black : Colors.Gray;
    }

    private void UpdateCreditText()
    {
        CreditCountRun.Text = _creditCount?.ToString() ?? "--";
        CreditCountRun.Foreground = _creditCount is > 0 ? _foreground : _muted;
        CreditCountRun.FontWeight = _creditCount is > 0 ? FontWeights.SemiBold : FontWeights.Normal;
        CreditSuffixRun.Foreground = _muted;
    }

    private void SetProgressBrush(double? remaining, bool blocked)
    {
        if (blocked)
        {
            FiveHourProgress.Foreground = _blockedProgress;
            return;
        }

        var colors = QuotaVisuals.ProgressBand(remaining ?? 0) switch
        {
            QuotaProgressBand.Orange => ("#FF8A3D", "#FF5F45"),
            QuotaProgressBand.Yellow => ("#F6C344", "#F2A93B"),
            _ => ("#34C759", "#22B573"),
        };
        FiveHourProgress.Foreground = new LinearGradientBrush(
            (System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(colors.Item1),
            (System.Windows.Media.Color)System.Windows.Media.ColorConverter.ConvertFromString(colors.Item2),
            0);
    }

    private static string FormatReset(QuotaWindow? window) => window?.ResetsAt is { } reset
        ? $"将于 {reset.ToLocalTime():HH:mm} 重置"
        : "重置时间未知";

    private static string FormatPercent(double? remaining) =>
        remaining is null ? "--%" : $"{Math.Round(remaining.Value):0}%";
}
