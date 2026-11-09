# actionlint-rs

[![Release](https://github.com/zydou/actionlint-rs/actions/workflows/release.yml/badge.svg)](https://github.com/zydou/actionlint-rs/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[rhysd/actionlint](https://github.com/rhysd/actionlint)（GitHub Actions workflow 静态检查器，Go 实现）的 **Rust 移植版**， 产出同名的 `actionlint` 二进制，目标是成为原版的 drop-in
替代（功能对齐，原版Bug 除外）。

> **用法**：与原版完全一致，请查阅[原仓库文档](https://github.com/rhysd/actionlint#usage)
>
> 当前对齐的上游基线为 commit [`011a6d15e749`](https://github.com/rhysd/actionlint/tree/011a6d15e749bb3f2d771eed9c7aa0e7e3e10ee7)。移植规划、能力清单与上游 re-sync 追踪见 [ROADMAP.md](ROADMAP.md)。

## 关于本移植

- **单命令 CLI**：`actionlint [FLAGS] [FILES...] [-]`，无子命令，参数语义与原版逐一对齐。
- **完整规则集**：移植了原版全部检查规则（表达式语义、action 元数据、可复用 workflow、matrix、runner label、事件、权限、凭据、glob、`if` 条件等）。
- **可选外部集成**：`shellcheck`（检查 `run:` 脚本）与 `pyflakes`（检查 Python 脚本）。
- **格式化输出**：支持 `-format` Go template 自定义输出（含 `json` 等模板函数，可渲染 SARIF）。
- **零 `unsafe`**：crate 级 `unsafe_code = "forbid"`，并通过 `clippy` pedantic 严格检查。

## 安装

从 [Releases](https://github.com/zydou/actionlint-rs/releases) 下载对应平台的 `actionlint-<target>.tar.xz`；每次分支推送发布 `nightly`，`v*` 标签推送发布正式版本。

或从源码构建（需最新稳定版 Rust 工具链）：

```bash
cargo build --release --locked
```

## 支持平台

发布 CI（`.github/workflows/release.yml`）在 lint 与全部测试通过后构建以下目标：

| 目标三元组                   | 支持等级 |
| ---------------------------- | -------- |
| `x86_64-unknown-linux-musl`  | 支持（静态链接） |
| `aarch64-apple-darwin`       | 支持（Apple Silicon，原生构建） |
| `x86_64-apple-darwin`        | 支持（Intel，原生构建） |
| `aarch64-unknown-linux-musl` | 支持（静态链接） |
| `aarch64-unknown-linux-gnu`  | 支持（GLIBC >= 2.18） |
| `x86_64-unknown-linux-gnu`   | 支持（GLIBC >= 2.18） |

Linux 目标通过 [cross-rs](https://github.com/cross-rs/cross) 构建；macOS 目标仅在 macOS runner 上原生构建。

## 开发

```bash
cargo fmt --all -- --check                                          # 格式检查
cargo clippy --all-targets --all-features --locked -- -D warnings   # 静态检查
cargo test --locked                                                 # 测试
cargo build --release --locked                                      # 构建
```

## 许可证

[MIT](LICENSE)
