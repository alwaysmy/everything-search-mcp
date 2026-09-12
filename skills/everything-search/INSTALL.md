# everything-search MCP 安装指南

仅在**首次安装 / 换机器 / 重新配置**时读本文件。日常调用 MCP 不需要。

## 0. 一句话版本

```powershell
# 让所有检测到的客户端都指向本 skill 自带的 exe（会先备份每个配置文件）
<本 skill 目录>\bin\everything-search-mcp.exe config --write
```

它会自动把 exe 的**绝对路径**写进 DeepSeek Harness / Claude Code / Codex / Gemini /
Cursor / VS Code 里已经存在的配置文件，并顺手探测 Everything 的 HTTP 服务是否通。

只想看看会写什么、不动任何文件：去掉 `--write` 即可。

### exe 从哪来

本 skill 自带 `bin\everything-search-mcp.exe`（约 580 KB，自包含，无运行时依赖）。
如果这个文件不在（比如只拷了 `SKILL.md`），从 release 取：

```powershell
# 不带 tag 就是取 latest，这样升级时这行不会过期
gh release download --repo alwaysmy/everything-search-mcp `
  --pattern 'everything-search-mcp.exe' --dir '<本 skill 目录>\bin'
# 或者直接下整个 skill 包：--pattern 'everything-search-skill.zip'
# 钉版本：gh release download v0.3.2 --repo ... 
```

或浏览器打开 <https://github.com/alwaysmy/everything-search-mcp/releases>。
放好后 `config --write` 会自动用上它的绝对路径，不需要改任何脚本。

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

## 2. 生成 / 应用配置

### 2.1 自动写入（推荐）

```powershell
& '<本 skill 目录>\bin\everything-search-mcp.exe' config --write
```

- 默认只处理**本机已存在**的客户端配置文件，不会凭空创建目录。
- 每个被改动的文件都会先备份成 `<原文件名>.bak-<UTC 时间戳>Z`。
- 只替换自己那一条 server 条目，其它内容原样保留（DSH 的 YAML 注释也不会被吃掉）。

只配某一个：

```powershell
... config --target dsh --write
... config --target claude,codex --write
... config --target vscode --write
```

删掉某一条（改了 server 名时用它，避免新旧两条同时存在）：

```powershell
... config --target opencode --name 旧名字 --remove
```

写到别处 / 换个 server 名：

```powershell
... config --file D:\some\.mcp.json --write      # 按扩展名推断格式（.yml/.yaml/.toml 其它按 JSON）
... config --name everything-search --write      # 默认名是 everything
```

### 2.2 只看不写

```powershell
... config                 # 列出检测到的目标 + 每个目标该粘什么
... config --json          # 机器可读版本
... config --target json   # 只要一段通用 mcpServers 块
```

### 2.3 可配置的环境变量

生成的配置默认**不写任何 env**——默认值已经是本机最常用的一组。要覆盖时用命令行旗标：

| 旗标 | 环境变量 | 默认 | 说明 |
|---|---|---|---|
| `--http-url` | `EVERYTHING_HTTP_URL` | `http://127.0.0.1:23333` | Everything HTTP 地址；改过端口才需要 |
| `--timeout` | `EVERYTHING_TIMEOUT` | `30` | 单次请求超时秒数 |
| `--max-results-cap` | `EVERYTHING_MAX_RESULTS_CAP` | `1000` | 单次返回条数硬上限（控制 token 用量） |

旧版的 `EVERYTHING_ES_PATH` / `EVERYTHING_INSTANCE` 已**不存在**，不要再用。

## 3. 各客户端配置形态（手工参考）

`config` 输出的就是下面这些；这里列出来是为了让你在别的机器上能照抄。

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
> 先用 `config --target opencode --name everything-search --remove` 把旧条目删掉，
> 否则 opencode 会把同一套工具注册两遍。

## 4. 验证

```powershell
# 1) 配置状态 + Everything 是否可达（会打印已索引对象数）
... config

# 2) 直接跑一次查询（不经过 MCP）
... search "*.py" --path D:\Projects --max 5
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

## 6. 卸载

```powershell
# 1) 从各客户端配置里删掉 everything 条目（本工具不提供自动删除，避免误删）
#    备份文件是 *.bak-<时间戳>Z，可以直接还原
# 2) 删掉 skill 目录即可（exe 就在 skill 里，没有装到系统任何地方）
Remove-Item -Recurse '<本 skill 目录>'
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
