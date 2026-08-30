using ReactiveUI;
using System.Reactive;

namespace NeriPlayer.UI.ViewModels;

public sealed class MainWindowViewModel : ReactiveObject
{
    private object? _currentPage;
    private readonly Dictionary<string, object> _pages;

    public PlaybackBarViewModel PlaybackBar { get; }

    public object? CurrentPage
    {
        get => _currentPage;
        set => this.RaiseAndSetIfChanged(ref _currentPage, value);
    }

    public ReactiveCommand<string, Unit> NavigateToCommand { get; }

    public MainWindowViewModel(NeriPlayer.Core.Player.PlayerManager player)
    {
        PlaybackBar = new PlaybackBarViewModel(player);

        _pages = new Dictionary<string, object>
        {
            ["Home"]       = new HomeViewModel(),
            ["Discover"]   = new DiscoverViewModel(),
            ["Library"]    = new LibraryViewModel(),
            ["Downloads"]  = new DownloadsViewModel(),
            ["Settings"]   = new SettingsViewModel(),
        };

        _currentPage = _pages["Home"];

        NavigateToCommand = ReactiveCommand.Create<string>(name =>
        {
            if (_pages.TryGetValue(name, out var page))
                CurrentPage = page;
        });
    }
}
