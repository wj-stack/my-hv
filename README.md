# NT 最小驱动模板（cargo-generate）

基于 [windows-drivers-rs](https://github.com/microsoft/windows-drivers-rs) 的 WDM 最小示例：内核驱动 + 用户态客户端 + `shared-contract` IOCTL 常量。适合作为新驱动项目的起点。

## 用 cargo-generate 创建新项目

在本仓库（或已包含 `nt-minimal-template` 与 `windows-drivers-rs` 的目录）下：

```bash
cargo install cargo-generate --locked
cargo generate --path nt-minimal-template --name my-wdm-sample
```

（请勿写成 `cargo generate --path nt-minimal-template my-wdm-sample`：第二个位置参数表示**模板仓库里的子目录**，不是项目名；项目名必须用 `--name`。）

交互时会询问 **`wdk_repo_path`**：从生成项目的 **`driver/`** 目录到 **`windows-drivers-rs` 仓库根** 的相对路径。

默认 **`../../windows-drivers-rs`** 适用于如下布局（新项目与 `windows-drivers-rs` 在同一父目录下）：

```text
<parent>/
  windows-drivers-rs/
  my-wdm-sample/          ← cargo generate 生成在此
    driver/
    client/
    shared-contract/
```

若你把新项目放到其他位置，请按实际目录关系填写该路径，或直接编辑生成后 `driver/Cargo.toml` 中的 `wdk-*` 与 `wdk-build` 的 `path`。

## 生成之后

1. 在生成项目根目录执行 `cargo build -p <你的项目名>-client` 验证用户态工程。
2. 按 [windows-drivers-rs 文档](https://github.com/microsoft/windows-drivers-rs) 配置 WDK / EWDK，再构建驱动（通常配合 `cargo-make` / `Makefile.toml`）。
3. 根据产品信息编辑 `driver/` 下与驱动 `.sys` 同名的 INF 源文件（例如项目名为 `my-wdm-sample` 时，对应 `my_wdm_sample_driver.inx`），补全 Provider、硬件 ID、字符串表等。

## 模板维护说明

本目录作为 **cargo-generate 模板源** 时，`Cargo.toml`、源码等处含有 Liquid 占位符；请勿在未展开占位符的模板根目录直接 `cargo build`。请在 **生成后的新项目目录** 中构建与调试。

INF 模板在仓库中的文件名为 `crate_name` 加后缀 `_driver.inx`（与 `项目名-driver` 包产出的 `*_driver.sys` 一致）；文件名里未使用带竖线 `|` 的 Liquid 过滤器，以便在 Windows 上正常纳入版本控制。
