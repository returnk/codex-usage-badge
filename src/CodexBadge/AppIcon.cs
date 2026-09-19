using System.Drawing;
using System.Windows;

namespace CodexBadge;

internal static class AppIcon
{
    internal static Icon LoadDrawingIcon()
    {
        var resource = System.Windows.Application.GetResourceStream(
            new Uri("pack://application:,,,/Assets/CodexBadge.ico"));
        if (resource is null) return SystemIcons.Application;
        using var icon = new Icon(resource.Stream);
        return (Icon)icon.Clone();
    }
}
