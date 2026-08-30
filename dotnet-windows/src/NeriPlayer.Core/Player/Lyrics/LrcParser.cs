using System.Globalization;

namespace NeriPlayer.Core.Player.Lyrics;

/// <summary>
/// LRC 歌词解析器（对标 start.md 10.5）。
/// 支持 [mm:ss.xx] / [mm:ss.xxx] / [mm:ss] 偏移、多语言 [offset:]、空行容错。
/// 损坏输入返回空列表。
/// </summary>
public static class LrcParser
{
    public static IReadOnlyList<LyricLine> Parse(string? raw)
    {
        if (string.IsNullOrWhiteSpace(raw)) return [];

        var lines = new List<LyricLine>();
        var offsetMs = 0;

        foreach (var line in SplitLines(raw))
        {
            var trimmed = line.Trim();
            if (trimmed.Length == 0) continue;

            // [offset:+/-ms]
            if (trimmed.StartsWith("[offset:", StringComparison.OrdinalIgnoreCase) && trimmed.EndsWith(']'))
            {
                var inner = trimmed[8..^1];
                if (int.TryParse(inner, NumberStyles.Integer | NumberStyles.AllowLeadingSign,
                        CultureInfo.InvariantCulture, out var off))
                    offsetMs = off;
                continue;
            }

            // [ti:xxx] / [ar:xxx] 等信息行跳过（无后续内容）
            if (TryParseTags(trimmed, out var tags, out var content))
            {
                foreach (var tag in tags)
                {
                    if (TryParseTimestamp(tag, out var ts))
                    {
                        var adjusted = ts + TimeSpan.FromMilliseconds(offsetMs);
                        lines.Add(new LyricLine(adjusted, content));
                    }
                }
            }
        }

        lines.Sort((a, b) => a.Offset.CompareTo(b.Offset));
        return lines;
    }

    /// <summary>
    /// 二分查找当前进度对应的行索引（对标 start.md 10.4）。
    /// 返回最后一个小于等于 position 的行索引，无匹配返回 -1。
    /// </summary>
    public static int FindIndexAt(IReadOnlyList<LyricLine> lines, TimeSpan position)
    {
        if (lines.Count == 0) return -1;
        int lo = 0, hi = lines.Count - 1;
        while (lo <= hi)
        {
            var mid = (lo + hi) / 2;
            if (lines[mid].Offset <= position) lo = mid + 1;
            else hi = mid - 1;
        }
        return Math.Max(0, lo - 1);
    }

    // ── 内部辅助 ─────────────────────────────────────────────────────────

    private static IEnumerable<string> SplitLines(string s)
    {
        using var reader = new StringReader(s);
        while (reader.ReadLine() is { } line)
            yield return line;
    }

    /// <summary>
    /// 解析 [tag1][tag2]...content 形式，tags 返回时间戳标签列表，content 返回正文。
    /// </summary>
    private static bool TryParseTags(string line, out List<string> tags, out string content)
    {
        tags = [];
        var i = 0;
        while (i < line.Length && line[i] == '[')
        {
            var close = line.IndexOf(']', i + 1);
            if (close < 0) { content = line; return false; }
            tags.Add(line[(i + 1)..close]);
            i = close + 1;
        }
        content = line[i..].Trim();
        return tags.Count > 0;
    }

    private static bool TryParseTimestamp(string tag, out TimeSpan ts)
    {
        ts = TimeSpan.Zero;
        var parts = tag.Split(':');
        if (parts.Length != 2) return false;
        if (!int.TryParse(parts[0], NumberStyles.Integer, CultureInfo.InvariantCulture, out var min)) return false;
        // 支持 ss.xx / ss.xxx
        if (!double.TryParse(parts[1], NumberStyles.Float, CultureInfo.InvariantCulture, out var sec)) return false;
        ts = TimeSpan.FromMinutes(min) + TimeSpan.FromSeconds(sec);
        return true;
    }
}
