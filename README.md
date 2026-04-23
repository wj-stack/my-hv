# my-hv

## 本仓库 VT-x 驱动与 `my-hv-client`

在已配置 WDK/eWDK 的机器上，于仓库根目录执行 `.\build.bat` 完成驱动签名包与用户态客户端构建。

安装并启动驱动后，可使用 `my-hv-client`（设备路径默认 `\\.\MyHvTpl`，与 `shared_contract::USER_DEVICE_PATH` 一致）：

```text
my-hv-client ping
my-hv-client start
my-hv-client hv-ping
my-hv-client stop
```

