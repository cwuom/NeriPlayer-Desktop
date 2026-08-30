namespace NeriPlayer.Data.Database;

using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Design;

/// <summary>
/// 设计时工厂：供 dotnet ef migrations 使用（对标 start.md 4.2）。
/// 数据库路径：%APPDATA%\NeriPlayer\neriplayer.db。
/// </summary>
public sealed class NeriDbContextFactory : IDesignTimeDbContextFactory<NeriDbContext>
{
    public NeriDbContext CreateDbContext(string[] args)
    {
        var dbPath = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "NeriPlayer", "neriplayer.db");
        Directory.CreateDirectory(Path.GetDirectoryName(dbPath)!);

        var options = new DbContextOptionsBuilder<NeriDbContext>()
            .UseSqlite($"Data Source={dbPath}")
            .Options;
        return new NeriDbContext(options);
    }
}