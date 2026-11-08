# actionlint 移植规划（Go → Rust）

将 `~/Projects/actionlint`（Go 实现的 GitHub Actions workflow 静态检查器，约 34k 行 Go）完整移植为 Rust CLI。
最终产物：名为 `actionlint` 的二进制，功能与原项目对齐（原项目 Bug 除外），符合 Rust 工业级规范。

> 原项目路径：`~/Projects/actionlint`（禁止修改其代码）
> 目标项目路径：`~/Projects/actionlint-rs`

---

## 1. 原项目完整能力清单

actionlint 是单命令工具：`actionlint [FLAGS] [FILES...] [-]`。无子命令，全部能力通过 flag 与输入方式体现。

### 1.1 命令行参数（`command.go`）

| 参数 | 短选项 | 类型 | 默认值 | 必填 | 说明 |
|------|--------|------|--------|------|------|
| `--ignore` | 无 | 正则（可重复） | 空 | 否 | 匹配错误消息则忽略该错误，可多次指定 |
| `--shellcheck` | 无 | string | `shellcheck` | 否 | shellcheck 可执行文件名/路径，空则禁用 shellcheck 集成 |
| `--pyflakes` | 无 | string | `pyflakes` | 否 | pyflakes 可执行文件名/路径，空则禁用 pyflakes 集成 |
| `--oneline` | 无 | bool | `false` | 否 | 每个错误输出一行（便于程序读取） |
| `--format` | 无 | string | 空 | 否 | 用 Go template 语法自定义错误输出格式 |
| `--config-file` | 无 | string | 空 | 否 | 指定配置文件路径 |
| `--init-config` | 无 | bool | `false` | 否 | 在当前项目生成默认 `.github/actionlint.yaml` 后退出 |
| `--no-color` | 无 | bool | `false` | 否 | 关闭彩色输出 |
| `--color` | 无 | bool | `false` | 否 | 强制开启彩色输出 |
| `--verbose` | 无 | bool | `false` | 否 | 输出详细日志 |
| `--debug` | 无 | bool | `false` | 否 | 输出调试日志（开发用） |
| `--version` | 无 | bool | `false` | 否 | 显示版本与构建信息 |
| `--stdin-filename` | 无 | string | `<stdin>` | 否 | 从 stdin 读取时的文件名 |
| `-h` / `--help` | `-h` | — | — | 否 | 打印用法后以状态 0 退出 |

位置参数 `FILES...`：待检查的 workflow 文件路径；单个 `-` 表示从 stdin 读取；无参数则自动定位最近的 `.github/workflows` 目录检查整个仓库。

### 1.2 退出码定义（`command.go`）

| 退出码 | 常量 | 含义 |
|--------|------|------|
| 0 | `ExitStatusSuccessNoProblem` | 成功且未发现问题（含 `-h`、`--version`、`--init-config`） |
| 1 | `ExitStatusSuccessProblemFound` | 成功运行但发现了问题 |
| 2 | `ExitStatusInvalidCommandOption` | 命令行参数解析失败 |
| 3 | `ExitStatusFailure` | 检查过程中发生致命错误 |

### 1.3 输入输出与核心功能

| 功能 | 输入 | 输出/行为 | 依赖资源 |
|------|------|-----------|----------|
| 检查整个仓库 | 无参数 | 递归检查 `.github/workflows` 下所有 `.yml`/`.yaml`，按路径排序 | 文件系统、`.git` 目录、`.github/workflows` |
| 检查指定文件 | 文件路径列表 | 逐个检查（多文件并行，受 CPU 数限制） | 文件系统 |
| 检查 stdin | `-` | 将 stdin 内容作为 workflow 检查 | 标准输入 |
| 生成默认配置 | `--init-config` | 写 `.github/actionlint.yaml`（已存在则报错） | 文件系统 |
| 错误格式化 | `--format` | Go template 渲染错误，支持 `json`/`replace`/`toPascalCase`/`getVersion`/`allKinds` 函数 | — |
| 错误过滤 | `--ignore`、配置文件 `paths.*.ignore` | 正则匹配错误消息则忽略 | — |
| 并行外部命令 | shellcheck/pyflakes | 以 CPU 数为上限并发运行外部命令 | 外部命令、管道 |

### 1.4 检查规则清单（`rule_*.go`）

| 规则名 | 来源文件 | 功能 |
|--------|----------|------|
| `matrix` | rule_matrix.go | 检查 matrix 组合（include/exclude、重复、类型） |
| `credentials` | rule_credentials.go | 检查容器 credentials 中疑似明文密码 |
| `shell-name` | rule_shell_name.go | 检查 `shell:` 取值是否受支持 |
| `runner-label` | rule_runner_label.go | 检查 `runs-on` label（预设/GitHub-hosted/自托管） |
| `events` | rule_events.go | 检查 `on:` 事件名与 webhook filter |
| `job-needs` | rule_job_needs.go | 检查 `needs:` 引用的 job ID 存在且无环 |
| `action` | rule_action.go | 检查 action 引用（本地/远程 metadata、inputs） |
| `env-var` | rule_env_var.go | 检查 env var 名合法性 |
| `id` | rule_id.go | 检查 step/job ID 唯一性与命名 |
| `glob` | rule_glob.go | 检查 filter glob 模式语法 |
| `permissions` | rule_permissions.go | 检查 permissions scope 与取值 |
| `workflow-call` | rule_workflow_call.go | 检查 reusable workflow 调用（inputs/secrets/outputs） |
| `expression` | rule_expression.go | 检查 `${{ }}` 表达式（上下文/函数/类型/不可信输入） |
| `deprecated-commands` | rule_deprecated_commands.go | 检查已废弃的 set-output 等命令 |
| `if-cond` | rule_if_cond.go | 检查 `if:` 条件中多余的 `${{ }}` 包裹 |
| `shellcheck` | rule_shellcheck.go | 集成外部 shellcheck 检查 `run:` 脚本 |
| `pyflakes` | rule_pyflakes.go | 集成外部 pyflakes 检查 Python 脚本 |
| `syntax-check` | parse.go | workflow YAML 语法检查（解析期产生，非独立 rule） |

### 1.5 依赖的系统资源与第三方库（原项目）

- 系统资源：文件系统遍历、Git 仓库识别（`.git`）、环境变量（`$NO_COLOR`）、外部命令执行（shellcheck/pyflakes）、标准输入/输出/错误、CPU 核数。
- Go 第三方库：
  - `go.yaml.in/yaml/v4`：YAML 解析（保留注释/位置/anchor）
  - `github.com/bmatcuk/doublestar/v4`：`**` glob 匹配（配置 `paths`）
  - `github.com/fatih/color` + `mattn/go-colorable`：彩色输出
  - `mattn/go-runewidth`：终端字符宽度（错误指示符对齐）
  - `mattn/go-shellwords`：解析 shellcheck/pyflakes 命令行
  - `robfig/cron/v3`：校验 schedule cron 表达式
  - `golang.org/x/sync`（errgroup/semaphore）：并发控制
  - `golang.org/x/net`、`yuin/goldmark`：仅文档/脚本使用，运行时非必需

---

## 2. 功能移植优先级与排期

按「核心基础功能 → 高频使用功能 → 边缘辅助功能」排序，每个阶段可独立验证。

### 阶段 P0：项目骨架与基础类型
- **交付物**：`Cargo.toml`（bin 名 `actionlint`、release 优化、clippy 配置）、模块骨架、`Pos`/`Error`/`String`/`Bool` 等基础类型、`Config` 解析。
- **验收**：`cargo build` 通过；`Config` 能解析 `.github/actionlint.yaml`；`cargo clippy` 无警告。

### 阶段 P1：表达式引擎
- **交付物**：`expr_type`（类型系统）、`expr_ast`（表达式 AST）、`expr_lexer`（词法）、`expr_parser`（语法）、`expr_insecure`（不可信输入检测）。
- **验收**：表达式词法/语法/类型单测通过；覆盖原项目 `expr_*_test.go` 的代表性用例。

### 阶段 P2：workflow YAML 解析器
- **交付物**：`parse`（把 YAML 解析为 `Workflow` AST，收集 `syntax-check` 错误）、`availability`（context/function 可用性表）。
- **验收**：对 `testdata/examples`、`testdata/ok`、`testdata/err` 中的样例解析结果与原项目一致。

### 阶段 P3：数据表
- **交付物**：`popular_actions`（热门 action metadata）、`all_webhooks`（webhook 活动类型）。
- **验收**：数据条目数与原项目一致；`rule_action`/`rule_events` 可查询。

### 阶段 P4：Visitor 与检查规则
- **交付物**：`pass`（Visitor）、`rule` trait、全部 17 个规则（不含外部命令集成的 shellcheck/pyflakes 先行，含 expression）。
- **验收**：`testdata/ok`、`testdata/err` 端到端检查结果与原项目对齐。

### 阶段 P5：外部命令集成
- **交付物**：`process`（并发外部命令）、`rule_shellcheck`、`rule_pyflakes`。
- **验收**：本机装有 shellcheck/pyflakes 时集成生效；未安装时优雅禁用。

### 阶段 P6：Linter 编排、错误格式化与项目定位
- **交付物**：`linter`（仓库/目录/文件/stdin 检查、并行、错误排序去重过滤）、`error`（格式化、彩色、snippet 指示符）、`project`（项目定位与缓存）、Go template 格式化。
- **验收**：`actionlint`（无参）、`actionlint file.yaml`、`actionlint -`、`--format`、`--init-config` 端到端行为与原项目一致。

### 阶段 P7：CLI 与跨平台
- **交付物**：`main`/命令行参数解析、退出码、`--version`；CI 配置、`cross` 交叉编译。
- **验收**：所有退出码正确；必须支持目标平台编译通过；CI 全绿。

---

## 3. 实现方案说明

### 3.1 模块划分（Rust）

```
src/
├── main.rs            # 入口，调用 command::run 并返回退出码
├── lib.rs             # 库根，re-export 公共 API
├── command.rs         # CLI 参数解析、退出码、用法文本
├── linter.rs          # Linter 编排（仓库/目录/文件/stdin）
├── error.rs           # Error、格式化、彩色、snippet、Go template
├── config.rs          # Config 解析与 paths ignore
├── project.rs         # Project/Projects 定位与缓存
├── glob.rs            # glob 校验器（ref/path）
├── process.rs         # 并发外部命令执行
├── parse.rs           # workflow YAML → AST，收集 syntax-check 错误
├── ast.rs             # Workflow/Job/Step/Event 等语法树
├── pass.rs            # Visitor 遍历
├── rule.rs            # Rule trait 与 RuleBase
├── rule_*.rs          # 各检查规则
├── expr.rs            # ExprError
├── expr_ast.rs        # 表达式 AST
├── expr_type.rs       # 表达式类型系统
├── expr_lexer.rs      # 表达式词法分析
├── expr_parser.rs     # 表达式语法分析
├── expr_sema.rs       # 表达式类型检查
├── expr_insecure.rs   # 不可信输入检测
├── availability.rs    # context/function 可用性
├── action_metadata.rs # action metadata 解析
├── popular_actions.rs # 热门 action 数据（生成）
├── all_webhooks.rs    # webhook 活动类型（生成）
└── reusable_workflow.rs # reusable workflow metadata 缓存
```

### 3.2 第三方 crate 选型（含理由）

| crate | 版本 | 用途 | 选型理由 |
|-------|------|------|----------|
| `saphyr` | 0.3 | workflow/config YAML 解析 | YAML 1.2 完整实现，提供带位置信息的原始节点树（`Yaml`/`Scalar`/`Sequence`/`Mapping`），可拿到 anchor，是 `go.yaml.in/yaml/v4` 的 Rust 对应物。维护活跃 |
| `serde` + `serde_yaml`/`serde_json` | 最新稳定 | action metadata、webhook 数据、JSON 输出 | 成熟稳定，社区标准 |
| `regex` | 最新稳定 | `--ignore`、错误过滤 | Rust 生态标准正则库，性能好 |
| `globset` | 0.4 | 配置 `paths` 的 `**` glob | ripgrep 同款，成熟稳定，支持 `**` 语义 |
| `colored` | 3.x | 彩色输出 | 成熟稳定，支持 `NO_COLOR`/`isatty` |
| `unicode-width` | 0.2 | 终端字符宽度 | 错误指示符 `^~~~` 对齐，对应 `mattn/go-runewidth` |
| `shellwords` | 1.1 | 解析 shellcheck/pyflakes 命令行 | 对应 `mattn/go-shellwords` |
| `croner` 或 `cron` | 最新稳定 | schedule cron 校验 | 对应 `robfig/cron`；优先 `croner`（维护活跃、支持标准 5 段） |
| `rayon` | 最新稳定 | 多文件并行检查 | 对应 errgroup+semaphore，成熟稳定 |
| `gtmpl` | 0.7 | `--format` Go template 渲染 | 直接提供 Golang template 语法，避免重写模板引擎 |
| `anyhow`/`thiserror` | 最新稳定 | 错误处理 | `thiserror` 定义库错误类型，`anyhow` 用于顶层命令错误传播 |
| `terminal_size` 或 `is-terminal` | 最新稳定 | 终端检测 | 彩色自动判断（配合 `colored` 自带能力，按需引入） |

> 说明：优先使用标准库（如 `std::fs`、`std::process::Command`、`std::path`、`std::sync`）。仅当标准库无对应能力时引入上表 crate。`saphyr` 与 `serde_yaml` 分工：`saphyr` 用于需要原始节点树与位置的 workflow/metadata 解析，`serde_yaml` 用于结构化的 config 解析；若 `saphyr` 已足够，则不引入 `serde_yaml` 以精简依赖。

### 3.3 关键技术风险与备选方案

| 风险 | 说明 | 备选方案 |
|------|------|----------|
| Go template 兼容 | `--format` 依赖 Go template 语法 | 用 `gtmpl`；若个别函数缺失则自行注册 `json`/`replace`/`toPascalCase`/`getVersion`/`allKinds` |
| YAML 位置信息 | workflow 解析需精确行/列 | `saphyr` 节点树带位置；若不足则在词法层自行记录 |
| anchor/merge key | workflow 可能使用 YAML anchor | `saphyr` 支持 anchor；merge key 若不支持则在解析层手工展开 |
| 生成数据体积 | `popular_actions.go` 约 275KB | 转为构建期生成的 Rust 源或 `include_str!` + JSON，避免运行时解析开销 |
| 并发外部命令 | 需限制并发数 | 用 `rayon` 线程池上限或 `Semaphore`（`tokio` 不引入，保持同步） |
| cron 语义差异 | GitHub schedule 用标准 5 段 cron | `croner` 支持 5 段；不一致处以原项目行为为准 |

---

## 4. 不可移植功能说明

以下功能在 Rust 版本中**不适用或无需移植**，后续无需重复评估：

| 功能 | 不可移植原因 | 替代方案 |
|------|--------------|----------|
| Go `runtime.Version()` / `GOOS` / `GOARCH` 构建信息 | `--version` 中的 "built with goX.Y.Z compiler for os/arch" 是 Go 运行时专属 | Rust 版改为输出 Rust 编译器版本（`env!("RUSTC_VERSION")` 或构建脚本注入）与目标三元组；保持输出结构一致 |
| `debug.ReadBuildInfo()` 读取版本 | Go module 专属 | 用 `env!("CARGO_PKG_VERSION")` 提供版本号 |
| `text/template` 原生引擎 | Go 标准库专属 | 用 `gtmpl` 提供 Golang template 语法 |
| Go playground 链接生成 | 仅文档/CI 脚本使用 | 不移植（非运行时功能） |
| `scripts/*`（数据生成、文档校验） | Go 构建期脚本 | 用等价 Rust 构建脚本或一次性迁移工具生成 Rust 数据文件 |

除上表外，所有 workflow 检查能力、规则、表达式引擎、格式化、项目定位均可移植。

---

## 5. 原 Bug 修复清单

移植过程中发现的 Bug 将在此记录（含修复方案）。**不沿用原错误逻辑。**

| 编号 | 位置 | 原 Bug 描述 | Rust 修复方案 |
|------|------|-------------|---------------|
| — | — | 移植进行中，发现的 Bug 将补充于此 | — |

---

## 6. 测试规划

- **单元测试**：每个模块内 `#[cfg(test)]`，覆盖核心函数、边界与错误分支。目标：核心逻辑（expr 引擎、parse、glob、config、error 格式化、各 rule）覆盖率 ≥80%。
  - expr 词法/语法/类型：移植 `expr_lexer_test.go`、`expr_parser_test.go`、`expr_sema_test.go`、`expr_type_test.go` 代表性用例。
  - glob 校验：移植 `glob_test.go` 用例（ref/path 两类）。
  - config：移植 `config_test.go`。
  - error 格式化：移植 `error_test.go`（snippet 指示符、JSON、PascalCase）。
- **集成测试**：`tests/` 下针对 CLI 子命令与 Linter API。
  - 检查 `testdata/ok`、`testdata/err`、`testdata/examples` 样例，比对错误集合。
  - 覆盖正常流程、参数校验、异常输入、文件不存在、权限不足、stdin。
- **端到端测试**：`tests/e2e.rs` 调用编译后的 `actionlint` 二进制，模拟真实使用，校验 stdout/stderr/退出码与原项目一致。
- **数据驱动测试**：复用原项目 `testdata/`（拷贝到本仓库 `testdata/`），用表格驱动比对。

---

## 7. 平台兼容规划

| 目标平台三元组 | 支持等级 | 编译方式 | CI 验证 | 注意事项 |
|----------------|----------|----------|---------|----------|
| `x86_64-unknown-linux-musl` | 必须 | `cross build --release --locked --target x86_64-unknown-linux-musl` | Linux Runner + cross | 静态链接，兼容低 GLIBC；外部命令 shellcheck/pyflakes 需运行时存在 |
| `aarch64-apple-darwin` | 必须 | macOS Runner 原生 `cargo build --release --locked --target aarch64-apple-darwin` | macOS ARM64 Runner | 禁止在 Linux 本机编译 |
| `x86_64-apple-darwin` | 必须 | macOS Runner 原生 `cargo build --release --locked --target x86_64-apple-darwin` | macOS Intel Runner | 禁止在 Linux 本机编译 |
| `aarch64-unknown-linux-musl` | 可选 | cross | 失败不阻塞 | 忽略即可 |
| `aarch64-unknown-linux-gnu` | 可选 | cross | 失败不阻塞 | 忽略即可 |
| `x86_64-unknown-linux-gnu` | 可选 | cross | 失败不阻塞 | 忽略即可 |

- 本机为高版本 GLIBC 的 Linux，Linux 目标一律用 `cross` 交叉编译；切换目标前清理 `target/` 缓存。
- CI 环节必须包含：`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、全层级测试、release 产物校验、产物上传（各平台 `actionlint` 二进制）。

---

## 8. 交付物清单

1. `ROADMAP.md`（本文档）
2. Rust 源码（bin 名 `actionlint`）
3. 单元 / 集成 / 端到端测试
4. 适配目标平台的 CI 配置、`Cargo.toml`、Dependabot 配置
5. 各必须支持目标的编译验证结果