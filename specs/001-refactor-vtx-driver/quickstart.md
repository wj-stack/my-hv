# Quickstart: Rust 重构 VT-x 驱动

## Prerequisites

- 已安装并加载 WDK/eWDK 或 VS 驱动构建环境
- 具备 VT-x 的 Windows 10/11 x64 目标机器用于验证

## Build

在仓库根目录执行：

```text
.\build.bat
```

`build.bat` 会负责初始化环境并调用工作区构建流程。

## Minimum Validation

1. 安装并加载生成的驱动。
2. 使用用户态 `client` 调用 `INIT`/`START`/`STOP`/`QUERY`/`SHUTDOWN` 验证基础流程。
3. 在不支持 VT-x 的机器或禁用 VT-x 时，`INIT` 返回可识别错误。
4. 重复启动/停止 20 次，确保无系统崩溃或驱动异常退出。
