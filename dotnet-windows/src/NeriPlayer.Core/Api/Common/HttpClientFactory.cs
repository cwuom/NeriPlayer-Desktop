using System.Net;
using System.Net.Http.Headers;

namespace NeriPlayer.Core.Api.Common;

/// <summary>
/// 共享 HttpClient 工厂（对标 Analysis.md 22.1 buildSharedOkHttpClient）。
/// 单例 HttpClient，支持 Cookie、Brotli/GZIP、可选代理。
/// </summary>
public sealed class HttpClientFactory : IDisposable
{
    private readonly HttpClient _http;

    /// <summary>平台共享 HttpClient（线程安全单例）</summary>
    public HttpClient Http => _http;

    /// <summary>可注入 CookieContainer（三平台登录态）</summary>
    public CookieContainer CookieContainer { get; }

    public HttpClientFactory(CookieContainer? cookies = null, WebProxy? proxy = null)
    {
        CookieContainer = cookies ?? new CookieContainer();

        var handler = new HttpClientHandler
        {
            CookieContainer = CookieContainer,
            UseCookies = true,
            AutomaticDecompression = DecompressionMethods.Brotli | DecompressionMethods.GZip | DecompressionMethods.Deflate,
        };

        // 可选代理（对标 Analysis.md 22.2 DynamicProxySelector）
        if (proxy is not null) handler.Proxy = proxy;

        _http = new HttpClient(handler)
        {
            // 禁用总时限，防止截断长播放/同步请求（对标 Analysis.md 22.1 callTimeout=0）
            Timeout = System.Threading.Timeout.InfiniteTimeSpan,
        };

        // 共用 UA
        _http.DefaultRequestHeaders.UserAgent.ParseAdd(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
        _http.DefaultRequestHeaders.AcceptLanguage.ParseAdd("zh-CN,zh;q=0.9,en;q=0.8");
    }

    public HttpClientFactory() : this(null, null) { }

    public void Dispose() => _http.Dispose();
}