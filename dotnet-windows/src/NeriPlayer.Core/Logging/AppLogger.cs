using Serilog;

namespace NeriPlayer.Core.Logging;

public static class AppLogger
{
    public static readonly Serilog.Core.Logger Instance = new LoggerConfiguration()
        .MinimumLevel.Information()
        .WriteTo.Console(outputTemplate:
            "[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}")
        .WriteTo.File(
            path: Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "NeriPlayer", "logs", "app-.log"),
            rollingInterval: RollingInterval.Day,
            retainedFileCountLimit: 14,
            outputTemplate: "[{Timestamp:yyyy-MM-dd HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}")
        .CreateLogger();

    public static void Flush() => Instance.Dispose();
}
