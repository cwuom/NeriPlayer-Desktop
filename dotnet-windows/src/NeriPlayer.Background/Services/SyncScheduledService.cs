using Microsoft.Extensions.Hosting;
using Quartz;
using Quartz.Impl;
using NeriPlayer.Data.Sync;

namespace NeriPlayer.Background.Services;

/// <summary>
/// 定时同步服务（对标 start.md 9.5 / Process.md 12.6 WorkManager）。
/// 注入 SyncCoordinator，Quartz 每日 02:00 触发 + 支持手动触发。
/// </summary>
public sealed class SyncScheduledService(SyncCoordinator coordinator) : BackgroundService
{
    /// <summary>手动触发一次同步（供 UI / 测试调用）</summary>
    public Task<SyncResult> TriggerNowAsync(string scope) => coordinator.SyncAsync(scope);

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var factory = new StdSchedulerFactory();
        var scheduler = await factory.GetScheduler(stoppingToken);
        await scheduler.Start(stoppingToken);

        var job = JobBuilder.Create<SyncJob>()
            .UsingJobData("scope", "playlists")
            .Build();
        var trigger = TriggerBuilder.Create()
            .WithIdentity("daily-sync", "sync")
            .WithCronSchedule("0 0 2 * * ?")          // 每日 02:00
            .Build();
        await scheduler.ScheduleJob(job, trigger, stoppingToken);
    }
}

/// <summary>Quartz 同步任务（对标 start.md 9.5 SyncJob）
/// 通过 JobDataMap 传递 scope，实际协调逻辑由宿主注入的 SyncCoordinator 执行。</summary>
public sealed class SyncJob : IJob
{
    private static SyncCoordinator? _coordinator;

    /// <summary>由 DI 宿主在启动时注入（Quartz IJob 生命周期与 DI 解耦）</summary>
    public static void SetCoordinator(SyncCoordinator coordinator) => _coordinator = coordinator;

    public async Task Execute(IJobExecutionContext context)
    {
        var scope = context.MergedJobDataMap.GetString("scope") ?? "playlists";
        if (_coordinator is null) return;
        var result = await _coordinator.SyncAsync(scope);
        // 日志输出由调用方托管，此处仅执行
    }
}
