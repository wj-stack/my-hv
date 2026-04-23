//! Build script: configures WDK linker flags via `wdk-build`.

fn main() -> Result<(), wdk_build::ConfigError> {
    wdk_build::configure_wdk_binary_build()
}
