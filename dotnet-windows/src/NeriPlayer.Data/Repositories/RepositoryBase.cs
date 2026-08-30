namespace NeriPlayer.Data.Repositories;

using Microsoft.EntityFrameworkCore;

/// <summary>
/// 仓储基类：事务包装器（对标 start.md 4.4 / Process.md 5.4）。
/// 所有写操作通过 RunAsync 保证事务性（三段式迁移策略配套）。
/// </summary>
public abstract class RepositoryBase<TDbContext>(TDbContext db) where TDbContext : DbContext
{
    protected TDbContext Db { get; } = db;

    /// <summary>在显式事务内执行操作并提交</summary>
    protected async Task<T> RunAsync<T>(Func<TDbContext, Task<T>> action)
    {
        await using var t = await Db.Database.BeginTransactionAsync();
        var r = await action(Db);
        await t.CommitAsync();
        return r;
    }
}