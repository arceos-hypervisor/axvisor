use alloc::sync::Arc;
use axerrno::{AxError, AxResult};
use axfs_ng_vfs::{NodeType, VfsResult};

use crate::fs;

pub(crate) fn ramfs() -> Arc<fs::RamFileSystem> {
    Arc::new(fs::RamFileSystem::new())
}

pub(crate) fn procfs() -> AxResult<Arc<fs::RamFileSystem>> {
    let procfs = fs::RamFileSystem::new();
    let proc_root_entry = procfs.root_dir_entry();

    // Create /proc/sys/net/core/somaxconn
    let proc_root = proc_root_entry.as_dir()?;
    proc_root.create("sys", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    proc_root.create("sys/net", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    proc_root.create("sys/net/core", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    proc_root.create("sys/net/core/somaxconn", NodeType::RegularFile, axfs_ng_vfs::NodePermission::from_bits_truncate(0o644))?;
    let entry_somaxconn = proc_root.lookup("sys/net/core/somaxconn")?;
    let file_somaxconn = entry_somaxconn.as_file()?;
    file_somaxconn.write_at(b"4096\n", 0)?;

    // Create /proc/sys/vm/overcommit_memory
    proc_root.create("sys/vm", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    proc_root.create("sys/vm/overcommit_memory", NodeType::RegularFile, axfs_ng_vfs::NodePermission::from_bits_truncate(0o644))?;
    let entry_over = proc_root.lookup("sys/vm/overcommit_memory")?;
    let file_over = entry_over.as_file()?;
    file_over.write_at(b"0\n", 0)?;

    // Create /proc/self/stat
    proc_root.create("self", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    proc_root.create("self/stat", NodeType::RegularFile, axfs_ng_vfs::NodePermission::from_bits_truncate(0o644))?;

    Ok(Arc::new(procfs))
}

pub(crate) fn sysfs() -> AxResult<Arc<fs::RamFileSystem>> {
    let sysfs = fs::RamFileSystem::new();
    let sys_root_entry = sysfs.root_dir_entry();
    let sys_root = sys_root_entry.as_dir()?;

    // Create /sys/kernel/mm/transparent_hugepage/enabled
    sys_root.create("kernel", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("kernel/mm", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("kernel/mm/transparent_hugepage", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("kernel/mm/transparent_hugepage/enabled", NodeType::RegularFile, axfs_ng_vfs::NodePermission::from_bits_truncate(0o644))?;
    let entry_hp = sys_root.lookup("kernel/mm/transparent_hugepage/enabled")?;
    let file_hp = entry_hp.as_file()?;
    file_hp.write_at(b"always [madvise] never\n", 0)?;

    // Create /sys/devices/system/clocksource/clocksource0/current_clocksource
    sys_root.create("devices", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("devices/system", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("devices/system/clocksource", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create("devices/system/clocksource/clocksource0", NodeType::Directory, axfs_ng_vfs::NodePermission::from_bits_truncate(0o755))?;
    sys_root.create(
        "devices/system/clocksource/clocksource0/current_clocksource",
        NodeType::RegularFile,
        axfs_ng_vfs::NodePermission::from_bits_truncate(0o644),
    )?;
    let entry_cc = sys_root.lookup("devices/system/clocksource/clocksource0/current_clocksource")?;
    let file_cc = entry_cc.as_file()?;
    file_cc.write_at(b"tsc\n", 0)?;

    Ok(Arc::new(sysfs))
}
