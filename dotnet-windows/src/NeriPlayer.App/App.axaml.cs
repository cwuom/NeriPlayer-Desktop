using System.Runtime.Versioning;
using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Microsoft.Extensions.DependencyInjection;

namespace NeriPlayer.App;

[SupportedOSPlatform("windows")]
public partial class App : Application
{
    public override void Initialize()
    {
        AvaloniaXamlLoader.Load(this);
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            var sp = AppStartup.BuildServices();
            var vm = sp.GetRequiredService<NeriPlayer.UI.ViewModels.MainWindowViewModel>();
            desktop.MainWindow = new NeriPlayer.UI.Views.MainWindow
            {
                DataContext = vm
            };
        }

        base.OnFrameworkInitializationCompleted();
    }
}