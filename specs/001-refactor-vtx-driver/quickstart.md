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
2. 使用用户态 `my-hv-client`（见根目录 `README.md` 示例）：`ping` → `start` → `hv-ping` → `stop`（或 `hv-unload`）。
3. 在不支持 VT-x 的机器或禁用 VT-x 时，`start`（`IOCTL_HV_START`）应返回失败状态（例如 `STATUS_NOT_SUPPORTED`）。
4. 重复 `start` / `stop` 多次，确保无系统崩溃或驱动异常退出。

## Build validation note

- 2026-04-23：在本机 eWDK 环境下执行仓库根目录 `.\build.bat`，驱动包 `my_hv_driver_package` 与用户态 `my-hv-client` 均构建成功。
