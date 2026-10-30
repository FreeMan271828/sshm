# sshm

比 sshs 更强的 SSH 连接管理器：增删改查 / 分类 / 备注 / 有效性 / 智能排序 / 连接测试 / 一键连接。

## 特性

- **连接增删改查（CRUD）**：`a` 新增、`e` 编辑、`d` 删除（带确认）、`Enter` 一键连接
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

写入策略：先写同目录临时文件再 `rename` 覆盖（原子写）；若现有文件损坏，先备份为 `hosts.json.corrupt-<unix秒>.bak` 再写新文件，绝不静默覆盖。文件结构为 `{ "version": 1, "hosts": [...] }`。

## 安全说明

- 密码以**明文**存储在 JSON 文件中；Unix 上文件权限自动收紧为 `0600`（仅属主可读写）。建议优先使用密钥认证，密码字段留空。
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
cargo test          # 单元 + CLI 集成 + 渲染快照，共 101 个
cargo clippy --all-targets   # 0 warning
```

| 层 | 文件 | 覆盖内容 |
| --- | --- | --- |
| 数据模型与排序 | `src/model.rs` | 排序不变式、过滤、字段校验、端口解析 |
| 应用状态与键位 | `src/app.rs` | 表单/列表/弹窗状态机，编辑无效配置默认有效 |
| 持久化 | `src/storage.rs` | 原子写、损坏备份、目录优先级（临时目录隔离） |
| 连接层 | `src/ssh/` | 探测超时/阶段分类、ssh 命令构造、askpass |
| 导入 | `src/import.rs` | 引号/注释/通配符/去重/多名称 Host |
| CLI | `tests/cli.rs` | 真实启动 `sshm list/import --json`，断言不泄露密码 |
| 渲染 | `tests/render.rs` | `TestBackend` 无头渲染：列表/表单/帮助/弹窗/极窄终端 |

## 项目结构

```text
src/
  main.rs      CLI 解析、事件循环、副作用（保存/连接/探测线程）
  app.rs       纯状态机：Mode/Dialog/FormState/Action/键位分发
  model.rs     Host/Store/SortMode，sorted_view 是唯一的排序实现
  storage.rs   hosts.json 原子读写，配置目录解析
  ssh/         probe.rs（russh 真实握手探测）+ launch.rs（拉起系统 ssh）
  import.rs    ~/.ssh/config 解析与合并
  ui/          list / form / help / dialogs / theme 渲染
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

> sshs 一栏基于其公开功能认知，如有出入以最新版本为准。
