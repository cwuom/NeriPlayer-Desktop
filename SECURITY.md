[English](#english) | [中文](#中文)

# 安全策略 / Security Policy

## 中文

> [!CAUTION]
> 桌面端仍处于 🚧 **开发中** 阶段，安全相关实现可能不完善；维护者精力有限，响应可能延迟。

### 报告安全问题

请**不要**在公开 Issue、Discussions 或 PR 中披露安全问题。

- 通过 [私密安全报告](https://github.com/cwuom/NeriPlayer-Desktop/security/advisories/new) 提交
- 如果该页面提示仓库未开启私密报告，请新建一个标题为「请求私下报告安全问题」的 Issue，正文**不写任何细节**；维护者会回复私下沟通方式
- 请说明影响范围、复现步骤与受影响的版本（设置 → 关于中的构建版本）
- 附件中请移除真实的 Cookie、Token、密码与个人信息，使用测试账号或打码数据

### 适用范围

- 凭据存储与泄露（平台 Cookie、GitHub Token、WebDAV 密码）
- 日志、调试报告或崩溃报告中的敏感信息
- 云同步与一起听链路中的鉴权、直链暴露与数据完整性
- 本地文件写入、路径处理与更新流程

第三方音乐平台自身的问题请联系对应平台；Android 端问题请提交到
[主仓库](https://github.com/cwuom/NeriPlayer)。

### 支持版本

仅最新的 Release 与 main 分支 CI 构建接受安全修复。

## English

> [!CAUTION]
> The desktop app is 🚧 **work in progress**; security-related code may be incomplete, and maintainer response may be slow.

### Reporting a vulnerability

Please **do not** disclose security issues in public issues, discussions, or pull requests.

- Submit a [private security advisory](https://github.com/cwuom/NeriPlayer-Desktop/security/advisories/new)
- If that page says private reporting is not enabled, open an issue titled "Request to report a security issue privately" **without any details**; a maintainer will reply with a private channel
- Describe the impact, reproduction steps, and affected build (Settings → About)
- Remove real cookies, tokens, passwords, and personal data from attachments; use test accounts or redacted data

### Scope

- Credential storage and leaks (platform cookies, GitHub token, WebDAV password)
- Sensitive data in logs, debug reports, or crash reports
- Authentication, direct-link exposure, and data integrity in cloud sync and Listen Together
- Local file writes, path handling, and update flows

Issues in third-party music platforms belong to those platforms; Android app issues belong in the
[main repository](https://github.com/cwuom/NeriPlayer).

### Supported versions

Only the latest Release and CI builds from the main branch receive security fixes.
