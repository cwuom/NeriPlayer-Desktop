using System.Globalization;
using Avalonia.Data.Converters;

namespace NeriPlayer.UI.Converters;

/// <summary>Bool → 播放/暂停图标文字（编译绑定友好）。</summary>
public sealed class BoolToPlayIconConverter : IValueConverter
{
    public object Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
        => value is true ? "⏸" : "▶";

    public object ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        => throw new NotSupportedException();
}

/// <summary>bool → Brush（歌词高亮用）：true → AppActiveBrush，false → Transparent。</summary>
public sealed class ActiveBrushConverter : IValueConverter
{
    public object Convert(object? value, Type targetType, object? parameter, CultureInfo culture)
        => value is true
            ? Avalonia.Media.Brushes.CornflowerBlue
            : Avalonia.Media.Brushes.Transparent;

    public object ConvertBack(object? value, Type targetType, object? parameter, CultureInfo culture)
        => throw new NotSupportedException();
}
