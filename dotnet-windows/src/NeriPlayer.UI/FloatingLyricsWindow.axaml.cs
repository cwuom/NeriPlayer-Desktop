using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Threading;

namespace NeriPlayer.UI;

/// <summary>
/// 桌面歌词窗口（对标 Analysis.md 6.3 / Process.md 12.3 / start.md 11.3）。
/// 置顶透明无边框窗口，可拖动，随歌词滚动。
/// </summary>
public partial class FloatingLyricsWindow : Window
{
    private Point _dragStartPoint;
    private bool _isDragging;

    public FloatingLyricsWindow()
    {
        InitializeComponent();
        DragArea.PointerPressed += OnPointerPressed;
        DragArea.PointerMoved += OnPointerMoved;
        DragArea.PointerReleased += OnPointerReleased;
    }

    /// <summary>更新歌词行（从外部调用，如 PlaybackService/歌词滚动订阅）。</summary>
    public void UpdateLyrics(string current, string? translated = null)
    {
        Dispatcher.UIThread.Post(() =>
        {
            CurrentLine.Text = current;
            TranslatedLine.Text = translated;
            TranslatedLine.IsVisible = !string.IsNullOrEmpty(translated);
        });
    }

    /// <summary>更新歌词高亮位置（平滑滚动）。</summary>
    public void ScrollToLine(int lineIndex)
    {
        // 后续可接入 ScrollViewer 实现平滑滚动
    }

    private void OnPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            _isDragging = true;
            _dragStartPoint = e.GetPosition(this);
            e.Handled = true;
        }
    }

    private void OnPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_isDragging) return;
        var current = e.GetPosition(this);
        var delta = current - _dragStartPoint;
        Position = new PixelPoint(
            Position.X + (int)delta.X,
            Position.Y + (int)delta.Y);
        e.Handled = true;
    }

    private void OnPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        _isDragging = false;
        e.Handled = true;
    }
}
