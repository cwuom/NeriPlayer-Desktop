namespace NeriPlayer.Core.Player.Engine;

/// <summary>播放引擎异常</summary>
public sealed class EngineException(string message) : Exception(message);
