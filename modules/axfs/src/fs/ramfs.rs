//! RAM filesystem implementation based on axfs-ng-vfs
//!
//! This module provides an in-memory filesystem that implements the
//! axfs-ng-vfs traits (FilesystemOps, FileNodeOps, DirNodeOps).

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{any::Any, sync::atomic};
use axfs_ng_vfs::{
    DirEntry, DirEntrySink, DirNode, FileNode, FileNodeOps, FilesystemOps, Metadata, MetadataUpdate,
    NodeOps, NodePermission, NodeType, VfsError, VfsResult, Reference, DirNodeOps,
};
use axpoll::Pollable;
use spin::Mutex;
use core::time::Duration;
use spin::RwLock;

/// RAM filesystem that implements FilesystemOps
pub struct RamFileSystem {
    root: Arc<RamDirNode>,
    device: u64,
}

static DEVICE_COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(1);

impl RamFileSystem {
    /// Create a new RAM filesystem
    pub fn new() -> Self {
        let device = DEVICE_COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
        let root = Arc::new_cyclic(|weak_self| RamDirNode::new_root(weak_self.clone(), device));
        Self { root, device }
    }

    /// Get the root directory entry
    pub fn root_dir_entry(&self) -> DirEntry {
        use axfs_ng_vfs::node::dir::DirNode;
        DirEntry::new_dir(
            |_weak| DirNode::new(self.root.clone()),
            Reference::new(None, String::from("/")),
        )
    }
}

impl Default for RamFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FilesystemOps for RamFileSystem {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn root_dir(&self) -> DirEntry {
        self.root_dir_entry()
    }

    fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> {
        Ok(axfs_ng_vfs::StatFs {
            fs_type: 0x858458f6, // RAMFS_MAGIC
            block_size: 4096,
            blocks: 0,
            blocks_free: 0,
            blocks_available: 0,
            file_count: self.root.child_count() as u64,
            free_file_count: 0,
            name_length: 255,
            fragment_size: 4096,
            mount_flags: 0,
        })
    }

    fn flush(&self) -> VfsResult<()> {
        // No-op for RAM filesystem
        Ok(())
    }
}

/// File node in RAM filesystem
pub struct RamFileNode {
    content: Mutex<Vec<u8>>,
    inode: u64,
    parent: Weak<dyn NodeOps>,
    node_type: NodeType,
    mode: NodePermission,
    device: u64,
    atime: Mutex<Duration>,
    mtime: Mutex<Duration>,
    ctime: Mutex<Duration>,
}

impl RamFileNode {
    fn new(inode: u64, parent: Weak<dyn NodeOps>, device: u64) -> Self {
        Self {
            content: Mutex::new(Vec::new()),
            inode,
            parent,
            node_type: NodeType::RegularFile,
            mode: NodePermission::from_bits_truncate(0o644),
            device,
            atime: Mutex::new(Duration::ZERO),
            mtime: Mutex::new(Duration::ZERO),
            ctime: Mutex::new(Duration::ZERO),
        }
    }
}

impl NodeOps for RamFileNode {
    fn inode(&self) -> u64 {
        self.inode
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        let content = self.content.lock();
        Ok(Metadata {
            device: self.device,
            inode: self.inode,
            nlink: 1,
            mode: self.mode,
            node_type: self.node_type,
            uid: 0,
            gid: 0,
            size: content.len() as u64,
            block_size: 4096,
            blocks: (content.len() as u64 + 4095) / 4096,
            rdev: axfs_ng_vfs::DeviceId::new(0, 0),
            atime: *self.atime.lock(),
            mtime: *self.mtime.lock(),
            ctime: *self.ctime.lock(),
        })
    }

    fn update_metadata(&self, update: MetadataUpdate) -> VfsResult<()> {
        // RamFileNode has immutable metadata for now
        // This is a simplified implementation
        if let Some(_mode) = update.mode {
            // Cannot update mode in this implementation
        }
        if let Some(atime) = update.atime {
            *self.atime.lock() = atime;
        }
        if let Some(mtime) = update.mtime {
            *self.mtime.lock() = mtime;
        }
        Ok(())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        // For now, we don't have a proper reference to the filesystem
        // Return a static placeholder
        // TODO: Redesign to maintain a proper filesystem reference
        struct DummyFs;
        impl FilesystemOps for DummyFs {
            fn name(&self) -> &str { "ramfs" }
            fn root_dir(&self) -> DirEntry { panic!("Should not be called") }
            fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> {
                Ok(axfs_ng_vfs::StatFs {
                    fs_type: 0x858458f6, // RAMFS_MAGIC
                    block_size: 4096,
                    blocks: 0,
                    blocks_free: 0,
                    blocks_available: 0,
                    file_count: 0,
                    free_file_count: 0,
                    name_length: 255,
                    fragment_size: 4096,
                    mount_flags: 0,
                })
            }
            fn flush(&self) -> VfsResult<()> { Ok(()) }
        }
        static DUMMY_FS: DummyFs = DummyFs;
        &DUMMY_FS
    }

    fn len(&self) -> VfsResult<u64> {
        Ok(self.content.lock().len() as u64)
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        // No-op for RAM filesystem
        Ok(())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl FileNodeOps for RamFileNode {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        let content = self.content.lock();
        let offset = offset as usize;
        if offset >= content.len() {
            return Ok(0);
        }
        let end = (offset + buf.len()).min(content.len());
        let src = &content[offset..end];
        buf[..src.len()].copy_from_slice(src);
        Ok(src.len())
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> VfsResult<usize> {
        let offset = offset as usize;
        let mut content = self.content.lock();
        if offset + buf.len() > content.len() {
            content.resize(offset + buf.len(), 0);
        }
        let dst = &mut content[offset..offset + buf.len()];
        dst.copy_from_slice(buf);
        *self.mtime.lock() = Duration::from_secs(0);
        Ok(buf.len())
    }

    fn append(&self, buf: &[u8]) -> VfsResult<(usize, u64)> {
        let mut content = self.content.lock();
        let offset = content.len();
        content.extend_from_slice(buf);
        *self.mtime.lock() = Duration::from_secs(0);
        Ok((buf.len(), content.len() as u64))
    }

    fn set_len(&self, len: u64) -> VfsResult<()> {
        let mut content = self.content.lock();
        if len as usize > content.len() {
            content.resize(len as usize, 0);
        } else {
            content.truncate(len as usize);
        }
        *self.mtime.lock() = Duration::from_secs(0);
        Ok(())
    }

    fn set_symlink(&self, _target: &str) -> VfsResult<()> {
        Err(VfsError::InvalidInput)
    }

    fn ioctl(&self, _cmd: u32, _arg: usize) -> VfsResult<usize> {
        Err(VfsError::NotATty)
    }
}

impl Pollable for RamFileNode {
    fn poll(&self) -> axpoll::IoEvents {
        axpoll::IoEvents::IN | axpoll::IoEvents::OUT
    }

    fn register(&self, _context: &mut core::task::Context<'_>, _events: axpoll::IoEvents) {
        // No-op for RAM file
    }
}

/// Directory node in RAM filesystem
pub struct RamDirNode {
    this: Weak<Self>,
    parent: RwLock<Weak<dyn NodeOps>>,
    children: RwLock<BTreeMap<String, DirEntry>>,
    inode: u64,
    device: u64,
    mode: NodePermission,
    atime: Mutex<Duration>,
    mtime: Mutex<Duration>,
    ctime: Mutex<Duration>,
}

impl Clone for RamDirNode {
    fn clone(&self) -> Self {
        Self {
            this: Weak::new(),
            parent: RwLock::new(self.parent.read().clone()),
            children: RwLock::new(self.children.read().clone()),
            inode: self.inode,
            device: self.device,
            mode: self.mode,
            atime: Mutex::new(*self.atime.lock()),
            mtime: Mutex::new(*self.mtime.lock()),
            ctime: Mutex::new(*self.ctime.lock()),
        }
    }
}

impl RamDirNode {
    fn new_root(this: Weak<Self>, device: u64) -> Self {
        Self {
            this,
            parent: RwLock::new(Weak::<RamDirNode>::new()),
            children: RwLock::new(BTreeMap::new()),
            inode: 1,
            device,
            mode: NodePermission::from_bits_truncate(0o755),
            atime: Mutex::new(Duration::ZERO),
            mtime: Mutex::new(Duration::ZERO),
            ctime: Mutex::new(Duration::ZERO),
        }
    }

    fn new(
        this: Weak<Self>,
        parent: Weak<dyn NodeOps>,
        inode: u64,
        device: u64,
    ) -> Self {
        Self {
            this,
            parent: RwLock::new(parent),
            children: RwLock::new(BTreeMap::new()),
            inode,
            device,
            mode: NodePermission::from_bits_truncate(0o755),
            atime: Mutex::new(Duration::ZERO),
            mtime: Mutex::new(Duration::ZERO),
            ctime: Mutex::new(Duration::ZERO),
        }
    }

    fn child_count(&self) -> usize {
        self.children.read().len()
    }

    fn split_path(path: &str) -> (&str, Option<&str>) {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return ("", None);
        }
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        if parts.len() == 1 {
            (parts[0], None)
        } else {
            (parts[0], Some(parts[1]))
        }
    }
}

impl NodeOps for RamDirNode {
    fn inode(&self) -> u64 {
        self.inode
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        let children = self.children.read();
        let child_count = children.len() as u64;
        Ok(Metadata {
            device: self.device,
            inode: self.inode,
            nlink: child_count + 2, // children + . + ..
            mode: self.mode,
            node_type: NodeType::Directory,
            uid: 0,
            gid: 0,
            size: 4096,
            block_size: 4096,
            blocks: 1,
            rdev: axfs_ng_vfs::DeviceId::new(0, 0),
            atime: *self.atime.lock(),
            mtime: *self.mtime.lock(),
            ctime: *self.ctime.lock(),
        })
    }

    fn update_metadata(&self, update: MetadataUpdate) -> VfsResult<()> {
        // RamDirNode has immutable mode for now
        // This is a simplified implementation
        if let Some(_mode) = update.mode {
            // Cannot update mode in this implementation
        }
        if let Some(atime) = update.atime {
            *self.atime.lock() = atime;
        }
        if let Some(mtime) = update.mtime {
            *self.mtime.lock() = mtime;
        }
        Ok(())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        // For now, we don't have a proper reference to the filesystem
        // Return a static placeholder
        // TODO: Redesign to maintain a proper filesystem reference
        struct DummyFs;
        impl FilesystemOps for DummyFs {
            fn name(&self) -> &str { "ramfs" }
            fn root_dir(&self) -> DirEntry { panic!("Should not be called") }
            fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> {
                Ok(axfs_ng_vfs::StatFs {
                    fs_type: 0x858458f6, // RAMFS_MAGIC
                    block_size: 4096,
                    blocks: 0,
                    blocks_free: 0,
                    blocks_available: 0,
                    file_count: 0,
                    free_file_count: 0,
                    name_length: 255,
                    fragment_size: 4096,
                    mount_flags: 0,
                })
            }
            fn flush(&self) -> VfsResult<()> { Ok(()) }
        }
        static DUMMY_FS: DummyFs = DummyFs;
        &DUMMY_FS
    }

    fn len(&self) -> VfsResult<u64> {
        Ok(4096)
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        Ok(())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl axfs_ng_vfs::DirNodeOps for RamDirNode {
    fn read_dir(&self, offset: u64, sink: &mut dyn DirEntrySink) -> VfsResult<usize> {
        let children = self.children.read();
        let mut count = 0;

        // Handle . and ..
        match offset {
            0 => {
                if !sink.accept(".", self.inode, NodeType::Directory, offset + 1) {
                    return Ok(count);
                }
                count += 1;
            }
            1 => {
                if !sink.accept("..", self.inode, NodeType::Directory, offset + 1) {
                    return Ok(count);
                }
                count += 1;
            }
            _ => {}
        }

        // Handle children
        let start_index = if offset < 2 { 0 } else { (offset - 2) as usize };
        for (i, (name, entry)) in children.iter().enumerate().skip(start_index) {
            if !sink.accept(name, entry.inode(), entry.node_type(), offset + 1 + i as u64) {
                return Ok(count);
            }
            count += 1;
        }

        Ok(count)
    }

    fn lookup(&self, name: &str) -> VfsResult<DirEntry> {
        use axfs_ng_vfs::node::dir::DirNode;
        match name {
            "" | "." => {
                let this = self.this.upgrade().ok_or(VfsError::NotFound)?;
                Ok(DirEntry::new_dir(
                    |_weak| DirNode::new(this.clone()),
                    Reference::new(None, String::from(".")),
                ))
            }
            ".." => {
                let parent = self.parent.read().upgrade().ok_or(VfsError::NotFound)?;
                // Convert parent to DirNodeOps by downcasting through Any
                use axfs_ng_vfs::node::dir::DirNode;
                let parent_any = parent.clone().into_any();
                if let Ok(dir_ops) = parent_any.downcast::<RamDirNode>() {
                    Ok(DirEntry::new_dir(
                        |_weak| DirNode::new(dir_ops),
                        Reference::new(None, String::from("..")),
                    ))
                } else {
                    // Parent is not a RamDirNode (shouldn't happen in ramfs)
                    Err(VfsError::NotADirectory)
                }
            }
            _ => {
                let children = self.children.read();
                children
                    .get(name)
                    .cloned()
                    .ok_or(VfsError::NotFound)
            }
        }
    }

    fn is_cacheable(&self) -> bool {
        true
    }

    fn create(
        &self,
        name: &str,
        node_type: NodeType,
        permission: NodePermission,
    ) -> VfsResult<DirEntry> {
        if name.is_empty() || name == "." || name == ".." {
            return Ok(self.lookup(name)?);
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(VfsError::AlreadyExists);
        }

        let this = self.this.upgrade().ok_or(VfsError::NotFound)?;
        static INODE_COUNTER: atomic::AtomicU64 = atomic::AtomicU64::new(2);

        let entry = match node_type {
            NodeType::RegularFile => {
                let inode = INODE_COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
                let file = Arc::new(RamFileNode::new(
                    inode,
                    Arc::downgrade(&this) as Weak<dyn NodeOps>,
                    self.device,
                ));
                let file_node = FileNode::new(file);
                DirEntry::new_file(
                    file_node,
                    NodeType::RegularFile,
                    Reference::new(None, name.to_string()),
                )
            }
            NodeType::Directory => {
                let inode = INODE_COUNTER.fetch_add(1, atomic::Ordering::Relaxed);
                let dir = Arc::new_cyclic(|weak_self| {
                    RamDirNode::new(
                        weak_self.clone(),
                        Arc::downgrade(&this) as Weak<dyn NodeOps>,
                        inode,
                        self.device,
                    )
                });
                DirEntry::new_dir(
                    |_weak| {
                        use axfs_ng_vfs::node::dir::DirNode;
                        DirNode::new(dir.clone())
                    },
                    Reference::new(None, name.to_string()),
                )
            }
            _ => return Err(VfsError::Unsupported),
        };

        children.insert(name.to_string(), entry.clone());
        *self.mtime.lock() = Duration::from_secs(0);
        Ok(entry)
    }

    fn link(&self, name: &str, node: &DirEntry) -> VfsResult<DirEntry> {
        if name.is_empty() || name == "." || name == ".." {
            return Err(VfsError::InvalidInput);
        }

        let mut children = self.children.write();
        if children.contains_key(name) {
            return Err(VfsError::AlreadyExists);
        }

        children.insert(name.to_string(), node.clone());
        *self.mtime.lock() = Duration::from_secs(0);
        Ok(node.clone())
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        if name == "." || name == ".." {
            return Err(VfsError::InvalidInput);
        }

        let mut children = self.children.write();
        let entry = children.get(name).ok_or(VfsError::NotFound)?;

        // Check if directory is not empty
        if entry.node_type() == NodeType::Directory {
            if let Ok(dir) = entry.downcast::<RamDirNode>() {
                if !dir.children.read().is_empty() {
                    return Err(VfsError::InvalidInput);
                }
            }
        }

        children.remove(name);
        *self.mtime.lock() = Duration::from_secs(0);
        Ok(())
    }

    fn rename(
        &self,
        src_name: &str,
        dst_dir: &axfs_ng_vfs::DirNode,
        dst_name: &str,
    ) -> VfsResult<()> {
        if src_name == "." || src_name == ".." {
            return Err(VfsError::InvalidInput);
        }

        // Get source entry
        let src_entry = self.lookup(src_name)?;

        // If src and dst are the same directory and same name, do nothing
        if self.inode() == dst_dir.inode() && src_name == dst_name {
            return Ok(());
        }

        // Check if destination directory is not empty (when moving a directory)
        if src_entry.node_type() == NodeType::Directory {
            if let Ok(dst_entry) = dst_dir.lookup(dst_name) {
                if dst_entry.node_type() == NodeType::Directory {
                    if let Ok(dst_dir_node) = dst_entry.downcast::<RamDirNode>() {
                        if !dst_dir_node.children.read().is_empty() {
                            return Err(VfsError::InvalidInput);
                        }
                    }
                } else {
                    return Err(VfsError::IsADirectory);
                }
            }
        } else if let Ok(dst_entry) = dst_dir.lookup(dst_name) {
            // Source is a file, destination must not be a directory
            if dst_entry.node_type() == NodeType::Directory {
                return Err(VfsError::IsADirectory);
            }
        }

        // Remove source
        self.unlink(src_name)?;

        // Add to destination
        if let Ok(dst_dir_node) = dst_dir.downcast::<RamDirNode>() {
            let mut children = dst_dir_node.children.write();
            children.insert(dst_name.to_string(), src_entry);
            *dst_dir_node.mtime.lock() = Duration::from_secs(0);
        }

        Ok(())
    }
}

/// Create a new RAM filesystem
pub fn new_ramfs() -> Arc<RamFileSystem> {
    Arc::new(RamFileSystem::new())
}
