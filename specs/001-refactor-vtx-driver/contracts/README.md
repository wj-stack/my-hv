# Contracts Overview

本目录定义 Rust 重构驱动的对外接口契约。接口为全新设计，不要求兼容旧驱动的设备名或 IOCTL 语义。契约内容必须与 `shared-contract` 中的常量与版本保持一致。

## Contract Set

- `driver-interface.md`: 设备命名、控制请求语义、响应格式与错误范围。

## Versioning

每次接口形态变化必须更新契约版本，并同步到 `shared-contract` 的 `CONTRACT_VERSION`。
