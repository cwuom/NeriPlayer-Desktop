using Microsoft.EntityFrameworkCore.Migrations;

#nullable disable

namespace NeriPlayer.Data.Migrations
{
    /// <inheritdoc />
    public partial class AddSyncTables : Migration
    {
        /// <inheritdoc />
        protected override void Up(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.CreateTable(
                name: "sync_checkpoints",
                columns: table => new
                {
                    Scope = table.Column<string>(type: "TEXT", nullable: false),
                    Token = table.Column<string>(type: "TEXT", nullable: true),
                    UpdatedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_sync_checkpoints", x => x.Scope);
                });

            migrationBuilder.CreateTable(
                name: "sync_metadata",
                columns: table => new
                {
                    Key = table.Column<string>(type: "TEXT", nullable: false),
                    Etag = table.Column<string>(type: "TEXT", nullable: true),
                    Revision = table.Column<string>(type: "TEXT", nullable: true),
                    UpdatedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_sync_metadata", x => x.Key);
                });

            migrationBuilder.CreateTable(
                name: "sync_outbox",
                columns: table => new
                {
                    Id = table.Column<long>(type: "INTEGER", nullable: false)
                        .Annotation("Sqlite:Autoincrement", true),
                    RefKey = table.Column<string>(type: "TEXT", nullable: false),
                    Action = table.Column<string>(type: "TEXT", nullable: false),
                    PayloadJson = table.Column<string>(type: "TEXT", nullable: true),
                    CreatedAt = table.Column<long>(type: "INTEGER", nullable: false)
                },
                constraints: table =>
                {
                    table.PrimaryKey("PK_sync_outbox", x => x.Id);
                });

            migrationBuilder.CreateIndex(
                name: "IX_sync_outbox_RefKey",
                table: "sync_outbox",
                column: "RefKey");
        }

        /// <inheritdoc />
        protected override void Down(MigrationBuilder migrationBuilder)
        {
            migrationBuilder.DropTable(
                name: "sync_checkpoints");

            migrationBuilder.DropTable(
                name: "sync_metadata");

            migrationBuilder.DropTable(
                name: "sync_outbox");
        }
    }
}
