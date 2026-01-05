use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::{any::Any, cell::OnceCell, time::Duration};

use axfs_ng_vfs::{DirEntry, DirEntrySink, DirNodeOps, FileNode, FileNodeOps, FilesystemOps, Metadata, MetadataUpdate, NodeOps, NodePermission, NodeType, VfsError, VfsResult, Reference};
use axfs_ng_vfs::node::dir::DirNode;
use axpoll::Pollable;
use spin::Mutex;
use fatfs::{Dir, File, LossyOemCpConverter, NullTimeProvider, Read, Seek, SeekFrom, Write};

use crate::dev::{Disk, Partition};

const BLOCK_SIZE: usize = 512;

pub struct FatFileSystem {
    inner: fatfs::FileSystem<PartitionWrapper, NullTimeProvider, LossyOemCpConverter>,
    root_dir: OnceCell<DirEntry>,
}

/// A wrapper for Partition to implement the required traits for fatfs
pub struct PartitionWrapper {
    partition: Partition,
}

impl PartitionWrapper {
    pub fn new(partition: Partition) -> Self {
        Self { partition }
    }
}

impl fatfs::IoBase for PartitionWrapper {
    type Error = ();
}

impl fatfs::Read for PartitionWrapper {
    fn read(&mut self, mut buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut read_len = 0;
        while !buf.is_empty() {
            match self.partition.read_one(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                    read_len += n;
                }
                Err(_) => return Err(()),
            }
        }
        Ok(read_len)
    }
}

impl fatfs::Write for PartitionWrapper {
    fn write(&mut self, mut buf: &[u8]) -> Result<usize, Self::Error> {
        let mut write_len = 0;
        while !buf.is_empty() {
            match self.partition.write_one(buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf = &buf[n..];
                    write_len += n;
                }
                Err(_) => return Err(()),
            }
        }
        Ok(write_len)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl fatfs::Seek for PartitionWrapper {
    fn seek(&mut self, pos: fatfs::SeekFrom) -> Result<u64, Self::Error> {
        let size = self.partition.size();
        let new_pos = match pos {
            fatfs::SeekFrom::Start(pos) => Some(pos),
            fatfs::SeekFrom::Current(off) => self.partition.position().checked_add_signed(off),
            fatfs::SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .ok_or(())?;
        if new_pos > size {
            warn!("Seek beyond the end of the partition");
        }
        self.partition.set_position(new_pos);
        Ok(new_pos)
    }
}

pub struct FileWrapper<'a>(
    Mutex<File<'a, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>>,
);
pub struct DirWrapper<'a>(Dir<'a, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>);

unsafe impl Sync for FatFileSystem {}
unsafe impl Send for FatFileSystem {}
unsafe impl Send for FileWrapper<'_> {}
unsafe impl Sync for FileWrapper<'_> {}
unsafe impl Send for DirWrapper<'_> {}
unsafe impl Sync for DirWrapper<'_> {}

impl FatFileSystem {
    #[cfg(feature = "use-ramdisk")]
    #[allow(dead_code)]
    pub fn new(mut disk: Disk) -> Self {
        let opts = fatfs::FormatVolumeOptions::new();
        fatfs::format_volume(&mut disk, opts).expect("failed to format volume");
        let inner = fatfs::FileSystem::new(disk, fatfs::FsOptions::new())
            .expect("failed to initialize FAT filesystem");

        Self {
            inner,
            root_dir: OnceCell::new(),
        }
    }

    #[cfg(not(feature = "use-ramdisk"))]
    #[allow(dead_code)]
    pub fn new(disk: Disk) -> Self {
        let disk_size = disk.size();
        let wrapper = PartitionWrapper::new(crate::dev::Partition::new(disk, 0, disk_size / 512));
        let inner = fatfs::FileSystem::new(wrapper, fatfs::FsOptions::new())
            .expect("failed to initialize FAT filesystem");
        Self {
            inner,
            root_dir: OnceCell::new(),
        }
    }

    /// Create a new FAT filesystem from a partition
    pub fn from_partition(partition: Partition) -> Self {
        let wrapper = PartitionWrapper::new(partition);
        let inner = fatfs::FileSystem::new(wrapper, fatfs::FsOptions::new())
            .expect("failed to initialize FAT filesystem on partition");
        Self {
            inner,
            root_dir: OnceCell::new(),
        }
    }

    #[allow(dead_code)]
    pub fn init(&'static self) {
        // root_dir is already initialized in new(), so nothing to do here
    }

    fn new_file(
        file: File<'_, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>,
    ) -> DirEntry {
        // Use a Box to extend the lifetime of the file
        let file_box = Box::new(file);
        let file_static = unsafe {
            core::mem::transmute::<
                Box<File<'_, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>>,
                Box<File<'static, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>>,
            >(file_box)
        };
        let file_wrapper = Arc::new(FileWrapper(Mutex::new(*file_static)));
        let file_node = FileNode::new(file_wrapper);
        DirEntry::new_file(
            file_node,
            NodeType::RegularFile,
            Reference::new(None, String::new()),
        )
    }

    fn new_dir(
        dir: Dir<'_, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>,
    ) -> DirEntry {
        // Use a Box to extend the lifetime of the dir
        let dir_box = Box::new(dir);
        let dir_static = unsafe {
            core::mem::transmute::<
                Box<Dir<'_, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>>,
                Box<Dir<'static, PartitionWrapper, NullTimeProvider, LossyOemCpConverter>>,
            >(dir_box)
        };
        let dir_wrapper = Arc::new(DirWrapper(*dir_static));
        DirEntry::new_dir(
            |_weak| DirNode::new(dir_wrapper.clone()),
            Reference::new(None, String::new()),
        )
    }
}

impl NodeOps for FileWrapper<'static> {
    fn inode(&self) -> u64 {
        // FAT fs doesn't have inode numbers
        0
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        let size = self.0.lock().seek(SeekFrom::End(0)).map_err(as_vfs_err)?;
        let blocks = size.div_ceil(BLOCK_SIZE as u64);
        // FAT fs doesn't support permissions, we just set everything to 644
        let perm = NodePermission::from_bits_truncate(0o644);
        Ok(Metadata {
            device: 0,
            inode: 0,
            nlink: 1,
            mode: perm,
            node_type: NodeType::RegularFile,
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
        // FAT fs doesn't support metadata updates
        Ok(())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        // For now, we don't have a proper reference to the filesystem
        // Return a static placeholder
        // TODO: Redesign to maintain a proper filesystem reference
        struct DummyFs;
        impl FilesystemOps for DummyFs {
            fn name(&self) -> &str { "fatfs" }
            fn root_dir(&self) -> DirEntry { panic!("Should not be called") }
            fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> { Err(VfsError::Unsupported) }
            fn flush(&self) -> VfsResult<()> { Ok(()) }
        }
        static DUMMY_FS: DummyFs = DummyFs;
        &DUMMY_FS
    }

    fn len(&self) -> VfsResult<u64> {
        let mut file = self.0.lock();
        file.seek(SeekFrom::End(0)).map_err(as_vfs_err)
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        let mut file = self.0.lock();
        file.flush().map_err(as_vfs_err)
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl FileNodeOps for FileWrapper<'static> {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        let mut file = self.0.lock();
        file.seek(SeekFrom::Start(offset)).map_err(as_vfs_err)?;
        file.read(buf).map_err(as_vfs_err)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> VfsResult<usize> {
        let mut file = self.0.lock();
        file.seek(SeekFrom::Start(offset)).map_err(as_vfs_err)?;
        file.write(buf).map_err(as_vfs_err)
    }

    fn append(&self, buf: &[u8]) -> VfsResult<(usize, u64)> {
        let mut file = self.0.lock();
        let offset = file.seek(SeekFrom::End(0)).map_err(as_vfs_err)?;
        let written = file.write(buf).map_err(as_vfs_err)?;
        Ok((written, offset + written as u64))
    }

    fn set_len(&self, size: u64) -> VfsResult<()> {
        let mut file = self.0.lock();
        let current_size = file.seek(SeekFrom::End(0)).map_err(as_vfs_err)?;

        if size <= current_size {
            // If the target size is smaller than the current size,
            // perform a standard truncation operation
            file.seek(SeekFrom::Start(size)).map_err(as_vfs_err)?;
            file.truncate().map_err(as_vfs_err)
        } else {
            // Calculate the number of bytes to fill
            let mut zeros_needed = size - current_size;
            // Create a buffer of zeros
            let zeros = [0u8; 4096];
            while zeros_needed > 0 {
                let to_write = core::cmp::min(zeros_needed, zeros.len() as u64);
                let write_buf = &zeros[..to_write as usize];
                file.write(write_buf).map_err(as_vfs_err)?;
                zeros_needed -= to_write;
            }
            Ok(())
        }
    }

    fn set_symlink(&self, _target: &str) -> VfsResult<()> {
        Err(VfsError::InvalidInput)
    }

    fn ioctl(&self, _cmd: u32, _arg: usize) -> VfsResult<usize> {
        Err(VfsError::NotATty)
    }
}

impl Pollable for FileWrapper<'static> {
    fn poll(&self) -> axpoll::IoEvents {
        axpoll::IoEvents::IN | axpoll::IoEvents::OUT
    }

    fn register(&self, _context: &mut core::task::Context<'_>, _events: axpoll::IoEvents) {
        // No-op for FAT file system
    }
}

impl NodeOps for DirWrapper<'static> {
    fn inode(&self) -> u64 {
        // FAT fs doesn't have inode numbers
        0
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        // FAT fs doesn't support permissions, we just set everything to 755
        let perm = NodePermission::from_bits_truncate(0o755);
        Ok(Metadata {
            device: 0,
            inode: 0,
            nlink: 2,
            mode: perm,
            node_type: NodeType::Directory,
            uid: 0,
            gid: 0,
            size: BLOCK_SIZE as u64,
            block_size: BLOCK_SIZE as u64,
            blocks: 1,
            rdev: axfs_ng_vfs::DeviceId::new(0, 0),
            atime: Duration::ZERO,
            mtime: Duration::ZERO,
            ctime: Duration::ZERO,
        })
    }

    fn update_metadata(&self, _update: MetadataUpdate) -> VfsResult<()> {
        // FAT fs doesn't support metadata updates
        Ok(())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        // For now, we don't have a proper reference to the filesystem
        // Return a static placeholder
        // TODO: Redesign to maintain a proper filesystem reference
        struct DummyFs;
        impl FilesystemOps for DummyFs {
            fn name(&self) -> &str { "fatfs" }
            fn root_dir(&self) -> DirEntry { panic!("Should not be called") }
            fn stat(&self) -> VfsResult<axfs_ng_vfs::StatFs> { Err(VfsError::Unsupported) }
            fn flush(&self) -> VfsResult<()> { Ok(()) }
        }
        static DUMMY_FS: DummyFs = DummyFs;
        &DUMMY_FS
    }

    fn len(&self) -> VfsResult<u64> {
        Ok(BLOCK_SIZE as u64)
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        Ok(())
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl DirNodeOps for DirWrapper<'static> {
    fn lookup(&self, path: &str) -> VfsResult<DirEntry> {
        debug!("lookup at fatfs: {}", path);
        let path = path.trim_matches('/');
        if path.is_empty() || path == "." {
            return Ok(FatFileSystem::new_dir(self.0.clone()));
        }
        if let Some(rest) = path.strip_prefix("./") {
            return self.lookup(rest);
        }

        // TODO: use `fatfs::Dir::find_entry`, but it's not public.
        if let Ok(file) = self.0.open_file(path) {
            Ok(FatFileSystem::new_file(file))
        } else if let Ok(dir) = self.0.open_dir(path) {
            Ok(FatFileSystem::new_dir(dir))
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn create(
        &self,
        path: &str,
        ty: NodeType,
        _permission: NodePermission,
    ) -> VfsResult<DirEntry> {
        debug!("create {:?} at fatfs: {}", ty, path);
        let path = path.trim_matches('/');
        if path.is_empty() || path == "." {
            return Ok(FatFileSystem::new_dir(self.0.clone()));
        }
        if let Some(rest) = path.strip_prefix("./") {
            return self.create(rest, ty, _permission);
        }

        match ty {
            NodeType::RegularFile => {
                let file = self.0.create_file(path).map_err(as_vfs_err)?;
                Ok(FatFileSystem::new_file(file))
            }
            NodeType::Directory => {
                let dir = self.0.create_dir(path).map_err(as_vfs_err)?;
                Ok(FatFileSystem::new_dir(dir))
            }
            _ => Err(VfsError::Unsupported),
        }
    }

    fn link(&self, _name: &str, _node: &DirEntry) -> VfsResult<DirEntry> {
        // FAT fs doesn't support hard links
        Err(VfsError::Unsupported)
    }

    fn unlink(&self, path: &str) -> VfsResult<()> {
        debug!("remove at fatfs: {}", path);
        let path = path.trim_matches('/');
        assert!(!path.is_empty()); // already check at `root.rs`
        if let Some(rest) = path.strip_prefix("./") {
            return self.unlink(rest);
        }
        self.0.remove(path).map_err(as_vfs_err)
    }

    fn read_dir(&self, start_idx: u64, sink: &mut dyn DirEntrySink) -> VfsResult<usize> {
        let mut iter = self.0.iter().skip(start_idx as usize);
        let mut count = 0;
        for entry in iter {
            match entry {
                Ok(entry) => {
                    let ty = if entry.is_dir() {
                        NodeType::Directory
                    } else if entry.is_file() {
                        NodeType::RegularFile
                    } else {
                        continue;
                    };
                    if !sink.accept(&entry.file_name(), count as u64, ty, (count + 1) as u64) {
                        break;
                    }
                    count += 1;
                }
                Err(_) => break,
            }
        }
        Ok(count)
    }

    fn rename(
        &self,
        src_name: &str,
        _dst_dir: &DirNode,
        dst_name: &str,
    ) -> VfsResult<()> {
        // `src_path` and `dst_path` should in the same mounted fs
        debug!(
            "rename at fatfs, src_path: {}, dst_path: {}",
            src_name, dst_name
        );

        self.0
            .rename(src_name, &self.0, dst_name)
            .map_err(as_vfs_err)
    }
}

impl FilesystemOps for FatFileSystem {
    fn name(&self) -> &str {
        "fat"
    }

    fn root_dir(&self) -> DirEntry {
        self.root_dir
            .get_or_init(|| {
                debug!("Creating root directory for FAT filesystem");
                let root_dir = self.inner.root_dir();
                debug!("Successfully got root directory from FAT filesystem");
                Self::new_dir(root_dir)
            })
            .clone()
    }

    fn stat(&self) -> VfsResult<axfs_ng_vfs::fs::StatFs> {
        // FAT filesystem statistics - simplified implementation
        Ok(axfs_ng_vfs::fs::StatFs {
            fs_type: 0x4d44, // FAT32 magic number
            block_size: BLOCK_SIZE as u32,
            blocks: 0,
            blocks_free: 0,
            blocks_available: 0,
            file_count: 0,
            free_file_count: 0,
            name_length: 255,
            fragment_size: 0,
            mount_flags: 0,
        })
    }
}

impl fatfs::IoBase for Disk {
    type Error = ();
}

impl Read for Disk {
    fn read(&mut self, mut buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut read_len = 0;
        while !buf.is_empty() {
            match self.read_one(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                    read_len += n;
                }
                Err(_) => return Err(()),
            }
        }
        Ok(read_len)
    }
}

impl Write for Disk {
    fn write(&mut self, mut buf: &[u8]) -> Result<usize, Self::Error> {
        let mut write_len = 0;
        while !buf.is_empty() {
            match self.write_one(buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf = &buf[n..];
                    write_len += n;
                }
                Err(_) => return Err(()),
            }
        }
        Ok(write_len)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Seek for Disk {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let size = self.size();
        let new_pos = match pos {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(off) => self.position().checked_add_signed(off),
            SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .ok_or(())?;
        if new_pos > size {
            warn!("Seek beyond the end of the block device");
        }
        self.set_position(new_pos);
        Ok(new_pos)
    }
}

const fn as_vfs_err(err: fatfs::Error<()>) -> VfsError {
    use fatfs::Error::*;
    match err {
        AlreadyExists => VfsError::AlreadyExists,
        CorruptedFileSystem => VfsError::InvalidData,
        DirectoryIsNotEmpty => VfsError::DirectoryNotEmpty,
        InvalidInput | InvalidFileNameLength | UnsupportedFileNameCharacter => {
            VfsError::InvalidInput
        }
        NotEnoughSpace => VfsError::StorageFull,
        NotFound => VfsError::NotFound,
        UnexpectedEof => VfsError::UnexpectedEof,
        WriteZero => VfsError::WriteZero,
        Io(_) => VfsError::Io,
        _ => VfsError::Io,
    }
}
