using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace NeriPlayer.Data.Migrations
{
    /// <inheritdoc />
    public partial class InitialCreate : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "playback_stats",
                columns: table => new
                {
                    SongId = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    PlayCount = table.Column<long>(type: "INTEGER", nullable: false),
                    TotalPlayMs = table.Column<long>(type: "INTEGER", nullable: false),
                    LastPlayedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_playback_stats", x => x.SongId);
                });

            migrationBuilder.CreateTable(
                name: "playlists",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    Name = table.Column<string>(type: "TEXT", nullable: false),
                    Kind = table.Column<string>(type: "TEXT", nullable: false),
                    RemotePlatform = table.Column<string>(type: "TEXT", nullable: true),
                    RemoteId = table.Column<string>(type: "TEXT", nullable: true),
                    CreatedAt = table.Column<long>(type: "INTEGER", nullable: false),
                    UpdatedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_playlists", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "songs",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    StableKey = table.Column<string>(type: "TEXT", nullable: false),
                    Name = table.Column<string>(type: "TEXT", nullable: false),
                    Artist = table.Column<string>(type: "TEXT", nullable: false),
                    Album = table.Column<string>(type: "TEXT", nullable: false),
                    AlbumId = table.Column<long>(type: "INTEGER", nullable: false),
                    DurationMs = table.Column<long>(type: "INTEGER", nullable: false),
                    CoverUrl = table.Column<string>(type: "TEXT", nullable: true),
                    MediaUri = table.Column<string>(type: "TEXT", nullable: true),
                    StreamUrl = table.Column<string>(type: "TEXT", nullable: true),
                    ChannelId = table.Column<string>(type: "TEXT", nullable: true),
                    AudioId = table.Column<string>(type: "TEXT", nullable: true),
                    SubAudioId = table.Column<string>(type: "TEXT", nullable: true),
                    MatchedLyric = table.Column<string>(type: "TEXT", nullable: true),
                    MatchedTranslatedLyric = table.Column<string>(type: "TEXT", nullable: true),
                    MatchedLyricSource = table.Column<string>(type: "TEXT", nullable: true),
                    UserLyricOffsetMs = table.Column<long>(type: "INTEGER", nullable: false),
                    CustomName = table.Column<string>(type: "TEXT", nullable: true),
                    CustomArtist = table.Column<string>(type: "TEXT", nullable: true),
                    CustomCoverUrl = table.Column<string>(type: "TEXT", nullable: true),
                    OriginalName = table.Column<string>(type: "TEXT", nullable: true),
                    OriginalArtist = table.Column<string>(type: "TEXT", nullable: true),
                    LocalFileName = table.Column<string>(type: "TEXT", nullable: true),
                    LocalFilePath = table.Column<string>(type: "TEXT", nullable: true),
                    AddedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_songs", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "stat_buckets",
                columns: table => new
                {
                    SongId = table.Column<long>(type: "INTEGER", nullable: false),
                    DayKey = table.Column<long>(type: "INTEGER", nullable: false),
                    PlayCount = table.Column<long>(type: "INTEGER", nullable: false),
                    ListenMs = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_stat_buckets", x => new { x.SongId, x.DayKey });
                });

            migrationBuilder.CreateTable(
                name: "playlist_members",
                columns: table => new
                {
                    PlaylistId = table.Column<long>(type: "INTEGER", nullable: false),
                    Position = table.Column<int>(type: "INTEGER", nullable: false),
                    SongId = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_playlist_members", x => new { x.PlaylistId, x.Position });
                    table.ForeignKey(
                        name: "FK_playlist_members_playlists_PlaylistId",
                        column: x => x.PlaylistId,
                        principalTable: "playlists",
                        principalColumn: "Id",
                        onDelete: ReferentialAction.Cascade);
                    table.ForeignKey(
                        name: "FK_playlist_members_songs_SongId",
                        column: x => x.SongId,
                        principalTable: "songs",
                        principalColumn: "Id",
                        onDelete: ReferentialAction.Cascade);
                });

            migrationBuilder.CreateIndex(
                name: "IX_playlist_members_SongId",
                table: "playlist_members",
                column: "SongId");

            migrationBuilder.CreateIndex(
                name: "IX_songs_StableKey",
                table: "songs",
                column: "StableKey",
                unique: true);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "playback_stats");

            migrationBuilder.DropTable(
                name: "playlist_members");

            migrationBuilder.DropTable(
                name: "stat_buckets");

            migrationBuilder.DropTable(
                name: "playlists");

            migrationBuilder.DropTable(
                name: "songs");
        }
    }
}
