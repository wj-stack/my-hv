//! 按 `RtlGetVersion().dwBuildNumber` 解析 `HvOffsets` 中除 `system_eprocess` / `cached_system_cr3` 外的字段。
//!
//! 数值来自 [Vergilius Project](https://www.vergiliusproject.com) x64 结构体页面（Windows 10 1903、11 23H2、11 24H2 等）。
//! 新累积更新若改动 `_EPROCESS` / `_KTHREAD`，请用 WinDbg `dt nt!_EPROCESS` 核对后增补区间。

/// 与 [`crate::introspection::HvOffsets`] 中对应字段同义（不含运行时填充的两项）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct OffsetRow {
    pub kprocess_directory_table_base_offset: usize,
    pub kpcr_pcrb_offset: usize,
    pub kprcb_current_thread_offset: usize,
    pub kthread_apc_state_offset: usize,
    pub eprocess_unique_process_id_offset: usize,
    pub eprocess_image_file_name: usize,
    pub kapc_state_process_offset: usize,
}

struct BuildRange {
    min: u32,
    max: u32,
    row: OffsetRow,
}

/// 按 `min` 升序；区间互不重叠。
static RANGES: &[BuildRange] = &[
    // Windows 10 1903 (Vergilius) — 覆盖 19H1/19H2 常见布局（build 18362–19040）。
    BuildRange {
        min: 18362,
        max: 19040,
        row: OffsetRow {
            kprocess_directory_table_base_offset: 0x28,
            kpcr_pcrb_offset: 0x180,
            kprcb_current_thread_offset: 0x8,
            kthread_apc_state_offset: 0x98,
            eprocess_unique_process_id_offset: 0x2e8,
            eprocess_image_file_name: 0x450,
            kapc_state_process_offset: 0x20,
        },
    },
    // Windows 10 20H1 至 Windows 11 23H2 / Server 等同布局（Nickel 等，Vergilius 22H2/23H2 `_EPROCESS`）。
    BuildRange {
        min: 19041,
        max: 26099,
        row: OffsetRow {
            kprocess_directory_table_base_offset: 0x28,
            kpcr_pcrb_offset: 0x180,
            kprcb_current_thread_offset: 0x8,
            kthread_apc_state_offset: 0x98,
            eprocess_unique_process_id_offset: 0x440,
            eprocess_image_file_name: 0x5a8,
            kapc_state_process_offset: 0x20,
        },
    },
    // Windows 11 24H2+（Germanium，Vergilius 24H2 `_EPROCESS` / `_KPROCESS` / `_KTHREAD`）。
    BuildRange {
        min: 26100,
        max: u32::MAX,
        row: OffsetRow {
            kprocess_directory_table_base_offset: 0x28,
            kpcr_pcrb_offset: 0x180,
            kprcb_current_thread_offset: 0x8,
            kthread_apc_state_offset: 0x98,
            eprocess_unique_process_id_offset: 0x1d0,
            eprocess_image_file_name: 0x338,
            kapc_state_process_offset: 0x20,
        },
    },
];

pub(crate) fn lookup(build: u32) -> Option<OffsetRow> {
    for r in RANGES {
        if build >= r.min && build <= r.max {
            return Some(r.row);
        }
    }
    None
}
