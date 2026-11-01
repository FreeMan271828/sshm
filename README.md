# sshm

比 sshs 更强的 SSH 连接管理器：增删改查 / 分类 / 备注 / 有效性 / 智能排序 / 连接测试 / 一键连接。

## 特性

- **连接增删改查（CRUD）**：`a` 新增、`e` 编辑、`d` 删除（带确认）、`Enter` 一键连接
- **打开位置可选**：新建标签页（默认，列表继续可用）或当前标签页，`o` 随时切换，配置立即落盘
- **默认终端可选**：自动跟随所在终端（Tabby / WezTerm / Windows Terminal），也可指定 Tabby、WezTerm、Windows Terminal、PowerShell、cmd、bash（Unix 另有 zsh）
- **完整字段**：IP/主机、端口（默认 22）、账号、密码、备注、分类、是否有效
- **智能排序**：默认 / 最近使用两种模式，两种模式下**有效连接永远排在无效连接前面**
- **编辑即复活**：编辑一个无效配置并保存后，默认恢复为有效
- **连接测试**：新增/编辑时可测试连接——有密码走真实密码认证握手，无密码则握手 + `none` 认证
- **导入 ~/.ssh/config**：一键导入（`i` 或 `sshm import`），自动去重，跳过通配符名称
- **使用统计**：记录最近使用时间与使用次数
- **搜索/过滤**：`/` 对全部可搜索字段做大小写不敏感的子串匹配
- **可靠持久化**：操作后立即落盘，原子写（临时文件 + 重命名），损坏的旧文件自动备份为 `hosts.json.corrupt-<时间戳>.bak`

## 安装

```bash
cargo install --path .
```

或只构建不安装：

```bash
cargo build --release
# 产物: target/release/sshm[.exe]
```

## 使用

直接运行 `sshm` 进入 TUI：

```bash
sshm
```

### 列表模式键位

| 键 | 功能 |
| --- | --- |
| `a` | 新增连接 |
| `e` | 编辑选中项 |
| `d` | 删除选中项（弹确认框） |
| `x` | 切换选中项 有效/无效 |
| `t` | 测试选中项连接 |
| `i` | 从 `~/.ssh/config` 导入 |
| `r` | 从磁盘重新加载 |
| `s` | 切换排序模式（默认 ⇄ 最近使用） |
| `o` | 打开设置（打开位置 / 默认终端） |
| `/` | 搜索/过滤（字符追加；`Esc` 清空并退出，`Enter` 退出搜索） |
| `?` / `F1` | 打开/关闭帮助 |
| `q` / `Esc` | 退出 |
| `Enter` | 连接选中项 |
| `j` / `k`（`↓`/`↑`） | 下移 / 上移（`PageDown`/`PageUp` 一次移动 10 行） |
| `g` | 跳到第一项 |
| `G` | 跳到最后一项 |

### 表单模式键位

| 键 | 功能 |
| --- | --- |
| `Tab` / `Shift+Tab` | 下一个 / 上一个字段 |
| `Enter` | 焦点在「是否有效」上时保存，否则跳到下一字段 |
| `Ctrl+S` | 保存 |
| `Ctrl+T` | 用当前表单值测试连接（不落盘） |
| `Ctrl+R` | 显示 / 隐藏密码 |
| `空格` / `x`（有效性字段） | 切换 有效/无效 |
| `Esc` | 关闭表单（有改动时先弹确认框） |

### 帮助页

| 键 | 功能 |
| --- | --- |
| `j` / `k` | 滚动 |
| `Esc` / `q` / `?` / `F1` | 返回列表 |

### 通用与弹窗

| 键 | 功能 |
| --- | --- |
| `Ctrl+C` | 任何时候直接退出（包括弹窗打开时） |
| `?` / `F1` | 打开帮助（列表模式） |
| 弹窗 `y` / `Enter` | 确认（删除主机 / 放弃修改） |
| 弹窗 其他任意键 | 取消；信息框任意键关闭 |

## 排序语义

两种排序模式共享一个不变式：**有效（valid）永远排在无效之前**。模式只决定各自组内的顺序：

- **默认**：分类（空分类视为 `~`，排在真实分类之后）→ 显示名 → 主机 → id，大小写不敏感，结果稳定
- **最近使用**：最近使用时间降序（**从未使用过的沉底**）→ 使用次数降序 → 显示名升序 → id

`last_used_at` / `use_count` 只在真正发起连接（`Enter`）时更新；连接测试只显示结果，不计入使用统计。`s` 键随时切换排序，列表立即重排。TUI 列表与 `sshm list` 共用同一个排序函数，不存在两套顺序。

## 设置：打开位置与默认终端

列表里按 `o` 打开设置页，两项配置修改后**立即写入** `hosts.json` 的 `settings` 段（无需保存键）：

| 配置 | 可选值 | 默认 |
| --- | --- | --- |
| 打开位置 | 新建标签页 / 当前标签页 | **新建标签页** |
| 默认终端 | 自动 / Tabby / WezTerm / Windows Terminal / PowerShell / cmd / bash（Unix 无后三项，另加 zsh） | 自动 |

- **新建标签页**：`Enter` 只负责在新终端里开会话，TUI 列表保持可用，可以连续开多台机器；`ssh` 退出后新标签页/窗口关闭（shell 预设会留在提示符，便于看报错或重连）。
- **当前标签页**：暂停 TUI，`ssh` 直接占用当前终端，会话结束后回到列表（旧行为）。此模式下「默认终端」不生效。
- **自动**：先看 sshm 自己跑在哪个终端里——Tabby（`TERM_PROGRAM=Tabby`）、WezTerm（`WEZTERM_PANE`）或 Windows Terminal（`WT_SESSION`）——就在该终端的新标签页打开；对应 CLI 找不到时退回 `wt -w 0 nt`，没有 `wt` 再用 PowerShell 窗口。Unix 上没有上述终端时自动挑一个终端模拟器（gnome-terminal / konsole / ptyxis / xfce4-terminal / kitty / alacritty / wezterm / xterm）。
- **各终端的打开方式**：Tabby = `tabby run`（在运行中的 Tabby 窗口新开标签页，Tabby 自身会弹一次“Run …?”确认框）；WezTerm = `wezterm cli spawn`（当前 WezTerm 窗口新标签页）；Windows Terminal = `wt -w 0 nt`；PowerShell / cmd / bash = 用 `cmd /c start` 开独立窗口（避免 `CREATE_NEW_CONSOLE` 继承 TUI 的标准句柄、把输出画到列表上）。
- **密码**：已保存的密码通过 `SSHM_ASKPASS_PW` 环境变量传给新进程，askpass 辅助脚本本身不含密码；Windows Terminal 会把调用方环境传给新标签页，因此保存的密码可直接免输。Tabby / WezTerm 的新标签页由它们自己的进程创建，不继承 sshm 的环境变量，`ssh` 会在新标签页里正常提示输入密码。
- **找不到终端程序**（例如没装 Tabby 就选了 Tabby）会给出「未找到 …」提示并在列表里保留 TUI，不会静默回退；可在设置里换一个终端。

设置页键位：`Tab` / `↑↓` 切换项目，`←` / `→` / `空格` / `Enter` 修改，`Esc` / `q` 返回。

## 数据存储位置

配置文件为 `hosts.json`，目录按以下优先级解析：

1. `--config <目录>` 命令行参数
2. `SSHM_CONFIG_DIR` 环境变量
3. 平台默认（均位于 `sshm` 子目录下）：

| 平台 | 默认位置 |
| --- | --- |
| Windows | `%APPDATA%\sshm\hosts.json` |
| macOS | `~/Library/Application Support/sshm/hosts.json` |
| Linux | `$XDG_CONFIG_HOME/sshm/hosts.json`（未设置则 `~/.config/sshm/hosts.json`） |

写入策略：先写同目录临时文件再 `rename` 覆盖（原子写）；若现有文件损坏，先备份为 `hosts.json.corrupt-<unix秒>.bak` 再写新文件，绝不静默覆盖。文件结构为：

```json
{
  "version": 1,
  "settings": { "open_mode": "new_tab", "terminal": "auto" },
  "hosts": []
}
```

`settings` 可选：旧文件缺少该段时按默认值加载；段内未知取值也会退回默认值，不会让整个文件解析失败。`open_mode` 取 `new_tab` / `current_tab`，`terminal` 取 `auto` / `tabby` / `wezterm` / `windows_terminal` / `powershell` / `cmd` / `bash` / `zsh`（跨平台同步配置时只保留当前平台存在的值）。

## 安全说明

- 密码以**明文**存储在 JSON 文件中；Unix 上文件权限自动收紧为 `0600`（仅属主可读写）。建议优先使用密钥认证，密码字段留空。
- 新标签页通过 `SSHM_ASKPASS_PW` 环境变量把密码交给 ssh；askpass 辅助脚本只 echo 该变量、本身不含密码，并在 sshm 退出时删除（位于系统临时目录）。
- 连接测试**不校验主机指纹**：方便快速验证连通性，但理论上存在中间人风险。正式连接交给系统 `ssh` 客户端，由 `known_hosts` 校验。

## CLI

```bash
sshm              # 启动 TUI
sshm list         # 列出所有连接（支持 --sort default|recent、--json）
sshm import       # 从 ~/.ssh/config 导入并打印导入报告
sshm --help       # 查看完整帮助
```

具体参数以 `sshm --help` 输出为准。`sshm list` 的输出示例（有效永远在前，端口 22 省略）：

```text
bastion              1.2.3.4                ops              -            有效     跳板
web-prod             10.0.0.11              root             生产           有效     生产 Web
old-db               10.0.0.12:2222         db               数据库          无效     已下线
```

`--json` 输出完整字段但**永不包含密码**：

```json
[
  {
    "id": "a3", "alias": "bastion", "host": "1.2.3.4", "port": 22,
    "username": "ops", "note": "跳板", "category": "", "valid": true,
    "last_used_at": 1700005000, "use_count": 1,
    "created_at": 1700000000, "updated_at": 1700000000
  }
]
```

导入报告形如 `导入完成: 新增 3 · 重复跳过 1 · 通配符跳过 1`。

### 导入规则

单向合并，不改动 `~/.ssh/config`：

- 只识别 `Host` / `HostName` / `Port` / `User`；一个 `Host` 行可写多个名称，每个名称一条记录（`alias` = 名称，`host` = `HostName`，缺省回落为名称本身）
- `Port` 非法或缺失按 22 处理，不中断导入
- 含 `*` / `?` / `!` 的名称视为通配符，**逐个跳过并计入「通配符跳过」**；同一 `Host` 行里的具体名称照常导入（`Host prod *.example.com` 会导入 `prod`）
- 已存在同 `host:port:user` 的记录（含同一文件内的重复块）计入「重复跳过」，不覆盖已有记录
- `Include`、`IdentityFile`、`ProxyJump`、`Match` 等指令忽略
- 新记录默认 `valid = true`、`password = null`、`note = "来自 ~/.ssh/config"`

## 测试

```bash
cargo test          # 单元 + CLI 集成 + 渲染快照，共 130 个
cargo clippy --all-targets   # 0 warning
```

| 层 | 文件 | 覆盖内容 |
| --- | --- | --- |
| 数据模型与排序 | `src/model.rs` | 排序不变式、过滤、字段校验、端口解析 |
| 设置 | `src/settings.rs` | 默认值、序列化键名、未知值降级、跨平台选项循环 |
| 应用状态与键位 | `src/app.rs` | 表单/列表/设置/弹窗状态机，编辑无效配置默认有效 |
| 持久化 | `src/storage.rs` | 原子写、损坏备份、目录优先级、settings 存取（临时目录隔离） |
| 连接层 | `src/ssh/` | 探测超时/阶段分类、当前页/新标签页命令构造、askpass |
| 导入 | `src/import.rs` | 引号/注释/通配符/去重/多名称 Host |
| CLI | `tests/cli.rs` | 真实启动 `sshm list/import --json`，断言不泄露密码、不丢设置 |
| 渲染 | `tests/render.rs` | `TestBackend` 无头渲染：列表/表单/设置/帮助/弹窗/极窄终端 |

真实终端冒烟（启动 TUI、渲染一帧后自动退出，验证 init/restore 正常）：

```bash
SSHM_TUI_EXIT_AFTER_MS=1500 cargo run --release -- --config /tmp/sshm-demo
```

## 项目结构

```text
src/
  main.rs      CLI 解析、事件循环、副作用（保存/连接/探测线程）
  app.rs       纯状态机：Mode/Dialog/FormState/Action/键位分发
  model.rs     Host/Store/SortMode，sorted_view 是唯一的排序实现
  settings.rs  OpenMode/Terminal/Settings：打开位置、默认终端，持久化契约
  storage.rs   hosts.json 原子读写，配置目录解析
  ssh/         probe.rs（russh 真实握手探测）+ launch.rs（当前页/新标签页拉起 ssh）
  import.rs    ~/.ssh/config 解析与合并
  ui/          list / form / settings / help / dialogs / theme 渲染
tests/
  cli.rs       端到端 CLI 行为
  render.rs    无头渲染断言
```

## 与 sshs 对比

| 功能 | sshs | sshm |
| --- | --- | --- |
| 增删改查 | ✅ | ✅ |
| 分类/备注 | ❌ | ✅ |
| 有效性标记 | ❌ | ✅ |
| 智能排序（有效优先 + 最近使用） | ❌ | ✅ |
| 连接测试 | ✅ | ✅（密码真实握手 / 无密码 `none`） |
| 导入 ~/.ssh/config | ✅ | ✅ |
| 最近使用时间/次数 | ❌ | ✅ |
| 新标签页 / 默认终端可选 | ❌ | ✅ |

> sshs 一栏基于其公开功能认知，如有出入以最新版本为准。
