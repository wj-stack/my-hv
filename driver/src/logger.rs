//! 诊断日志。对应 `hv/hv/logger.*`；当前使用 `wdk::println!`。

use wdk::println;

pub fn log(msg: &str) {
    println!("[my-hv-driver] {msg}");
}
