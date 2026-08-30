using System.Runtime.Versioning;
using Microsoft.Extensions.DependencyInjection;
using NeriPlayer.Core.Api.Common;

namespace NeriPlayer.App;

[SupportedOSPlatform("windows")]
public static class AppStartup
{
    public static ServiceProvider BuildServices()
    {
        var services = new ServiceCollection();

        // 数据层
        services.AddDbContext<Data.Database.NeriDbContext>();

        // 核心层：播放器（在线歌曲 URL 保鲜，注入三平台解析委托，A1）
        services.AddSingleton<Core.Player.PlayerManager>(sp =>
        {
            var netease = sp.GetRequiredService<Core.Api.Netease.NeteaseClient>();
            var bili = sp.GetRequiredService<Core.Api.Bili.BiliClient>();
            var ytm = sp.GetRequiredService<Core.Api.YouTube.YouTubeMusicClient>();
            return new Core.Player.PlayerManager(
                new Core.Player.Engine.VlcPlaybackEngine(),
                async song => song.ChannelId switch
                {
                    "netease" => (await netease.ResolveSongUrlAsync(song, "standard")) is { Success: true } nr ? nr.Url : null,
                    "bili" => (await bili.ResolveSongUrlAsync(song)) is { Success: true } br ? br.Url : null,
                    "youtube_music" => (await ytm.ResolveSongUrlAsync(song)) is { Success: true } yr ? yr.Url : null,
                    _ => null,
                });
        });

        // 下载管理（第八章）
        services.AddSingleton<Core.Download.DownloadQueue>(sp =>
        {
            var factory = sp.GetRequiredService<HttpClientFactory>();
            return new Core.Download.DownloadQueue(factory.Http);
        });
        services.AddScoped<Data.Repositories.DownloadRepository>();

        // API 客户端（第七章）
        services.AddSingleton<HttpClientFactory>();
        services.AddSingleton<Core.Api.Netease.NeteaseClient>();
        services.AddSingleton<Core.Api.Bili.BiliClient>();
        services.AddSingleton<Core.Api.YouTube.YouTubePlayerScriptStore>();
        services.AddSingleton<Core.Api.YouTube.YouTubeMusicClient>(sp =>
            new Core.Api.YouTube.YouTubeMusicClient(
                sp.GetRequiredService<HttpClientFactory>(),
                sp.GetRequiredService<Core.Api.YouTube.YouTubePlayerScriptStore>()));

        // 第九章：数据同步
        services.AddScoped<Data.Repositories.SyncRepository>();
        services.AddScoped<Data.Repositories.SongRepository>();
        services.AddSingleton<Data.Auth.CredentialStore>();
        services.AddSingleton<Data.Sync.GitHubSyncProvider>(sp =>
            new Data.Sync.GitHubSyncProvider(
                token: "your-github-token",
                owner: "your-github-owner",
                repo: "neriplayer-sync"));
        services.AddSingleton<Data.Sync.WebDavSyncProvider>(sp =>
            new Data.Sync.WebDavSyncProvider(
                server: new Uri("https://dav.example.com"),
                user: "user",
                password: "password"));
        services.AddScoped<Data.Sync.SyncCoordinator>(sp =>
            new Data.Sync.SyncCoordinator(
                sp.GetRequiredService<Data.Sync.GitHubSyncProvider>(),
                sp.GetRequiredService<Data.Repositories.SongRepository>(),
                sp.GetRequiredService<Data.Repositories.SyncRepository>()));

        // 第十一章：后台与系统集成
        // SMTC（Singleton：全局一个实例，持 Windows.Media.MediaPlayer 用于系统媒体控制）
        services.AddSingleton<Background.Services.SmtcIntegration>();
        // 后台播放服务（托管：窗口关闭后保持播放，接 SMTC 按钮）
        services.AddHostedService<Background.Services.PlaybackService>();
        // 定时同步服务（第 9 章已有，启用）
        services.AddHostedService<Background.Services.SyncScheduledService>();

        // UI 层（第十章）
        services.AddSingleton<UI.ViewModels.MainWindowViewModel>();

        return services.BuildServiceProvider();
    }
}
