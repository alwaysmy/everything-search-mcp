---
name: everything-search
description: "指导 AI 调用 everything-search MCP 工具（Windows 全盘毫秒级文件搜索）。触发词：找文件、搜文件、搜索文件、查找文件、文件搜索、Everything 搜索、按类型搜索、最近文件、文件详情、文件统计、文件大小、文件数量、找 datasheet / 手册 / 文档 / 图片 / 视频 / 代码、找工程目录、找项目、这个目录里有什么、列一下文件、哪个文件包含、在文件里找、什么最占空间、大文件、重复文件、空文件夹、在哪、位置、路径、搜别的电脑、远程搜索、另一台机器上的文件、多台机器一起搜、那台服务器上有没有。English triggers — find file, locate file, search files, where is, list files, file search, search my computer, find the config file, find a document, largest files, duplicate files, empty folders, recent changes, what changed, which file contains, grep for text, find the datasheet, file size, count files, find project, find the project folder, search another machine, search a remote computer, search multiple machines, remote Everything server."
---


# everything-search MCP 使用指南

> 判断该不该用、怎么不踩坑 → 本文。完整参数、CLI 兜底、**搜别的机器**、类别扩展名表、排错全集 → 同目录 `REFERENCE.md`。
> **没有 MCP 的环境（pi 之类只有 bash 的 harness）**：不要把 `everything-search-mcp.exe` 当命令跑，
> 改用 Everything 自带的 `es.exe`，见 `REFERENCE.md` 的「无 MCP 时的用法」。

## 前提：Everything 的 HTTP 服务器必须是开着的

服务端**不再调用 `es.exe`**，而是直接连 Everything 内置的 HTTP 服务器
（默认 `http://127.0.0.1:23333`）。

> **工具报连接错误时，先检查这一项**：Everything → 工具 → 选项 → **HTTP 服务器**，
> 确认已启用、端口是 23333。
> 若改过端口，用环境变量 `EVERYTHING_HTTP_URL` 覆盖。
>
> 不需要 `es.exe`，也**没有** `EVERYTHING_ES_PATH` 这个变量（写了会被忽略）——
> 所以没有任何外部程序路径需要配。整个服务端只依赖"Everything 进程在跑 + HTTP 口开着"。


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


## `everything_search` 关键参数

| 参数 | 说明 |
|---|---|
| `query` | Everything 语法，见下。用了 `category` 或 `entry_type` 时可以留空 |
| **`category`** | 十选一：`code` `document` `image` `video` `audio` `archive` `executable` `font` `3d` `data`。**优先用它**，不要自己手拼扩展名列表 |
| **`entry_type`** | `any`（默认）/ `file` / `folder`。**找工程目录、安装目录时必须用 `folder`**，不要靠结果猜类型 |
| `path` | 限定目录树。比在 query 里写 `path:` 更可靠 |
| `max_results` | 1–500，**默认 20** |
| `offset` | 分页 |
| `sort` | 14 种，见下表；默认 `date-modified-desc` |
| `match_case` / `match_whole_word` / `match_regex` / `match_path` | 匹配修饰符。**生效情况看回传的 `match_modes`**，别只看 `effective_query` |
| **`max_per_parent`** | 每个父目录最多保留 N 条。**Everything 很容易让前 20 条全来自同一个目录树**（比如一堆 `node_modules`），对定位没帮助；需要多样化时设成 2–3 |
| `include_total` | 文本形式里附上总数（总数本来就是精确的，这个只控制显不显示） |
| **`url`** | 搜哪台机器：名字、地址、或数组（多台一起搜）。**替换默认集合**，详见「搜别的机器」 |


## 精度：哪些是精确的，哪些是采样

| 量 | 精度 | 说明 |
|---|---|---|
| `count` / `total` | **exact / partial** | 单次可达索引报告精确数；存在失败索引时汇总为partial，scope说明覆盖范围 |
| `total_size` | **exact / unavailable** | 不外推。样本完整或exact_size完整读取且文件大小均已知时才给总和；目录消耗行但不计文件字节。缺失大小、断页、计数变化或后端失败则省略 |
| 扩展名分布 | **sampled / exact** | 只统计最初样本；覆盖全量时才是全量分布。某类缺失大小则省略size并标unavailable |

`exact_size`的多页遍历是实时扫描，不是原子快照；同样数量的文件在扫描中变化仍可能影响结果。
`probe=true`会把裸词编成一个名称/路径/目录的OR查询，再由Everything统一分页和计数，
不再只在少量结果时拼接三份首页。`max_total_results`严格限制批量输出总条数；
每个成功子项保留完整的游标、后端错误、match_modes和总量scope。


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


## 最佳实践

1. **先看 `total`，再决定要不要翻页。** 每条结果尾部都有
   `20 result(s) from offset 0 of 353178 matching (exact total).` ——
   `353178 / 20 = 17659`，这说明查询太宽，**该收窄，不该翻页**。
   翻 50 页才看完 1000 条，等于把 token 倒进一个没底的桶。

   **收窄手段，按实测倍数排**（基准：`read` 命中 155,903；`config` 141,221；
   `main` 91,684 —— 三组查询结论一致）：

   | 手段 | 例子 | 实测收窄 |
   |---|---|---|
   | **`category` / `ext:`** | `read ext:py` | **÷26 ~ ÷408**（最强） |
   | `path` 参数 | `--path D:\MyProjects` | ÷5 ~ ÷6 |
   | **两者叠加** | `read ext:py --path D:\MyProjects` | **÷500 ~ ÷1,250** |
   | `!path:<目录>` 排除整棵树 | `read !path:node_modules` | ÷1.25 |
   | `parent:` | `parent:D:\src`（只搜下一层） | 视层级 |
   | `exact:` / `wholeword:` / `case:` | 提高单条命中精度 | 不定量 |

   **两个必须记住的坑：**
   - **`!目录名` 只匹配"名字"，不匹配路径。** `!node_modules` 几乎不减条数
     （155,903 → 155,897，名字含它的只有 6 个）；要排除整棵树**必须写
     `!path:node_modules`**（→ 124,242）。
   - **排除的收益本来就有限** —— 索引已排除 `.git` / `__pycache__` / `.venv` 等。
     真正的数量级来自 **`ext:`/`category` 和 `path`**，不是排除。

   **两步就能把 15 万条降到几百条**：先 `ext:`（或 `category`），再 `path`。
   一次 5 条的精准查询远比一次 20 条的宽泛查询有用 —— 宽泛查询之后几乎总要再搜一次。
2. **找目录用 `entry_type: "folder"`**，不要从文件结果里猜哪个是工程根目录。
3. **类型搜索用 `category`**，不要手动拼扩展名列表。
4. **最近改动用 `everything_find_recent`**，不要手写 `dm:`。
5. **可能命中上千条时先用 `everything_count_stats` 探规模**，再决定要不要列表。
   `count` 是精确值，`0` 就是确定的"没有"。
6. **结果不合预期先读 `effective_query`**。
7. **要预览文件内容**：先 `everything_search` 拿路径，再 `everything_file_details`；
   或者直接用 agent 自己的 read 工具（本 MCP 不重复提供文件读取能力）。
8. **默认只返回 20 条**。要更多就显式传 `max_results`，或者跟着 `next_offset` 翻页 ——
   但先问一句：真的需要第 30 条吗，还是该换一个更准的查询？

## 陷阱与排错

- **连接失败** → 检查 Everything 的 HTTP 服务器是否启用（见文首）；这是唯一的传输途径，没有兜底。
  远程机器还要确认那台机器开着、Everything 在跑、HTTP 口对外可达。
- **拿远程路径在本机读** → 最隐蔽的坑：`source` 不是 `local` 的路径属于别的机器，
  在本机 read 只会报"找不到"，或者更糟 —— **本机恰好有同名路径时读到完全不同的文件，而且不报错**。
  要元数据就把同一个 `url` 传给 `everything_file_details`；要内容只能在那台机器上读。
- **多机结果里 `total` 是可达索引之和** → 检查accuracy与scope，存在失败就不是所有所选机器的全量。
  某台连不上时汇总行会写 `N index(es) unreachable`，**那行不能忽略**，否则会把不完整的数字当全量。
- **`total_size` 不按样本外推** → 不完整就省略并说明原因；多页精确求和也不是原子快照。
- **`breakdown` 里那个数字是"样本内的行数"，不是总数** → 表头写的是 `in sample`，
  下面还有一行明说。`exact_size: true`只扩展大小求和，不会把扩展名分布也变成全量。
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
  **本机路径可用 `everything_file_details`确认真实文件系统。** 远端只能查索引：
  未索引或连接失败时exists为null，另外看indexed/status；不能据此断言文件不存在。
  清掉幽灵：在那个路径上做一次干净的"创建 + 删除"即可（新 inode、无其它链接）。
- **NTFS 之外**：exFAT/FAT/网络盘没有 MFT，Everything 只能慢速扫描，结果可能不全。


## 更多

- `REFERENCE.md`：配置 MCP、CLI 模式、远程多机搜索、参数平铺与拼写报错、`effective_query`、
  `find_recent` / `file_details` / `count_stats` 细节、排序值、函数 vs 参数实测、类别扩展名表、pi 用法
- `INSTALL.md`：安装与配置排错