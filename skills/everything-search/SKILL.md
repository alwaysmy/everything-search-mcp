---
name: everything-search
description: "指导 AI 调用 everything-search MCP 工具（Windows 全盘毫秒级文件搜索）。触发词：找文件、搜文件、搜索文件、查找文件、文件搜索、Everything 搜索、按类型搜索、最近文件、文件详情、文件统计、文件大小、文件数量、找 datasheet / 手册 / 文档 / 图片 / 视频 / 代码、找工程目录、找项目、这个目录里有什么、列一下文件、哪个文件包含、在文件里找、什么最占空间、大文件、重复文件、空文件夹、在哪、位置、路径、搜别的电脑、远程搜索、另一台机器上的文件、多台机器一起搜、那台服务器上有没有。English triggers — find file, locate file, search files, where is, list files, file search, search my computer, find the config file, find a document, largest files, duplicate files, empty folders, recent changes, what changed, which file contains, grep for text, find the datasheet, file size, count files, find project, find the project folder, search another machine, search a remote computer, search multiple machines, remote Everything server."
---

# everything-search MCP 使用指南

调用 Windows 上基于 voidtools Everything 实时 NTFS 索引的 `everything_*` MCP 工具。
**单次查询约 1 毫秒**，所以任何"按文件名/扩展名/大小/日期找东西"的场景都应该优先用它，
而不是 `dir /s`、`Get-ChildItem -Recurse` 或 glob。

## 前提：Everything 的 HTTP 服务器必须是开着的

服务端**不再调用 `es.exe`**，而是直接连 Everything 内置的 HTTP 服务器
（默认 `http://127.0.0.1:23333`）。

> **工具报连接错误时，先检查这一项**：Everything → 工具 → 选项 → **HTTP 服务器**，
> 确认已启用、端口是 23333。
> 若改过端口，用环境变量 `EVERYTHING_HTTP_URL` 覆盖。
>
> 不需要 `es.exe`，也**没有** `EVERYTHING_ES_PATH` 这个变量（写了会被忽略）——
> 所以没有任何外部程序路径需要配。整个服务端只依赖"Everything 进程在跑 + HTTP 口开着"。

## 配置 MCP（不用手写路径）

同一个 exe 自带配置生成器：它知道自己的绝对路径，所以**任何客户端配置都不用手写**，
skill 目录搬走也不会失效。

```powershell
<本 skill 目录>\bin\everything-search-mcp.exe config            # 只看：列出本机有哪些客户端 + 该粘什么
<本 skill 目录>\bin\everything-search-mcp.exe config --write    # 应用：自动写进各客户端配置（先备份）
<本 skill 目录>\bin\everything-search-mcp.exe config --target dsh --write
<本 skill 目录>\bin\everything-search-mcp.exe config --target opencode --name 旧名字 --remove
<本 skill 目录>\bin\everything-search-mcp.exe config --json     # 机器可读
```

- 默认只处理**本机已存在**的配置文件：`dsh`(DeepSeek Harness) `claude`(Claude Code)
  `claude-desktop` `codex` `gemini` `cursor` `vscode` `opencode`，外加 `json`（纯打印一段通用块）。
- 每个被改动的文件先备份成 `<原名>.bak-<UTC 时间戳>Z`；只替换自己那一条条目，
  其它条目/注释原样保留；重复运行第二次是 no-op。
- `--remove` 是反向操作：把这一条条目删掉（同样是先备份）。**改了 server 名字时用它**，
  否则客户端里会同时存在新旧两条，工具被注册两遍。
- 输出里会顺带探测 Everything 是否可达、当前索引了多少对象。
- **不带 `--write` / `--remove` 时一个字节都不写**。
- 更多安装/排错细节见同目录 `INSTALL.md`。

> **给 AI 的提示**：只有当用户明确要求"配置/安装 everything MCP"时才动配置。
> 判断某个客户端到底有没有配好，用 `config` 的打印模式看，不要凭猜。

## 没有 MCP 时的裸调用（CLI 模式）

同一个 exe 也是命令行工具 —— **不带参数时是 MCP 服务端**（客户端就是这样拉起它的），
**带子命令时是一次性 CLI**。所以即使 MCP 没装、没配、或当前 agent 不支持 MCP，
也可以直接用 shell 调：

```
<本 skill 目录>\bin\everything-search-mcp.exe search "*.py" --path D:\Projects --max 20
<本 skill 目录>\bin\everything-search-mcp.exe recent --period 1week --path D:\Projects
<本 skill 目录>\bin\everything-search-mcp.exe count "ext:pdf" --exact-size
<本 skill 目录>\bin\everything-search-mcp.exe details D:\a\b.rs --preview 20
<本 skill 目录>\bin\everything-search-mcp.exe config            # 生成 MCP 配置
<本 skill 目录>\bin\everything-search-mcp.exe --help
```

常用 flag：`--path` `--category` `--type file|folder` `--max` `--offset` `--sort`
`--per-parent` `--regex` `--case` `--whole-word` `--match-path` `--total` `--probe`
`--period` `--ext` `--preview` `--exact-size` `--breakdown` `--json`
`--url <名字|地址>`（可重复，搜多台机器；见「搜别的机器」）

```powershell
<exe> servers list                                   # 已登记的机器与开关
<exe> servers add <名字> <地址> [--disabled]         # 登记（首次默认打开）
<exe> servers enable|disable|remove <名字>
<exe> servers path                                   # 配置文件位置
```

退出码：`0` 成功、`1` 查询失败（错误在 stderr）、`2` 参数错误。
`--json` 输出结构化结果而非文本。

> **优先用 MCP 工具**（`everything_*`）：它是常驻进程，省去每次启动开销，且结构化字段更全。
> CLI 是**没有 MCP 时的兜底**，以及在脚本/CI 里用的形态。

## 工具速查（5 个）

| 目标 | 工具 |
|------|------|
| 按名字/扩展名/大小/日期/类别找文件或文件夹 | `everything_search` |
| 最近改动过的东西（"这周改了什么"） | `everything_find_recent` |
| 已知路径，要元数据或文本预览 | `everything_file_details` |
| 只要数量和体积，不要列表 | `everything_count_stats` |
| **一次问好几件事**（有没有 Cargo.toml / package.json / pyproject.toml） | `everything_search_batch` |

> 老版本里的 `everything_search_by_type` **已并入 `everything_search` 的 `category` 参数**。
> 不要再调那个工具名。

## 搜别的机器（远程 Everything）

Everything 的 HTTP 接口不限于本机：另一台机器上只要 Everything 在跑、HTTP 口开着，
这些工具就能直接搜它的索引。**同一个工具，多一个 `url` 参数** ——
不需要 WinRM、不需要登录那台机器、不需要装东西。

`url` 直接给地址就能用，**不登记也行**：

```json
{ "query": "*.pdf", "url": "http://10.0.0.2:23333" }
{ "query": "*.pdf", "url": ["local", "http://10.0.0.2:23333"] }
```

用户登记过的机器可以用名字（工具描述里的 `Known names` 会列出当前有哪些）：

```json
{ "query": "*.pdf", "url": "workshop" }
```

三种取值：

| `url` | 搜谁 |
|---|---|
| 不传 | 所有**已打开**的机器（默认只有本机） |
| `"workshop"` 或 `"http://10.0.0.2:23333"` | 只搜这一台 |
| `["local", "workshop"]` | 两台一起搜，结果按机器分组 |

**`url` 是替换，不是追加。** 传了就只搜传的那些 —— 想"本机 + 远程"必须显式写
`["local", "workshop"]`。这样"搜那台机器"不会偷偷把本机也搜进去。

### 多机结果长什么样

文本按机器分段，每段头一行是机器名和地址；结构化形式每条命中带 `source`，
另有 `backends[]` 列出每台的 `total` / `returned` / 错误：

```
=== local (http://127.0.0.1:23333) - 842 matching
  [FILE] D:\Projects\a.pdf  (1.2 MB, 2026-09-22 17:45:34)
=== workshop (http://10.0.0.2:23333) - 1274 matching
  [FILE] E:\manuals\3458A\Ag_3458A_UserGuide_en.pdf  (7.7 MB, 2026-09-23 01:27:25)

4 result(s) from offset 0 across 2 of 2 indexes; 2116 matching in total (exact).
```

必须知道的几点：

- **路径是那台机器上的路径，在本机打不开。** 每条结果带 `source` 就是为了这个：
  **`source` 不是 `local` 的路径，不要拿去 read、不要拿去 `everything_file_details`**
  （除非把同一个 `url` 也传过去）—— 否则你是在本机找一个同名的、完全不同的文件，
  而且**不会报错**。
- **`total` 是各机精确总数之和**，仍然精确（每台的计数本来就是精确的，相加不引入误差）。
  各机自己的数字在 `backends[]` 里。
- **`offset` 是"每台各自"的**（回传带 `offset_scope: "per instance"`）。
  多机分页时 `next_offset` 按各台返回条数的**最大值**前进，所以既不重复也不跳过；
  某一台结果不够一页时它返回空，这是正常的。
- **某台连不上不会让整次搜索失败**：它的错误单独列在那一台下面，其它机器的结果照常返回，
  汇总行会写 `N index(es) unreachable`。**只有单台时**失败才等于工具报错。
- **单台时输出形状不变**（扁平列表，没有分组头）—— 只有多台才分组。

### 什么时候该用多机

- 用户说"**那台机器上**有没有 / **服务器上**找找" → `url` 给那一台。
- 用户说"哪台有都行 / 都找找" → 传数组，或者不传（如果用户已经把远程机器打开了）。
- **本机搜不到不等于没有** —— 如果用户提过有另一台机器放资料，值得带 `url` 再搜一次。
- 别为了"更全"就无脑把远程都打开：多一台就多一次网络往返，而且结果里会混进
  那台机器的路径，反而更难读。

### 登记（用户配置动作）

登记写进 `%APPDATA%\everything-search-mcp\servers.json`，开关会记住（关掉的机器保持关，
不会每次搜索都去等它超时）：

```powershell
<exe> servers add workshop http://10.0.0.2:23333     # 登记并打开
<exe> servers add nas http://10.0.0.3:23333 --disabled
<exe> servers list                                   # 看登记与开关
<exe> servers disable workshop                       # 临时摘掉，保留登记
<exe> servers path                                   # 配置文件在哪
```

`local` 是内置的，不用登记；登记它只是给本机一个自己的开关和地址。
配置文件**每次调用都会重读**，所以改完立刻生效，不用重启 MCP 服务端。

## 参数一律平铺在顶层

```json
{ "query": "*.py", "path": "D:\\Projects", "category": "code" }
```

**不要**包一层 `params` —— 服务端为了兼容老客户端虽然也接受 `{"params": {...}}`，
但正确写法是平铺。

> **参数名写错会被点名。** 未知参数一律被忽略，所以写错名字**不会**报"未知参数"，
> 而是表现为别的问题 —— 典型是把 `entry_type` 写成 `type`，于是 query 为空，
> 报的是 `query is required`，看起来像"没给查询"。
> 现在**报错里会列出你实际传了哪些不认识的参数，并指出近似拼写**：
> `'type' looks like a misspelling of 'entry_type'`。看到这段就直接改名字，不要绕路。

## `everything_search` 关键参数

| 参数 | 说明 |
|---|---|
| `query` | Everything 语法，见下。用了 `category` 或 `entry_type` 时可以留空 |
| **`category`** | 十选一：`code` `document` `image` `video` `audio` `archive` `executable` `font` `3d` `data`。**优先用它**，不要自己手拼扩展名列表 |
| **`entry_type`** | `any`（默认）/ `file` / `folder`。**找工程目录、安装目录时必须用 `folder`**，不要靠结果猜类型 |
| `path` | 限定目录树。比在 query 里写 `path:` 更可靠 |
| `max_results` | 1–500，默认 50 |
| `offset` | 分页 |
| `sort` | 14 种，见下表；默认 `date-modified-desc` |
| `match_case` / `match_whole_word` / `match_regex` / `match_path` | 匹配修饰符。**生效情况看回传的 `match_modes`**，别只看 `effective_query` |
| **`max_per_parent`** | 每个父目录最多保留 N 条。**Everything 很容易让前 50 条全来自同一个目录树**（比如一堆 `node_modules`），对定位没帮助；需要多样化时设成 2–3 |
| `include_total` | 文本形式里附上总数（总数本来就是精确的，这个只控制显不显示） |
| **`url`** | 搜哪台机器：名字、地址、或数组（多台一起搜）。**替换默认集合**，详见「搜别的机器」 |

## 看 `effective_query` 自查

每个结果都会回传 **`effective_query`** —— 服务端实际执行的 Everything 表达式
（`path`/`category`/`entry_type`/`period` 展开之后）。**结果不合预期时先看它**，
比猜哪个参数没生效快得多。

> **注意：`match_case` / `match_whole_word` / `match_path` 不会出现在 `effective_query` 里。**
> 它们是以 HTTP 参数下发的，不是表达式的一部分。这三个的生效情况看
> **`match_modes`** 字段（文本形式里是 `match modes:` 一行），没设时是空数组。
> `match_regex` 例外 —— 它会编译成 `regex:` 函数，所以在 `effective_query` 里看得见。

结构化客户端还能拿到 `structuredContent`，字段与文本内容一致：
`query` / `effective_query` / `match_modes` / `total` / `total_accuracy` / `returned` /
`offset` / `next_offset` / `has_more` / `results[]`，每条含 `name` `path` `full_path`
`type` `size` `modified` **`source`**；另有 `backends[]`（每台的 `source` / `url` /
`total` / `returned` / `error`），多台时还有 `offset_scope`。

> **`source` 是"这条命中来自哪个索引"**，`local` 是本机，其余是那台机器的名字或 `host:port`。
> 非 `local` 的路径属于那台机器，在本机读不到 —— 见「搜别的机器」。

> `modified` 是**本地时间**（和资源管理器显示的一致），格式 `YYYY-MM-DD HH:MM:SS`。
> 直接报给用户即可，不要自己再加时区偏移。

## 精度：哪些是精确的，哪些是采样

| 量 | 精度 | 说明 |
|---|---|---|
| `count` / `total` | **exact** | Everything 直接报告真实总数，与取多少行无关。多台机器时是各机精确计数之和，仍然精确 |
| `total_size` | **sampled** | HTTP API 没有聚合，是按取到的样本外推的，必须当成估算 |
| 扩展名分布 | **sampled** | 同上 |

**不要把采样的体积当成精确值报给用户。**

## `everything_find_recent`

- `period`：默认 `1day`。可选 `1min` `5min` `10min` `15min` `30min` `1hour` `2hours` `6hours`
  `12hours` `today` `yesterday` `1day` `3days` `1week` `2weeks` `1month` `3months`
  `6months` `1year`。
  **也接受任意数值写法**：`7days`、`24hours`、`90days`、`1d`、`30min`、`2mo` ——
  底层 Everything 本来就支持 `last<N><单位>`，上面的名单只是便捷别名，
  `7days` 和 `1week` 是同一个窗口。或直接写原始语法 `last2hours`。真正的垃圾值才会被拒绝。
- `extensions`：`py,js,ts` 或 `py;js;ts` 都行。
- `auto_expand`：默认 true。窗口内结果太少时**自动放宽到全部时间**。
- **注意回传的 `requested_period` / `effective_period` / `expanded`** ——
  `expanded: true` 说明结果**不是**那个时间窗内的，别当成"最近 24 小时的变化"。
- **结果按修改时间倒序，所以"未来时间戳"的文件永远排最前。** 有些文件被故意设成
  2098-01-01 之类（打包/破解软件常见），Everything 的 `dm:last*` 是"从 X 之前到现在"
  的开区间，未来时间也满足，于是它们会霸占"最近改动"的前几条。
  看到这种时间戳直接**跳过或明确说明**，不要当成"刚改过"；需要真实近期改动时用
  `everything_search` 配 `dm:last7days` 并自己过滤掉未来时间。

## `everything_file_details`

- `paths`：1–20 个路径，**必须绝对路径**。
- `preview_lines`：0–200。文本文件的预览；`preview_truncated` 告诉你是否被截断。
- `url`：这些路径在**哪台机器**上。**一次只能一台** —— 传多台会直接报错，
  因为同一个路径在不同机器上是不同文件，混在一份回答里说不清描述的是哪台。
- **远程机器**（`url` 指向别的机器）时，回答的是**那台机器索引里的元数据**：
  大小、修改时间是真的（Everything 存了），类型是按**扩展名**判的。
  **没有文件头嗅探、没有预览** —— HTTP 接口给的是索引，不是文件内容。
  文本形式会写 `preview: unavailable`，结构化里是 `resolved_from: "index"` 加
  `preview_unavailable`。**要读内容必须在那台机器上读**，别在这边猜。
- **"索引里没有"不等于"文件不存在"**：Everything 只索引它被指到的 NTFS 卷，
  网络盘和被排除的目录根本不在索引里。工具会这样说明，不要把它当成"文件已删除"。

## `everything_count_stats`

- `count` 精确、`total_size`/`breakdown` 采样（见上）。
- **`count: 0` 是确定的"没有"，不是"没取到"** —— 它来自 Everything 索引的精确计数，
  与取多少行无关，也**不受采样影响**。所以**不要再用递归扫盘去复核一个 0**：
  本机 `D:\Projects` 递归枚举一次要 **228 秒**（275,434 个目录），
  而索引回答同一个问题只要几毫秒。0 看起来像"空结果"，但这两个 0 不是一回事。
- `sample_sort`：**开了 breakdown 时不能用 `name`**（文件名排序与扩展名相关，会带偏采样）。
- `category` / `entry_type` / `path` 都可用来限定。
- 想按"盘"分布时，索引没有现成的分组，只能每个盘各查一次（`path=D:\` 等）；
  非 NTFS 的盘（exFAT/FAT/网络盘）**不在索引里**，要单独说明或单独扫。

## 排序值（14 种）

`name` `name-desc` `path` `path-desc` `size` `size-asc` `size-desc`
`date-modified` `date-modified-asc` `date-modified-desc`
`date-created` `date-created-asc` `date-created-desc` `extension`

非法排序值会被拒绝（不会静默回退）。

## 大小写 / 全词 / 精确：优先用**函数**，不要用参数

Everything 有一组**匹配修饰函数**，直接写在 `query` 里，**所有工具都自动支持**
（`count_stats`、`find_recent`、`batch` 都能用），因为它们属于查询串，不走 HTTP 参数那条路：

| 函数 | 作用 | 本机实测（`README` / `read`） |
|---|---|---|
| `case:` | 大小写敏感 | `README` 67964 → `case:README` **44675** |
| `nocase:` | 显式不敏感（默认） | `nocase:README` 67964 |
| `wholeword:` / `ww:` | 全词匹配 | `read` 141836 → `wholeword:read` **7574** |
| `exact:` | 精确匹配整个名字 | `exact:README` 4164 |

```text
case:README                     # 只有大写 README
wholeword:read                  # read 不匹配 thread / already / spread
path:D:\src wholeword:read      # 和 path 函数正常组合
case:README regex:^README       # 和正则也能共存（函数在 regex: 前后都行）
```

`case:` 与 HTTP 的 `&case=1` 实测**结果完全相同**（都是 44675），而且函数优先级更高 ——
`case:README` 同时传 `&case=0` 仍然是 44675。

> **那 `match_case` / `match_whole_word` 参数什么时候还要用？**
> **只有正则查询需要。** 正则模式下整串会被当成模式解析，函数边界不再可靠，
> 所以 `everything_search(query="^README\.md$", match_regex=true, match_case=true)`
> 是唯一必须走参数的组合。非正则场景请一律用函数 —— 它跨工具通用。
>
> 无论走哪条路，**回传的 `match_modes` 都会告诉你哪些标志真的生效了**。

## 常用 Everything 语法

```text
*.py                          # 所有 Python 文件
ext:py;js;ts                  # 多扩展名（优先用 category 参数代替）
ext:py !test !__pycache__     # 排除
folder:  /  file:             # 只搜目录 / 只搜文件（优先用 entry_type 参数）
size:>10mb   size:1kb..1mb    # 大小
dm:today   dm:last1week       # 修改时间（优先用 find_recent）
dc:2024                       # 创建时间（慢！默认未索引）
"exact name.txt"              # 精确文件名（含空格要加引号）
project1 | project2           # OR
!node_modules                 # 排除
dupe:                         # 重复文件名
empty:                        # 空文件夹
content:TODO ext:py           # 内容搜索：需要 Everything 开启内容索引，否则恒返回 0
regex:^test_.*\.py$           # 正则（或用 match_regex 参数）
parent:C:\src ext:py          # src 下一层
case:README                   # 大小写敏感（见上一节，所有工具通用）
wholeword:read                # 全词匹配
```

## 文件类别覆盖的扩展名

| 类别 | 扩展名 |
|---|---|
| `audio` | mp3 wav flac aac ogg wma m4a opus aiff alac |
| `video` | mp4 avi mkv mov wmv flv webm m4v mpeg mpg 3gp ts |
| `image` | jpg jpeg png gif bmp svg webp tiff tif ico raw heic heif avif psd |
| `document` | pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt md epub pages numbers key |
| `code` | py js ts jsx tsx c cpp h hpp cs java go rs rb php swift kt scala r lua sh bash ps1 bat cmd sql html css scss sass less vue svelte dart zig nim hx ex exs erl hs ml fs clj lisp asm toml yaml yml json xml ini cfg conf env dockerfile makefile cmake gradle sbt proto graphql tf hcl |
| `archive` | zip rar 7z tar gz bz2 xz tgz zst lz4 cab iso dmg |
| `executable` | exe msi dll sys com scr appx msix |
| `font` | ttf otf woff woff2 eot fon |
| `3d` | obj fbx stl blend dae 3ds gltf glb usd usda usdz step iges |
| `data` | csv tsv json jsonl ndjson xml sqlite db mdb accdb parquet arrow avro hdf5 feather |

## 最佳实践

1. **先粗后细**：先用简单 `query`，结果太多再加 `path` / `category` / `ext:`。
2. **找目录用 `entry_type: "folder"`**，不要从文件结果里猜哪个是工程根目录。
3. **类型搜索用 `category`**，不要手动拼扩展名列表。
4. **最近改动用 `everything_find_recent`**，不要手写 `dm:`。
5. **可能命中上千条时先用 `everything_count_stats` 探规模**，再决定要不要列表。
6. **结果不合预期先读 `effective_query`**。
7. **要预览文件内容**：先 `everything_search` 拿路径，再 `everything_file_details`；
   或者直接用 agent 自己的 read 工具（本 MCP 不重复提供文件读取能力）。

## 陷阱与排错

- **连接失败** → 检查 Everything 的 HTTP 服务器是否启用（见文首）；这是唯一的传输途径，没有兜底。
  远程机器还要确认那台机器开着、Everything 在跑、HTTP 口对外可达。
- **拿远程路径在本机读** → 最隐蔽的坑：`source` 不是 `local` 的路径属于别的机器，
  在本机 read 只会报"找不到"，或者更糟 —— **本机恰好有同名路径时读到完全不同的文件，而且不报错**。
  要元数据就把同一个 `url` 传给 `everything_file_details`；要内容只能在那台机器上读。
- **多机结果里 `total` 是各机之和** → 它仍然精确，但"总量"跨的是多台机器，报给用户时说清楚。
  某台连不上时汇总行会写 `N index(es) unreachable`，**那行不能忽略**，否则会把不完整的数字当全量。
- **`total_size` 是采样** → 不要当精确值。
- **`breakdown` 里那个数字是"样本内的行数"，不是总数** → 表头写的是 `in sample`，
  下面还有一行明说。要真实的每类数量/体积，传 `exact_size: true`。
- **正则写错会被明确拒绝**（括号/字符类不配对、结尾孤立反斜杠）→ 报错而不是返回空结果。
  但**更隐蔽的正则错误 Everything 仍然静默返回 0 条**，所以正则查询得到 0 时，
  先怀疑模式本身，别直接下"没有这个文件"的结论。
- **Everything 必须在运行**（系统托盘）。
- **默认搜全部已索引磁盘** → 不加 `path` 就是全盘；大结果集用 `max_results`/`offset` 分页。
- **`content:` 不是"慢"，是"没有"** → 未开启内容索引时（`include_file_content=0`）一律**约 2ms 返回 0 条**，
  Everything 不做任何按需扫描。这是个静默失败：**不会报错，只会返回空结果**，很容易误判成"文件里没这个词"。
  要按内容找文件，正确做法是**先按文件名/类型缩小候选，再用 agent 自己的 read/grep 工具去读**。
- **即便开了内容索引，也还依赖 IFilter** → 文件类型必须有可用的文本过滤器（Windows 自带纯文本；
  Office/PDF 需要额外装）。**本机没有 PDF IFilter**，所以 PDF 即使被列入索引范围也提取不出文本。
- **索引可能对"被删掉的硬链接"留下幽灵条目。** 一个 inode 有多个名字时，删掉**其中一个**
  名字，Everything 不会清除那条路径记录（实测可复现）—— 因为文件本身还活着（另一个名字还在）。
  删掉**最后一个**链接才正常清除。触发场景很常见：部署脚本用"改名让位 + 重建硬链接"来替换
  正在运行的可执行文件，就会造出同 inode 的两个名字。
  表现：搜 `*.old` 会返回**并不存在**的路径。
  **要确认某个路径是否真的存在，用 `everything_file_details`** —— 它读真实文件系统，
  不存在会报 `os error 2`，而 `exists` 字段同理。
  清掉幽灵：在那个路径上做一次干净的"创建 + 删除"即可（新 inode、无其它链接）。
- **NTFS 之外**：exFAT/FAT/网络盘没有 MFT，Everything 只能慢速扫描，结果可能不全。
