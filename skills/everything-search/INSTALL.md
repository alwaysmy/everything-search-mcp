# everything-search MCP 安装指南

运行方式是 **Everything 1.5 的插件 DLL**：它把 MCP 端点开在
`http://127.0.0.1:8286/mcp`——Everything 在跑，端点就在。

客户端只支持 stdio 的机器请用 `v2` 分支。

## 0. 一句话版本

装插件、重启 Everything、客户端里填一个 URL。

## 1. 装插件

前提：Everything **1.5**（1.4 不加载插件），管理员权限的 PowerShell。

```powershell
# 从仓库
.\install-plugin.ps1

# 从发布包（没有 Cargo.toml，脚本会自动用旁边那个 DLL）
.\install-plugin.ps1 -NoBuild
```

脚本做四件事：复制 DLL 到 `Plugins\`、写 `Everything.ini` 的加载列表、写
`Plugins.ini` 的 `enabled=1`、把文件名加进 HKLM 的 `AllowedPlugins`，然后重启 Everything 并验证。

> 白名单是**分号**分隔。逗号分隔会让整条白名单失效——这是"插件装了但不加载"最常见的原因。

## 2. 验证端点

```powershell
$body = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"probe","version":"1"}}}'
(Invoke-WebRequest 'http://127.0.0.1:8286/mcp' -Method Post -ContentType 'application/json' `
    -Headers @{Accept='application/json'} -Body $body -UseBasicParsing).Content
```

返回 `serverInfo` 即插件正常。连不上看 `%LOCALAPPDATA%\everything-search-mcp\plugin.log`。

## 3. 客户端配置（填 URL）

| 客户端 | 位置 | 形状 |
|---|---|---|
| DeepSeek Harness | `~/.dsh/profiles/<p>/cordis.patch.yml` | `transport: streamable-http` + `url:` |
| opencode | `~/.config/opencode/opencode.json` | `"type": "remote"` + `"url"` |
| Codex CLI | `~/.codex/config.toml` | `[mcp_servers.<name>]` + `url = "..."` |
| pi / omp | `~/.omp/agent/mcp.json` | `mcpServers.<name>.url` |
| Kimi Code | `~/.kimi-code/mcp.json` | `mcpServers.<name>.url` |

```json
{ "mcpServers": { "everything-search": { "url": "http://127.0.0.1:8286/mcp" } } }
```

两个坑：Kimi Code 读的是 `mcp.json`，写进 `config.toml` 会被静默忽略；opencode 的
`mcp add --url` 会**追加**嵌套对象而不是替换，容易出现同名的两个条目。

> **只支持 stdio 的客户端接不上**：ZCode 的 `mcp.servers` 没有 HTTP 形状，老版
> Claude Desktop 也只认 `command`。这些请用 v2 分支的 exe。

## 4. 没有 MCP 时的兜底

直接问 Everything 自己的 HTTP 接口（默认 23333），不需要插件也不经过 MCP：

```powershell
$q = 'path:"D:\Projects" *.md'
$u = "http://127.0.0.1:23333/?search=" + [uri]::EscapeDataString($q) + "&json=1&count=20&path_column=1"
$r = Invoke-RestMethod $u
$r.totalResults
$r.results | ForEach-Object { Join-Path $_.path $_.name }
```

细节（`path=` 不过滤、`path_column` 的作用）见 `REFERENCE.md`「没有 MCP 时的兜底」。

## 5. 排错

| 现象 | 原因 |
|---|---|
| 8286 连不上 | Everything 没重启；或插件没进 `AllowedPlugins`（分隔符用了逗号） |
| 工具列表是 0 个 | 客户端连的是旧地址；确认填的是 `/mcp` 结尾的 URL |
| 搜什么都返回 0 | Everything 的 HTTP 服务没开（23333），或 `path` 不存在——先看 `warnings` |
| 端口冲突 | 8286 被占用，插件日志里会写明 bind 失败 |

端口不是 23333 时，安装脚本会把地址写进 `%APPDATA%\everything-search-mcp\config.json`，
服务端启动时按 环境变量 → 该文件 → 默认值 的顺序取值。
