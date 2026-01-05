use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use axfs_ng_vfs::{
    DirEntry, DirEntrySink, DirNode, FileNode, FileNodeOps, FilesystemOps, Metadata, MetadataUpdate,
    NodeOps, NodePermission, NodeType, VfsError, VfsResult, Reference, DirNodeOps, NodeFlags,
};
use axpoll::Pollable;
use rsext4::{
    Ext4FileSystem as Rsext4FileSystem, Jbd2Dev, OpenFile,
    ext4_backend::disknode::Ext4Inode as Inode,
    ext4_backend::api::{fs_mount, lseek, open, read_at},
    ext4_backend::dir::{get_inode_with_num, mkdir},
    ext4_backend::entries::classic_dir::list_entries,
    ext4_backend::file::{delete_dir, mkfile, mv, truncate, unlink, write_file},
    ext4_backend::loopfile::resolve_inode_block_allextend,
};
use spin::Mutex;
use core::{any::Any, time::Duration};

use crate::dev::{Disk, Partition};
pub const BLOCK_SIZE: usize = 4096;

/// Wrapper to convert FileWrapper to Arc<dyn DirNodeOps>
pub struct FileWrapperAsDirOps(Arc<FileWrapper>);

impl NodeOps for FileWrapperAsDirOps {
    fn inode(&self) -> u64 { self.0.inode() }
    fn metadata(&self) -> VfsResult<Metadata> { self.0.metadata() }
    fn update_metadata(&self, update: MetadataUpdate) -> VfsResult<()> { self.0.update_metadata(update) }
    fn len(&self) -> VfsResult<u64> { self.0.len() }
    fn sync(&self, data_only: bool) -> VfsResult<()> { self.0.sync(data_only) }
    fn flags(&self) -> NodeFlags { self.0.flags() }
    fn filesystem(&self) -> &dyn FilesystemOps { self.0.filesystem() }
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        let wrapper: Arc<FileWrapper> = unsafe { core::mem::transmute(self) };
        wrapper
    }
}

impl DirNodeOps for FileWrapperAsDirOps {
    fn read_dir(&self, offset: u64, sink: &mut dyn DirEntrySink) -> VfsResult<usize> {
        self.0.read_dir(offset, sink)
    }
    fn lookup(&self, name: &str) -> VfsResult<DirEntry> {
        self.0.lookup(name)
    }
    fn is_cacheable(&self) -> bool {
        self.0.is_cacheable()
    }
    fn create(&self, name: &str, node_type: NodeType, permission: NodePermission) -> VfsResult<DirEntry> {
        self.0.create(name, node_type, permission)
    }
    fn link(&self, name: &str, node: &DirEntry) -> VfsResult<DirEntry> {
        self.0.link(name, node)
    }
    fn unlink(&self, name: &str) -> VfsResult<()> {
        self.0.unlink(name)
    }
    fn rename(&self, src_name: &str, dst_dir: &DirNode, dst_name: &str) -> VfsResult<()> {
        self.0.rename(src_name, dst_dir, dst_name)
    }
}

impl Pollable for FileWrapperAsDirOps {
    fn poll(&self) -> axpoll::IoEvents { self.0.poll() }
    fn register(&self, context: &mut core::task::Context<'_>, events: axpoll::IoEvents) {
        self.0.register(context, events)
    }
}

#[allow(dead_code)]
pub struct Ext4FileSystem {
    inner: Arc<Mutex<Jbd2Dev<Disk>>>,
    fs: Arc<Mutex<Rsext4FileSystem>>,
}

/// Ext4FileSystem that works with a partition
pub struct Ext4FileSystemPartition {
    inner: Arc<Mutex<Jbd2Dev<Partition>>>,
    fs: Arc<Mutex<Rsext4FileSystem>>,
}

unsafe impl Sync for Ext4FileSystem {}
unsafe impl Send for Ext4FileSystem {}

unsafe impl Sync for Ext4FileSystemPartition {}
unsafe impl Send for Ext4FileSystemPartition {}

impl Ext4FileSystem {
    #[cfg(feature = "use-ramdisk")]
    #[allow(dead_code)]
    pub fn new(mut disk: Disk) -> Self {
        unimplemented!()
    }

    #[cfg(not(feature = "use-ramdisk"))]
    #[allow(dead_code)]
    pub fn new(disk: Disk) -> Self {
        info!(
            "Got Disk size:{}, position:{}",
            disk.size(),
            disk.position()
        );
        let mut inner = Jbd2Dev::initial_jbd2dev(0, disk, false);
        let fs = fs_mount(&mut inner).expect("failed to initialize EXT4 filesystem");
        Self {
            inner: Arc::new(Mutex::new(inner)),
            fs: Arc::new(Mutex::new(fs)),
        }
    }

    /// Create a new ext4 filesystem from a partition
    pub fn from_partition(partition: Partition) -> Ext4FileSystemPartition {
        info!(
            "Got Partition size:{}, position:{}",
            partition.size(),
            partition.position()
        );
        let mut inner = Jbd2Dev::initial_jbd2dev(0, partition, false);
        let fs = fs_mount(&mut inner).expect("failed to initialize EXT4 filesystem on partition");
        Ext4FileSystemPartition {
            inner: Arc::new(Mutex::new(inner)),
            fs: Arc::new(Mutex::new(fs)),
        }
    }
}

/// The [`FilesystemOps`] trait provides operations on a filesystem.
impl FilesystemOps for Ext4FileSystem {
    fn name(&self) -> &str {
        "ext4"
    }

    fn root_dir(&self) -> DirEntry {
        debug!("Get root_dir");
        let file_wrapper = Arc::new(FileWrapper::new(
            "/",
            Ext4Inner::Disk(Arc::clone(&self.inner)),
            Arc::clone(&self.fs),
        ));
        // FileWrapper implements both FileNodeOps and DirNodeOps
        // Use new_dir for root directory
        DirEntry::new_dir(
            |_weak| {
                axfs_ng_vfs::node::dir::DirNode::new(file_wrapper.clone())
            },
            Reference::new(None, String::from("/")),
        )
    }

    fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> {
        let fs = self.fs.lock();
        Ok(axfs_ng_vfs::StatFs {
            fs_type: 0xEF53, // EXT4_SUPER_MAGIC
            block_size: BLOCK_SIZE as u32,
            blocks: fs.superblock.blocks_count(),
            blocks_free: fs.superblock.free_blocks_count(),
            blocks_available: fs.superblock.free_blocks_count(),
            file_count: fs.superblock.s_inodes_count as u64,
            free_file_count: fs.superblock.s_free_inodes_count as u64,
            name_length: 255,
            fragment_size: BLOCK_SIZE as u32,
            mount_flags: 0,
        })
    }

    fn flush(&self) -> VfsResult<()> {
        // TODO: implement filesystem flush
        Ok(())
    }
}

/// The [`FilesystemOps`] trait provides operations on a filesystem.
impl FilesystemOps for Ext4FileSystemPartition {
    fn name(&self) -> &str {
        "ext4"
    }

    fn root_dir(&self) -> DirEntry {
        debug!("Get root_dir");
        let file_wrapper = Arc::new(FileWrapper::new(
            "/",
            Ext4Inner::Partition(Arc::clone(&self.inner)),
            Arc::clone(&self.fs),
        ));
        DirEntry::new_dir(
            |_weak| {
                axfs_ng_vfs::node::dir::DirNode::new(file_wrapper.clone())
            },
            Reference::new(None, String::from("/")),
        )
    }

    fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> {
        let fs = self.fs.lock();
        Ok(axfs_ng_vfs::StatFs {
            fs_type: 0xEF53, // EXT4_SUPER_MAGIC
            block_size: BLOCK_SIZE as u32,
            blocks: fs.superblock.blocks_count(),
            blocks_free: fs.superblock.free_blocks_count(),
            blocks_available: fs.superblock.free_blocks_count(),
            file_count: fs.superblock.s_inodes_count as u64,
            free_file_count: fs.superblock.s_free_inodes_count as u64,
            name_length: 255,
            fragment_size: BLOCK_SIZE as u32,
            mount_flags: 0,
        })
    }

    fn flush(&self) -> VfsResult<()> {
        // TODO: implement filesystem flush
        Ok(())
    }
}

#[derive(Clone)]
pub enum Ext4Inner {
    Disk(Arc<Mutex<Jbd2Dev<Disk>>>),
    Partition(Arc<Mutex<Jbd2Dev<Partition>>>),
}

pub struct FileWrapper {
    path: String,
    file: Mutex<Option<OpenFile>>,
    inner: Ext4Inner,
    fs: Arc<Mutex<Rsext4FileSystem>>,
}

unsafe impl Send for FileWrapper {}
unsafe impl Sync for FileWrapper {}

impl Clone for FileWrapper {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            file: Mutex::new(None),
            inner: self.inner.clone(),
            fs: Arc::clone(&self.fs),
        }
    }
}

impl FileWrapper {
    fn new(path: &str, inner: Ext4Inner, fs: Arc<Mutex<Rsext4FileSystem>>) -> Self {
        debug!("FileWrapper new {}", path);
        Self {
            path: path.to_string(),
            file: Mutex::new(None),
            inner,
            fs,
        }
    }

    fn path_deal_with(&self, path: &str) -> String {
        if path.starts_with('/') {
            debug!("path_deal_with: {}", path);
        }
        let trim_path = path.trim_matches('/');
        if trim_path.is_empty() || trim_path == "." {
            return self.path.to_string();
        }

        if let Some(rest) = trim_path.strip_prefix("./") {
            //if starts with "./"
            return self.path_deal_with(rest);
        }
        let rest_p = trim_path.replace("//", "/");
        if trim_path != rest_p {
            return self.path_deal_with(&rest_p);
        }

        let base_path = self.path.trim_end_matches('/');
        if base_path == "/" {
            format!("/{}", trim_path)
        } else {
            format!("{}/{}", base_path, trim_path)
        }
    }

    fn get_inode(&self) -> VfsResult<Inode> {
        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &self.path)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)
                    .map(|(_, inode)| inode)
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &self.path)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)
                    .map(|(_, inode)| inode)
            }
        }
    }
}

/// The [`NodeOps`] trait provides operations on a node.
impl NodeOps for FileWrapper {
    fn inode(&self) -> u64 {
        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                match get_inode_with_num(&mut *fs, &mut *inner, &self.path) {
                    Ok(Some((inode_num, _))) => inode_num as u64,
                    _ => 0,
                }
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                match get_inode_with_num(&mut *fs, &mut *inner, &self.path) {
                    Ok(Some((inode_num, _))) => inode_num as u64,
                    _ => 0,
                }
            }
        }
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        let inode = self.get_inode()?;
        let perm = NodePermission::from_bits_truncate(0o755);
        let vtype = if inode.is_dir() {
            NodeType::Directory
        } else {
            NodeType::RegularFile
        };
        let size = inode.size() as u64;
        let blocks = inode.blocks_count() as u64;

        trace!(
            "metadata of {:?}, size: {}, blocks: {}",
            self.path, size, blocks
        );

        Ok(Metadata {
            device: 0,
            inode: self.inode(),
            nlink: inode.i_links_count as u64,
            mode: perm,
            node_type: vtype,
            uid: 0,
            gid: 0,
            size,
            block_size: BLOCK_SIZE as u64,
            blocks,
            rdev: axfs_ng_vfs::DeviceId::new(0, 0),
            atime: Duration::ZERO,
            mtime: Duration::ZERO,
            ctime: Duration::ZERO,
        })
    }

    fn update_metadata(&self, _update: MetadataUpdate) -> VfsResult<()> {
        // TODO: implement metadata update
        Ok(())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        // For now, we don't have a proper reference to the filesystem
        // Return a static placeholder
        // TODO: Redesign to maintain a proper filesystem reference
        struct DummyFs;
        impl FilesystemOps for DummyFs {
            fn name(&self) -> &str { "ext4" }
            fn root_dir(&self) -> DirEntry { panic!("Should not be called") }
            fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> { Err(VfsError::Unsupported) }
            fn flush(&self) -> VfsResult<()> { Ok(()) }
        }
        static DUMMY_FS: DummyFs = DummyFs;
        &DUMMY_FS
    }

    fn len(&self) -> VfsResult<u64> {
        let inode = self.get_inode()?;
        Ok(inode.size() as u64)
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        // TODO: implement sync
        Ok(())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// The [`FileNodeOps`] trait provides file operations.
impl FileNodeOps for FileWrapper {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        let mut file_guard = self.file.lock();
        if file_guard.is_none() {
            let mut fs = self.fs.lock();
            *file_guard = match self.inner {
                Ext4Inner::Disk(ref inner) => {
                    let mut inner = inner.lock();
                    open(&mut *inner, &mut *fs, &self.path, false).ok()
                }
                Ext4Inner::Partition(ref inner) => {
                    let mut inner = inner.lock();
                    open(&mut *inner, &mut *fs, &self.path, false).ok()
                }
            };
        }

        if let Some(ref mut file) = *file_guard {
            let mut fs = self.fs.lock();
            lseek(file, offset);
            let data = match self.inner {
                Ext4Inner::Disk(ref inner) => {
                    let mut inner = inner.lock();
                    read_at(&mut *inner, &mut *fs, file, buf.len()).map_err(|_| VfsError::Io)?
                }
                Ext4Inner::Partition(ref inner) => {
                    let mut inner = inner.lock();
                    read_at(&mut *inner, &mut *fs, file, buf.len()).map_err(|_| VfsError::Io)?
                }
            };
            let len = data.len().min(buf.len());
            buf[..len].copy_from_slice(&data[..len]);
            Ok(len)
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> VfsResult<usize> {
        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                write_file(&mut *inner, &mut *fs, &self.path, offset, buf)
                    .map_err(|_| VfsError::Io)?;
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                write_file(&mut *inner, &mut *fs, &self.path, offset, buf)
                    .map_err(|_| VfsError::Io)?;
            }
        };
        Ok(buf.len())
    }

    fn append(&self, buf: &[u8]) -> VfsResult<(usize, u64)> {
        let inode = self.get_inode()?;
        let offset = inode.size() as u64;
        self.write_at(buf, offset)?;
        Ok((buf.len(), offset + buf.len() as u64))
    }

    fn set_len(&self, size: u64) -> VfsResult<()> {
        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                let _ = truncate(&mut *inner, &mut *fs, &self.path, size);
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                let _ = truncate(&mut *inner, &mut *fs, &self.path, size);
            }
        }
        Ok(())
    }

    fn set_symlink(&self, _target: &str) -> VfsResult<()> {
        Err(VfsError::InvalidInput)
    }

    fn ioctl(&self, _cmd: u32, _arg: usize) -> VfsResult<usize> {
        Err(VfsError::NotATty)
    }
}

/// The [`DirNodeOps`] trait provides directory operations.
impl axfs_ng_vfs::DirNodeOps for FileWrapper {
    fn read_dir(&self, offset: u64, sink: &mut dyn DirEntrySink) -> VfsResult<usize> {
        let mut fs = self.fs.lock();
        let (_inode_num, mut inode) = match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &self.path)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)?
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &self.path)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)?
            }
        };

        if !inode.is_dir() {
            return Err(VfsError::NotADirectory);
        }

        let blocks = match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                resolve_inode_block_allextend(&mut *fs, &mut *inner, &mut inode)
                    .map_err(|_| VfsError::Io)?
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                resolve_inode_block_allextend(&mut *fs, &mut *inner, &mut inode)
                    .map_err(|_| VfsError::Io)?
            }
        };

        let mut data = Vec::new();
        for (_, phys_block) in blocks {
            let cached = match self.inner {
                Ext4Inner::Disk(ref inner) => {
                    let mut inner = inner.lock();
                    fs.datablock_cache
                        .get_or_load(&mut *inner, phys_block)
                        .map_err(|_| VfsError::Io)?
                }
                Ext4Inner::Partition(ref inner) => {
                    let mut inner = inner.lock();
                    fs.datablock_cache
                        .get_or_load(&mut *inner, phys_block)
                        .map_err(|_| VfsError::Io)?
                }
            };
            data.extend_from_slice(&cached.data);
        }

        let entries = list_entries(&data);
        let mut unique = BTreeMap::new();
        for entry in entries {
            if let Some(name) = entry.name_str() {
                if name != "." && name != ".." {
                    unique.insert(name.to_string(), entry.file_type);
                }
            }
        }
        let unique_vec: Vec<_> = unique.into_iter().collect();

        let mut count = 0;

        // Handle . and ..
        match offset {
            0 => {
                if !sink.accept(".", self.inode(), NodeType::Directory, offset + 1) {
                    return Ok(count);
                }
                count += 1;
            }
            1 => {
                if !sink.accept("..", 0, NodeType::Directory, offset + 1) {
                    return Ok(count);
                }
                count += 1;
            }
            _ => {}
        }

        // Handle children
        let start_index = if offset < 2 { 0 } else { (offset - 2) as usize };
        for (i, (name, file_type)) in unique_vec.iter().enumerate().skip(start_index) {
            let ty = match *file_type {
                2 => NodeType::Directory,
                _ => NodeType::RegularFile,
            };
            if !sink.accept(name, 0, ty, offset + 1 + i as u64) {
                return Ok(count);
            }
            count += 1;
        }

        Ok(count)
    }

    fn lookup(&self, name: &str) -> VfsResult<DirEntry> {
        trace!("lookup ext4fs: {}, {}", self.path, name);
        let fpath = self.path_deal_with(name);
        if fpath.is_empty() {
            let cloned = self.clone();
            return Ok(DirEntry::new_dir(
                move |_weak| {
                    let wrapper: Arc<dyn DirNodeOps> = Arc::new(FileWrapperAsDirOps(Arc::new(cloned)));
                    axfs_ng_vfs::node::dir::DirNode::new(wrapper)
                },
                Reference::new(None, String::from(".")),
            ));
        }

        let mut fs = self.fs.lock();
        let exists = match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &fpath)
                    .map_err(|_| VfsError::Io)?
                    .is_some()
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &fpath)
                    .map_err(|_| VfsError::Io)?
                    .is_some()
            }
        };

        if exists {
            let node = Arc::new(Self::new(
                &fpath,
                self.inner.clone(),
                Arc::clone(&self.fs),
            ));
            let node_type = node.get_inode().map_err(|_| VfsError::Io)?.is_dir()
                .then_some(NodeType::Directory)
                .unwrap_or(NodeType::RegularFile);

            // Create DirEntry using DirNode::new
            Ok(DirEntry::new_dir(
                move |_weak| {
                    let wrapper: Arc<dyn DirNodeOps> = Arc::new(FileWrapperAsDirOps(node));
                    axfs_ng_vfs::node::dir::DirNode::new(wrapper)
                },
                Reference::new(None, name.to_string()),
            ))
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn is_cacheable(&self) -> bool {
        true
    }

    fn create(
        &self,
        name: &str,
        node_type: NodeType,
        _permission: NodePermission,
    ) -> VfsResult<DirEntry> {
        debug!("create {:?} on Ext4fs: {}", node_type, name);
        let fpath = self.path_deal_with(name);
        if fpath.is_empty() {
            let cloned = self.clone();
            return Ok(DirEntry::new_dir(
                move |_weak| {
                    let wrapper: Arc<dyn DirNodeOps> = Arc::new(FileWrapperAsDirOps(Arc::new(cloned)));
                    axfs_ng_vfs::node::dir::DirNode::new(wrapper)
                },
                Reference::new(None, String::from(".")),
            ));
        }

        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                match node_type {
                    NodeType::Directory => {
                        mkdir(&mut *inner, &mut *fs, &fpath);
                    }
                    _ => {
                        mkfile(&mut *inner, &mut *fs, &fpath, None, None);
                    }
                }
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                match node_type {
                    NodeType::Directory => {
                        mkdir(&mut *inner, &mut *fs, &fpath);
                    }
                    _ => {
                        mkfile(&mut *inner, &mut *fs, &fpath, None, None);
                    }
                }
            }
        }

        // Create DirEntry for newly created node
        let node = Arc::new(Self::new(
            &fpath,
            self.inner.clone(),
            Arc::clone(&self.fs),
        ));

        // Create DirEntry using DirNode::new
        Ok(DirEntry::new_dir(
            move |_weak| {
                let wrapper: Arc<dyn DirNodeOps> = Arc::new(FileWrapperAsDirOps(node));
                axfs_ng_vfs::node::dir::DirNode::new(wrapper)
            },
            Reference::new(None, name.to_string()),
        ))
    }

    fn link(&self, _name: &str, _node: &DirEntry) -> VfsResult<DirEntry> {
        // Ext4 doesn't support hard links
        Err(VfsError::Unsupported)
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        debug!("unlink ext4fs: {}", name);
        let fpath = self.path_deal_with(name);
        if fpath.is_empty() {
            return Err(VfsError::InvalidInput);
        }

        let mut fs = self.fs.lock();
        let (_inode_num, inode) = match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &fpath)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)?
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                get_inode_with_num(&mut *fs, &mut *inner, &fpath)
                    .map_err(|_| VfsError::Io)?
                    .ok_or(VfsError::NotFound)?
            }
        };

        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                if inode.is_dir() {
                    delete_dir(&mut *fs, &mut *inner, &fpath);
                } else {
                    unlink(&mut *fs, &mut *inner, &fpath);
                }
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                if inode.is_dir() {
                    delete_dir(&mut *fs, &mut *inner, &fpath);
                } else {
                    unlink(&mut *fs, &mut *inner, &fpath);
                }
            }
        }
        Ok(())
    }

    fn rename(
        &self,
        src_name: &str,
        dst_dir: &DirNode,
        dst_name: &str,
    ) -> VfsResult<()> {
        debug!("rename from {} to {}", src_name, dst_name);

        let src_fpath = self.path_deal_with(src_name);
        // Get the FileWrapper from dst_dir
        let dst_wrapper = Arc::clone(&*dst_dir.inner())
                .clone()
                .into_any()
                .downcast::<FileWrapper>()
                .map_err(|_| VfsError::InvalidInput)?;

        let dst_fpath = dst_wrapper.path_deal_with(dst_name);

        let mut fs = self.fs.lock();
        match self.inner {
            Ext4Inner::Disk(ref inner) => {
                let mut inner = inner.lock();
                let _ = mv(&mut *fs, &mut *inner, &src_fpath, &dst_fpath);
            }
            Ext4Inner::Partition(ref inner) => {
                let mut inner = inner.lock();
                let _ = mv(&mut *fs, &mut *inner, &src_fpath, &dst_fpath);
            }
        }
        Ok(())
    }
}

impl Pollable for FileWrapper {
    fn poll(&self) -> axpoll::IoEvents {
        axpoll::IoEvents::IN | axpoll::IoEvents::OUT
    }

    fn register(&self, _context: &mut core::task::Context<'_>, _events: axpoll::IoEvents) {
        // No-op for EXT4 file
    }
}

impl Drop for FileWrapper {
    fn drop(&mut self) {
        debug!("Drop struct FileWrapper {:?}", self.path);
        // File will be automatically closed when OpenFile is dropped
    }
}

impl rsext4::BlockDevice for Disk {
    fn write(&mut self, buffer: &[u8], block_id: u32, count: u32) -> rsext4::BlockDevResult<()> {
        // RVlwext4 uses 4096 byte blocks, but Disk uses 512 byte blocks
        self.set_position(block_id as u64 * BLOCK_SIZE as u64);
        let mut total_written = 0;
        let to_write = count as usize * BLOCK_SIZE;

        while total_written < to_write {
            let remaining = &buffer[total_written..];
            let written = self
                .write_one(remaining)
                .map_err(|_| rsext4::BlockDevError::WriteError)?;
            total_written += written;
        }

        Ok(())
    }

    fn read(&mut self, buffer: &mut [u8], block_id: u32, count: u32) -> rsext4::BlockDevResult<()> {
        self.set_position(block_id as u64 * BLOCK_SIZE as u64);
        let mut total_read = 0;
        let to_read = count as usize * BLOCK_SIZE;

        while total_read < to_read {
            let remaining = &mut buffer[total_read..];
            let read = self
                .read_one(remaining)
                .map_err(|_| rsext4::BlockDevError::ReadError)?;
            total_read += read;
        }

        Ok(())
    }

    fn open(&mut self) -> rsext4::BlockDevResult<()> {
        Ok(())
    }

    fn close(&mut self) -> rsext4::BlockDevResult<()> {
        Ok(())
    }

    fn total_blocks(&self) -> u64 {
        // RVlwext4 uses 4096 byte blocks
        self.size() / BLOCK_SIZE as u64
    }
}

impl rsext4::BlockDevice for Partition {
    fn write(&mut self, buffer: &[u8], block_id: u32, count: u32) -> rsext4::BlockDevResult<()> {
        self.set_position(block_id as u64 * BLOCK_SIZE as u64);
        let mut total_written = 0;
        let to_write = count as usize * BLOCK_SIZE;

        while total_written < to_write {
            let remaining = &buffer[total_written..];
            let written = self
                .write_one(remaining)
                .map_err(|_| rsext4::BlockDevError::WriteError)?;
            total_written += written;
        }

        Ok(())
    }

    fn read(&mut self, buffer: &mut [u8], block_id: u32, count: u32) -> rsext4::BlockDevResult<()> {
        self.set_position(block_id as u64 * BLOCK_SIZE as u64);
        let mut total_read = 0;
        let to_read = count as usize * BLOCK_SIZE;

        while total_read < to_read {
            let remaining = &mut buffer[total_read..];
            let read = self
                .read_one(remaining)
                .map_err(|_| rsext4::BlockDevError::ReadError)?;
            total_read += read;
        }

        Ok(())
    }

    fn open(&mut self) -> rsext4::BlockDevResult<()> {
        Ok(())
    }

    fn close(&mut self) -> rsext4::BlockDevResult<()> {
        Ok(())
    }

    fn total_blocks(&self) -> u64 {
        // RVlwext4 uses 4096 byte blocks
        self.size() / BLOCK_SIZE as u64
    }
}
