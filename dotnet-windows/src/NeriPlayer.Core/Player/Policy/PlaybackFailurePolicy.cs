namespace NeriPlayer.Core.Player.Policy;

/// <summary>
/// 连续失败计数与停止阈值（对标 Analysis.md 3.2 / Process.md 4.5）
/// 阈值：MAX_CONSECUTIVE_FAILURES = 10
/// </summary>
public sealed class PlaybackFailurePolicy
{
    private const int MaxConsecutiveFailures = 10;
    private int _count;

    /// <summary>
    /// 记录一次失败，返回是否已达到停止阈值。
    /// </summary>
    /// <param name="shouldStop">true 表示应停止尝试，进入 Error 状态</param>
    /// <returns>与 <paramref name="shouldStop"/> 相同</returns>
    public bool RecordFailure(out bool shouldStop)
    {
        _count++;
        shouldStop = _count >= MaxConsecutiveFailures;
        return shouldStop;
    }

    /// <summary>播放成功后重置计数</summary>
    public void RecordSuccess() => _count = 0;

    /// <summary>当前连续失败次数（测试用）</summary>
    public int FailureCount => _count;
}
