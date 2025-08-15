use core::fmt;

use axaddrspace::{GuestVirtAddr, MappingFlags};
use memory_addr::{AddrRange, MemoryAddr};

/// A memory area represents a continuous range of virtual memory with the same
/// flags for LibOS.
#[derive(Clone, Copy, PartialEq)]
pub struct GuestMemoryArea {
    va_range: AddrRange<GuestVirtAddr>,
    flags: MappingFlags,
}

#[allow(unused)]
impl GuestMemoryArea {
    /// Creates a new memory area.
    pub fn new(va_range: AddrRange<GuestVirtAddr>, flags: MappingFlags) -> Self {
        Self { va_range, flags }
    }

    /// Returns the virtual address range.
    pub const fn va_range(&self) -> AddrRange<GuestVirtAddr> {
        self.va_range
    }

    /// Returns the memory flags, e.g., the permission bits.
    pub const fn flags(&self) -> MappingFlags {
        self.flags
    }

    /// Returns the start address of the memory area.
    pub const fn start(&self) -> GuestVirtAddr {
        self.va_range.start
    }

    /// Returns the end address of the memory area.
    pub const fn end(&self) -> GuestVirtAddr {
        self.va_range.end
    }

    /// Returns the size of the memory area.
    pub fn size(&self) -> usize {
        self.va_range.size()
    }
}

#[allow(unused)]
impl GuestMemoryArea {
    /// Changes the flags.
    pub(crate) fn set_flags(&mut self, new_flags: MappingFlags) {
        self.flags = new_flags;
    }

    /// Changes the end address of the memory area.
    pub(crate) fn set_end(&mut self, new_end: GuestVirtAddr) {
        self.va_range.end = new_end;
    }

    /// Shrinks the memory area at the right side.
    ///
    /// The end address of the memory area is decreased by `new_size`. The
    /// shrunk part is unmapped.
    ///
    /// `new_size` must be greater than 0 and less than the current size.
    pub(crate) fn shrink_right(&mut self, new_size: usize) -> GuestMemoryArea {
        assert!(new_size > 0 && new_size < self.size());
        let old_size = self.size();
        let unmap_size = old_size - new_size;

        // Use wrapping_add to avoid overflow check.
        // Safety: `new_size` is less than the current size, so it will never overflow.
        let unmap_start = self.start().wrapping_add(new_size);

        // Use wrapping_sub to avoid overflow check, same as above.
        self.va_range.end = self.va_range.end.wrapping_sub(unmap_size);

        // Return the new memory area that represents the shrunk part.
        return Self::new(
            AddrRange::from_start_size(unmap_start, unmap_size),
            self.flags,
        );
    }

    /// Shrinks the memory area at the left side.
    ///
    /// The start address of the memory area is increased by `new_size`. The
    /// shrunk part is unmapped.
    ///
    /// `new_size` must be greater than 0 and less than the current size.
    pub(crate) fn shrink_left(&mut self, new_size: usize) -> GuestMemoryArea {
        assert!(new_size > 0 && new_size < self.size());
        let old_size = self.size();
        let unmap_size = old_size - new_size;

        let left = Self::new(
            AddrRange::from_start_size(self.start(), unmap_size),
            self.flags,
        );

        // Use wrapping_add to avoid overflow check.
        // Safety: `unmap_size` is less than the current size, so it will
        // never overflow.
        self.va_range.start = self.va_range.start.wrapping_add(unmap_size);
        // Return the new memory area that represents the shrunk part.
        left
    }

    /// Splits the memory area at the given position.
    ///
    /// The original memory area is shrunk to the left part, and the right part
    /// is returned.
    ///
    /// Returns `None` if the given position is not in the memory area, or one
    /// of the parts is empty after splitting.
    pub(crate) fn split(&mut self, pos: GuestVirtAddr) -> Option<Self> {
        if self.start() < pos && pos < self.end() {
            let new_area = Self::new(
                AddrRange::from_start_size(
                    pos,
                    // Use wrapping_sub_addr to avoid overflow check. It is safe because
                    // `pos` is within the memory area.
                    self.end().wrapping_sub_addr(pos),
                ),
                self.flags,
            );
            self.va_range.end = pos;
            Some(new_area)
        } else {
            None
        }
    }
}

impl fmt::Debug for GuestMemoryArea
where
    GuestVirtAddr: fmt::Debug,
    MappingFlags: fmt::Debug + Copy,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("MemoryArea")
            .field("va_range", &self.va_range)
            .field("flags", &self.flags)
            .finish()
    }
}
