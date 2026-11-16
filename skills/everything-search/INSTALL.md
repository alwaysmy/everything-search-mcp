# everything-search MCP 安装指南

仅在**首次安装 / 换机器 / 重新配置**时读本文件。日常调用 MCP 不需要。

## 0. 一句话版本

本版本不带配置生成器（CLI 已移除），配置就是往客户端里粘一段 JSON：

```json
{
  "mcpServers": {
    "everything-search": {
      "command": "C:\\Users\\<你>\\.agents\\skills\\everything-search\\bin\\everything-search-mcp.exe"
    }
  }
}
```

路径换成本机 exe 的**绝对路径**（skill 目录搬到哪，这里就改到哪）。
配好后客户端里应出现 5 个 `everything_search*` 工具；没出现就是配置位置不对，按下面各客户端的表格核对。

### exe 从哪来

本 skill 自带 `bin\everything-search-mcp.exe`（约 580 KB，自包含，无运行时依赖）。
如果这个文件不在（比如只拷了 `SKILL.md`），从 release 取：

```powershell
# 不带 tag 就是取 latest，这样升级时这行不会过期
gh release download --repo alwaysmy/everything-search-mcp `
  --pattern 'everything-search-mcp.exe' --dir '<本 skill 目录>\bin'
# 或者直接下整个 skill 包：--pattern 'everything-search-skill-*.zip'
# 钉版本：gh release download v<version> --repo ... 
```

或浏览器打开 <https://github.com/alwaysmy/everything-search-mcp/releases>。
放好后按第 2 节把它的绝对路径粘进客户端配置。

## 1. 前提条件

| 项 | 要求 |
|---|---|
| 操作系统 | Windows（Everything 只有 Windows 版） |
| voidtools Everything | 已安装并**正在运行**（托盘有图标） |
| Everything HTTP 服务器 | **必须启用**，端口 `23333` |

服务端**不调用 `es.exe`**，也不加载任何 SDK DLL，只走 Everything 内置的 HTTP 接口
（回环实测 0.7 ms，比 `es.exe` 子进程快约 85 倍）。所以：

- ❌ 不需要 `es.exe`
- ❌ 不需要 `EVERYTHING_ES_PATH`（这个变量已经废弃，写了也会被忽略）
- ❌ 不需要 Python / uv / pip
- ❌ 不需要 `Everything64.dll` / `Everything3_x64.dll`

### 打开 HTTP 服务器

Everything → **工具 → 选项 → HTTP 服务器** → 勾选「启用 HTTP 服务器」，端口填 `23333`。

> **安全**：默认绑定是 `0.0.0.0`，也就是同一局域网内任何人都能查询你整块索引，
> 还能下载文件（`allow_file_download=1`）。建议在 Everything 的
> `Plugins.ini` 里把它限制到本机：
>
> ```ini
> [http_server]
> bindings=127.0.0.1
> ```

## 2. 配置客户端

把这段粘进客户端的 MCP 配置文件（各客户端位置见第 3 节）：

```json
{
  "mcpServers": {
    "everything-search": {
      "command": "C:\\Users\\<你>\\.agents\\skills\\everything-search\\bin\\everything-search-mcp.exe"
    }
  }
}
```

- 路径必须是本机绝对路径；skill 目录搬家后改这一行就行。
- 名字随便起，只要不和已有条目不冲突。
- 配完重启客户端，工具列表里出现 5 个 `everything_search*` 即成功。

### 2.1 可配置的环境变量

客户端配置里**默认不用写 env**——默认值就是本机最常用的一组。要覆盖时加 `env`：

| 环境变量 | 默认 | 说明 |
|---|---|---|
| `EVERYTHING_HTTP_URL` | `http://127.0.0.1:23333` | Everything HTTP 地址；改过端口才需要 |
| `EVERYTHING_TIMEOUT` | `30` | 单次请求超时秒数 |
| `EVERYTHING_MAX_RESULTS_CAP` | `1000` | 单次返回条数硬上限（控制 token 用量） |
| `EVERYTHING_SERVERS_FILE` | `%APPDATA%\everything-search-mcp\servers.json` | 远程机器登记表的位置 |

```json
{ "mcpServers": { "everything-search": {
    "command": "C:\\...\\everything-search-mcp.exe",
    "env": { "EVERYTHING_TIMEOUT": "10" }
}}}
```

旧版的 `EVERYTHING_ES_PATH` / `EVERYTHING_INSTANCE` 已**不存在**，不要再用。

要搜**别的机器**，往 `servers.json` 里加一条（见 `REFERENCE.md`「搜别的机器」），
或在调用里直接传 `url` —— 不需要环境变量，也不用在那台机器上装东西。

## 3. 各客户端配置形态（手工参考）

下面是各客户端该长什么样，照抄即可。


### DeepSeek Harness — `%USERPROFILE%\.dsh\cordis.patch.yml`

顶层 loader patch 条目（注意是 `insert` 列表里的一项）：

```yaml
- insert:
    - id: mcp-everything
      name: '@deepseek-ai/dsh-mcp-client'
      config:
        serverName: everything
        transport: stdio
        command: C:/Users/<用户名>/.agents/skills/everything-search/bin/everything-search-mcp.exe
```

路径用**正斜杠**：YAML 双引号标量里的 `\` 是转义符，正斜杠免掉这个坑。
DSH **只在启动时读这个文件**，改完要重启 DSH（或重载 profile）。

### Claude Code — `%USERPROFILE%\.claude.json`

```json
{
  "mcpServers": {
    "everything": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\.agents\\skills\\everything-search\\bin\\everything-search-mcp.exe",
      "args": []
    }
  }
}
```

项目级也可以放 `.mcp.json`（同样的 `mcpServers` 结构）。

### Codex CLI — `%USERPROFILE%\.codex\config.toml`

```toml
[mcp_servers.everything]
command = 'C:\Users\<用户名>\.agents\skills\everything-search\bin\everything-search-mcp.exe'
args = []
```

单引号是 TOML 字面量字符串，反斜杠不会被转义。

### Claude Desktop / Gemini CLI / Cursor

`mcpServers` 结构，同 Claude Code（`type` 可省）：

- Claude Desktop → `%APPDATA%\Claude\claude_desktop_config.json`
- Gemini CLI → `%USERPROFILE%\.gemini\settings.json`
- Cursor → `%USERPROFILE%\.cursor\mcp.json`

### VS Code — `%APPDATA%\Code\User\mcp.json`

VS Code 用 `servers` 而不是 `mcpServers`：

```json
{
  "servers": {
    "everything": {
      "type": "stdio",
      "command": "C:\\Users\\<用户名>\\.agents\\skills\\everything-search\\bin\\everything-search-mcp.exe",
      "args": []
    }
  }
}
```

### opencode — `%USERPROFILE%\.config\opencode\opencode.json`

opencode 的写法又不一样：`command` 是**数组**、类型叫 `local`、环境变量键叫
`environment`，而且容器键是 `mcp`：

```json
{
  "mcp": {
    "everything": {
      "type": "local",
      "command": ["C:\\Users\\<用户名>\\.agents\\skills\\everything-search\\bin\\everything-search-mcp.exe"],
      "enabled": true
    }
  }
}
```

> 注意 **server 名不要重复**。如果之前叫 `everything-search`、现在叫 `everything`，
> 先把旧的那条从配置文件里删掉，
> 否则 opencode 会把同一套工具注册两遍。

## 4. 验证

先确认 Everything 的 HTTP 服务通（不经过 MCP）：

```powershell
# 目录写进 search=；path_column=1 才会返回 path 字段
$u = 'http://127.0.0.1:23333/?search=' + [uri]::EscapeDataString('path:"D:\Projects" *.py') + '&json=1&count=5&path_column=1'
$r = Invoke-RestMethod $u
$r.totalResults     # 有数字就说明索引和端口都正常
$r.results | ForEach-Object { Join-Path $_.path $_.name }
```

MCP 侧验证：重启客户端后调用 `everything_search`，`query` 填 `*.py`。
返回结果即成功。结果不合预期时先看回传的 `effective_query`。

## 5. 故障排查

| 现象 | 原因 / 处理 |
|---|---|
| `cannot reach Everything's HTTP server` | HTTP 服务器没开（见 §1）；或端口不是 23333，用 `--http-url` 覆盖 |
| 工具列表里没有 `everything_*` | 客户端没重启；或配置文件里的路径不对（用 `config` 重新生成） |
| 查询恒返回 0 条且很快 | 该语法 Everything 没索引，典型是 `content:`（见 SKILL.md 陷阱一节） |
| 结果全是 `node_modules` | 用 `max_per_parent` 做结果多样化，或加 `path` 限定 |
| 想确认实际执行了什么表达式 | 看结果里的 `effective_query` |
| `unknown server 'xxx'` | 名字没登记过。看 `servers.json` 里有哪些，或直接传地址 `http://host:23333` |
| 远程机器报连不上 | 那台机器没开机 / Everything 没在跑 / HTTP 口没开 / 防火墙或网段不通。远程报错里给的就是这个地址，先 ping 一下 |
| 远程命中却在本机读不到文件 | 路径属于那台机器，本机没有。详见 SKILL.md「搜别的机器」 |
| 多机结果里的 `total` | 是各机精确计数之和；某台连不上时汇总行会写 `N index(es) unreachable`，那行不能忽略 |

## 6. 可选：插件模式（MCP 直接挂在 Everything 进程里）

stdio 模式每次调用都要拉起一个 exe 进程：多几毫秒启动开销、多一份进程内存，重启后还要客户端自己重连。插件模式把同一套工具装进 Everything 自己的进程，在里面监听 `127.0.0.1:8286`，客户端改成 HTTP 形态：

```json
{ "mcpServers": { "everything-search": { "url": "http://127.0.0.1:8286/" } } }
```

装（需要管理员；Everything 只在启动时加载插件，所以会重启一次，窗口不会抢前台）：

```powershell
.\install-plugin.ps1                 # 从仓库构建并安装
.\install-plugin.ps1 -NoBuild        # 用包里自带的 DLL（发布包解压后的形态）
```

发布包里对应的是 `everything-search-plugin-<v>.zip`，里面有 `everything_search_mcp64.dll`、`install-plugin.ps1` 和 `PLUGIN.md`。插件是未签名的，Everything 对未列入白名单的 DLL **会静默拒绝加载且不写日志**——`install-plugin.ps1` 会写 `HKLM\...\AllowedPlugins`（分号分隔，逗号会让整份名单失效），这一步不能省。

细节：端口怎么改、怎么卸干净、加载失败的排查表，见发布包里的 `PLUGIN.md`。

## 7. 卸载

```powershell
# 1) 从各客户端配置里删掉 everything 条目（本工具不提供自动删除，避免误删）
#    备份文件是 *.bak-<时间戳>Z，可以直接还原
# 2) 删掉 skill 目录即可（exe 就在 skill 里，没有装到系统任何地方）
Remove-Item -Recurse '<本 skill 目录>'
# 3) 远程机器登记表（可选；删掉就回到"只搜本机"）
Remove-Item "$env:APPDATA\everything-search-mcp\servers.json"
```

如果曾经装过 Python 版（`uv tool install` / `pip install`），可以顺手清掉：

```powershell
uv tool uninstall everything-search-mcp
pip uninstall everything-mcp
```

## 相关文件

- 使用指南：`SKILL.md`（工具选择、参数语义、Everything 语法、陷阱）
- 维护仓库：`https://github.com/alwaysmy/everything-search-mcp`
  （`main` = 原生实现；老的 Python 版冻结在 `legacy` 分支，不再维护、也不是回退方案）
