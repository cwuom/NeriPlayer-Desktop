using NeriPlayer.Core.Player.Lyrics;
using Xunit;

namespace NeriPlayer.Core.Tests;

public class LrcParserTests
{
    [Fact]
    public void Parse_Empty_ReturnsEmpty()
    {
        Assert.Empty(LrcParser.Parse(null));
        Assert.Empty(LrcParser.Parse(""));
        Assert.Empty(LrcParser.Parse("   \n  "));
    }

    [Fact]
    public void Parse_SingleLine_ParsesCorrectly()
    {
        var result = LrcParser.Parse("[01:23.45]Hello World");
        Assert.Single(result);
        Assert.Equal("Hello World", result[0].Text);
        Assert.Equal(TimeSpan.FromSeconds(83.45), result[0].Offset);
    }

    [Fact]
    public void Parse_MultipleSorted_ReturnsInOrder()
    {
        var result = LrcParser.Parse("[02:00.00]B\n[01:00.00]A\n[03:00.00]C");
        Assert.Equal(3, result.Count);
        Assert.Equal("A", result[0].Text);
        Assert.Equal("B", result[1].Text);
        Assert.Equal("C", result[2].Text);
    }

    [Fact]
    public void Parse_OffsetTag_AdjustsTimestamps()
    {
        var result = LrcParser.Parse("[offset:+500]\n[01:00.00]Line");
        Assert.Single(result);
        Assert.Equal(TimeSpan.FromSeconds(60) + TimeSpan.FromMilliseconds(500), result[0].Offset);
    }

    [Fact]
    public void Parse_ShortFormat_mmss_ParsesCorrectly()
    {
        var result = LrcParser.Parse("[01:30]Short format");
        Assert.Single(result);
        Assert.Equal(TimeSpan.FromSeconds(90), result[0].Offset);
    }

    [Fact]
    public void Parse_InvalidLine_IgnoredGracefully()
    {
        var result = LrcParser.Parse("no timestamp here\n[bad]\n[01:00.00]Good");
        Assert.Single(result);
        Assert.Equal("Good", result[0].Text);
    }

    [Fact]
    public void FindIndexAt_MiddleOfTwo_ReturnsFirst()
    {
        var lines = LrcParser.Parse("[00:10.00]A\n[00:20.00]B\n[00:30.00]C");
        Assert.Equal(0, LrcParser.FindIndexAt(lines, TimeSpan.FromSeconds(15)));
        Assert.Equal(1, LrcParser.FindIndexAt(lines, TimeSpan.FromSeconds(25)));
    }

    [Fact]
    public void FindIndexAt_BeforeFirst_ReturnsZero()
    {
        var lines = LrcParser.Parse("[00:10.00]A\n[00:20.00]B");
        Assert.Equal(0, LrcParser.FindIndexAt(lines, TimeSpan.FromSeconds(5)));
    }
}
