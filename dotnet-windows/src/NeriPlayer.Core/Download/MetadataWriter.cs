namespace NeriPlayer.Core.Download;

/// <summary>
/// 歌曲元数据（对标 start.md 8.3 SongMetadata record）。
/// </summary>
public sealed record SongMetadata(
    string Title,
    string Artist,
    string Album,
    byte[]? CoverBytes);

/// <summary>
/// TagLib# 标签写入器（对标 start.md 8.3 / Analysis.md 24.2）。
/// 失败重试上限 3 次，指数退避（200ms * attempt）。
/// </summary>
public static class MetadataWriter
{
    /// <summary>最大重试次数（对标 Analysis.md 24.2 标签写入重试上限）</summary>
    public const int MaxAttempts = 3;

    /// <summary>写入音频文件标签</summary>
    public static async Task WriteAsync(string filePath, SongMetadata metadata)
    {
        for (var attempt = 1; attempt <= MaxAttempts; attempt++)
        {
            try
            {
                WriteCore(filePath, metadata);
                return;
            }
            catch (Exception ex) when (attempt < MaxAttempts)
            {
                // 指数退避：200ms, 400ms, ...
                await Task.Delay(200 * attempt);
                System.Diagnostics.Debug.WriteLine(
                    $"[MetadataWriter] 标签写入失败（尝试 {attempt}/{MaxAttempts}）: {ex.Message}");
            }
        }

        // 最后一次尝试，让异常自然抛出
        WriteCore(filePath, metadata);
    }

    private static void WriteCore(string filePath, SongMetadata metadata)
    {
        using var file = TagLib.File.Create(filePath);

        file.Tag.Title = metadata.Title;
        file.Tag.Performers = [metadata.Artist];
        file.Tag.Album = metadata.Album;

        if (metadata.CoverBytes is { Length: > 0 })
        {
            file.Tag.Pictures =
            [
                new TagLib.Picture(new TagLib.ByteVector(metadata.CoverBytes))
                {
                    MimeType = "image/jpeg",
                    Type = TagLib.PictureType.FrontCover,
                }
            ];
        }

        file.Save();
    }
}