use alloc::sync::Arc;
use rdif_block::IQueue;
use spin::Mutex;

/// A disk device with a cursor.
pub struct Disk {
    block_id: u64,
    offset: usize,
    dev: Arc<Mutex<dyn IQueue>>,
}

impl Clone for Disk {
    fn clone(&self) -> Self {
        Self {
            block_id: self.block_id,
            offset: self.offset,
            dev: Arc::clone(&self.dev),
        }
    }
}

impl Disk {
    /// Create a new disk.
    pub fn new(dev: Arc<Mutex<dyn IQueue>>) -> Self {
        Self {
            block_id: 0,
            offset: 0,
            dev,
        }
    }

    /// Get the size of the disk.
    pub fn size(&self) -> u64 {
        let dev = self.dev.lock();
        dev.num_blocks() as u64 * dev.block_size() as u64
    }

    /// Get the position of the cursor.
    pub fn position(&self) -> u64 {
        let block_size = self.dev.lock().block_size();
        self.block_id * block_size as u64 + self.offset as u64
    }

    /// Set the position of the cursor.
    pub fn set_position(&mut self, pos: u64) {
        let block_size = self.dev.lock().block_size();
        self.block_id = pos / block_size as u64;
        self.offset = pos as usize % block_size;
    }

    /// Read within one block, returns the number of bytes read.
    pub fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, rdif_block::BlkError> {
        let block_size = self.dev.lock().block_size();
        let read_size = if self.offset == 0 && buf.len() >= block_size {
            // whole block
            let mut dev = self.dev.lock();
            let buffer = rdif_block::Buffer {
                virt: buf.as_mut_ptr(),
                bus: buf.as_ptr() as u64, // Assume physical address same as virtual for now
                size: block_size,
            };
            let request = rdif_block::Request {
                block_id: self.block_id as usize,
                kind: rdif_block::RequestKind::Read(buffer),
            };
            dev.submit_request(request)?;
            self.block_id += 1;
            block_size
        } else {
            // partial block
            let mut data = alloc::vec![0u8; block_size];
            let start = self.offset;
            let count = buf.len().min(block_size - self.offset);

            {
                let mut dev = self.dev.lock();
                let buffer = rdif_block::Buffer {
                    virt: data.as_mut_ptr(),
                    bus: data.as_ptr() as u64,
                    size: block_size,
                };
                let request = rdif_block::Request {
                    block_id: self.block_id as usize,
                    kind: rdif_block::RequestKind::Read(buffer),
                };
                dev.submit_request(request)?;
            }
            buf[..count].copy_from_slice(&data[start..start + count]);
            self.offset += count;
            if self.offset >= block_size {
                self.block_id += 1;
                self.offset -= block_size;
            }
            count
        };
        Ok(read_size)
    }

    /// Write within one block, returns the number of bytes written.
    pub fn write_one(&mut self, buf: &[u8]) -> Result<usize, rdif_block::BlkError> {
        let block_size = self.dev.lock().block_size();
        let write_size = if self.offset == 0 && buf.len() >= block_size {
            // whole block
            let mut dev = self.dev.lock();
            let request = rdif_block::Request {
                block_id: self.block_id as usize,
                kind: rdif_block::RequestKind::Write(&buf[0..block_size]),
            };
            dev.submit_request(request)?;
            self.block_id += 1;
            block_size
        } else {
            // partial block
            let mut data = alloc::vec![0u8; block_size];
            let start = self.offset;
            let count = buf.len().min(block_size - self.offset);

            let mut dev = self.dev.lock();
            let buffer = rdif_block::Buffer {
                virt: data.as_mut_ptr(),
                bus: data.as_ptr() as u64,
                size: block_size,
            };
            let request = rdif_block::Request {
                block_id: self.block_id as usize,
                kind: rdif_block::RequestKind::Read(buffer),
            };
            dev.submit_request(request)?;
            data[start..start + count].copy_from_slice(&buf[..count]);
            let request = rdif_block::Request {
                block_id: self.block_id as usize,
                kind: rdif_block::RequestKind::Write(&data),
            };
            dev.submit_request(request)?;

            self.offset += count;
            if self.offset >= block_size {
                self.block_id += 1;
                self.offset -= block_size;
            }
            count
        };
        Ok(write_size)
    }
}

/// A partition wrapper that provides access to a specific partition of a disk.
pub struct Partition {
    disk: Arc<Mutex<Disk>>,
    start_lba: u64,
    end_lba: u64,
    position: u64,
}

impl Partition {
    /// Create a new partition wrapper.
    pub fn new(disk: Disk, start_lba: u64, end_lba: u64) -> Self {
        Self {
            disk: Arc::new(Mutex::new(disk)),
            start_lba,
            end_lba,
            position: 0,
        }
    }

    /// Get the size of the partition.
    pub fn size(&self) -> u64 {
        let block_size = self.disk.lock().dev.lock().block_size();
        (self.end_lba - self.start_lba + 1) * block_size as u64
    }

    /// Get the position of the cursor.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Set the position of the cursor.
    pub fn set_position(&mut self, pos: u64) {
        self.position = pos.min(self.size());
    }

    /// Read within one block, returns the number of bytes read.
    pub fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, rdif_block::BlkError> {
        if self.position >= self.size() {
            return Ok(0);
        }

        let remaining = self.size() - self.position;
        let to_read = buf.len().min(remaining as usize);
        let buf = &mut buf[..to_read];

        // Calculate the absolute position on the disk
        let block_size = self.disk.lock().dev.lock().block_size();
        let abs_pos = self.start_lba * block_size as u64 + self.position;

        // Set disk position and read
        let read_len = {
            let mut disk = self.disk.lock();
            disk.set_position(abs_pos);
            disk.read_one(buf)?
        };

        self.position += read_len as u64;
        Ok(read_len)
    }

    /// Write within one block, returns the number of bytes written.
    pub fn write_one(&mut self, buf: &[u8]) -> Result<usize, rdif_block::BlkError> {
        if self.position >= self.size() {
            return Ok(0);
        }

        let remaining = self.size() - self.position;
        let to_write = buf.len().min(remaining as usize);
        let buf = &buf[..to_write];

        // Calculate the absolute position on the disk
        let block_size = self.disk.lock().dev.lock().block_size();
        let abs_pos = self.start_lba * block_size as u64 + self.position;

        // Set disk position and write
        let write_len = {
            let mut disk = self.disk.lock();
            disk.set_position(abs_pos);
            disk.write_one(buf)?
        };

        self.position += write_len as u64;
        Ok(write_len)
    }
}
