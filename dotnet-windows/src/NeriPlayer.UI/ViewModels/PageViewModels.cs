using ReactiveUI;

namespace NeriPlayer.UI.ViewModels;

public sealed class HomeViewModel       : ReactiveObject { public string Title => "首页"; }
public sealed class DiscoverViewModel   : ReactiveObject { public string Title => "发现"; }
public sealed class LibraryViewModel    : ReactiveObject { public string Title => "音乐库"; }
public sealed class DownloadsViewModel  : ReactiveObject { public string Title => "下载"; }
public sealed class SettingsViewModel   : ReactiveObject { public string Title => "设置"; }
