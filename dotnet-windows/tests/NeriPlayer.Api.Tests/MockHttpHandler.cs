using System.Net;
using System.Text;
using Xunit;

namespace NeriPlayer.Api.Tests;

/// <summary>可注入的 Mock HttpMessageHandler（用于测试 API 客户端）</summary>
public sealed class MockHttpHandler : HttpMessageHandler
{
    private readonly Dictionary<string, (string body, HttpStatusCode code)> _responses = new();
    public List<HttpRequestMessage> Requests { get; } = [];

    public void Register(string containsUrl, string body, HttpStatusCode code = HttpStatusCode.OK)
        => _responses[containsUrl] = (body, code);

    protected override Task<HttpResponseMessage> SendAsync(
        HttpRequestMessage request, CancellationToken ct)
    {
        Requests.Add(request);
        var url = request.RequestUri?.ToString() ?? "";
        foreach (var (key, (body, code)) in _responses)
        {
            if (url.Contains(key, StringComparison.OrdinalIgnoreCase))
                return Task.FromResult(new HttpResponseMessage(code)
                {
                    Content = new StringContent(body, Encoding.UTF8, "application/json"),
                });
        }
        return Task.FromResult(new HttpResponseMessage(HttpStatusCode.NotFound)
        { Content = new StringContent("") });
    }
}