using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace NeriPlayer.Data.Migrations
{
    /// <inheritdoc />
    public partial class AddDownloadTables : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "download_queue",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    StableKey = table.Column<string>(type: "TEXT", nullable: false),
                    SongName = table.Column<string>(type: "TEXT", nullable: false),
                    QualityKey = table.Column<string>(type: "TEXT", nullable: true),
                    TargetPath = table.Column<string>(type: "TEXT", nullable: false),
                    Priority = table.Column<int>(type: "INTEGER", nullable: false),
                    Status = table.Column<int>(type: "INTEGER", nullable: false),
                    EnqueuedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_download_queue", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "download_recovery",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    DownloadId = table.Column<long>(type: "INTEGER", nullable: false),
                    PartFilePath = table.Column<string>(type: "TEXT", nullable: false),
                    BytesReceived = table.Column<long>(type: "INTEGER", nullable: false),
                    RecoveryStatus = table.Column<int>(type: "INTEGER", nullable: false),
                    RecordedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_download_recovery", x => x.Id);
                });

            migrationBuilder.CreateTable(
                name: "download_snapshots",
                columns: table => new
                {
                    RootKey = table.Column<string>(type: "TEXT", nullable: false),
                    Bucket = table.Column<string>(type: "TEXT", nullable: false),
                    EntryKey = table.Column<string>(type: "TEXT", nullable: false),
                    LocalPath = table.Column<string>(type: "TEXT", nullable: false),
                    FileHash = table.Column<string>(type: "TEXT", nullable: true),
                    SnapshotAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_download_snapshots", x => new { x.RootKey, x.Bucket, x.EntryKey });
                });

            migrationBuilder.CreateTable(
                name: "downloads",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    StableKey = table.Column<string>(type: "TEXT", nullable: false),
                    LocalPath = table.Column<string>(type: "TEXT", nullable: false),
                    Status = table.Column<int>(type: "INTEGER", nullable: false),
                    QualityKey = table.Column<string>(type: "TEXT", nullable: true),
                    BytesReceived = table.Column<long>(type: "INTEGER", nullable: false),
                    TotalBytes = table.Column<long>(type: "INTEGER", nullable: true),
                    CreatedAt = table.Column<long>(type: "INTEGER", nullable: false),
                    UpdatedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_downloads", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_download_queue_StableKey",
                table: "download_queue",
                column: "StableKey");

            migrationBuilder.CreateIndex(
                name: "IX_download_recovery_DownloadId",
                table: "download_recovery",
                column: "DownloadId");

            migrationBuilder.CreateIndex(
                name: "IX_downloads_StableKey",
                table: "downloads",
                column: "StableKey",
                unique: true);
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "download_queue");

            migrationBuilder.DropTable(
                name: "download_recovery");

            migrationBuilder.DropTable(
                name: "download_snapshots");

            migrationBuilder.DropTable(
                name: "downloads");
        }
    }
}
