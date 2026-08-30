namespace NeriPlayer.Core.Player.Lyrics;

/// <summary>LRC 时间戳行（对标 start.md 10.5）。</summary>
public sealed record LyricLine(TimeSpan Offset, string Text);
