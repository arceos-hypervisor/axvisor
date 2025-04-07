// shm.h - shared memory with AxVisor
#ifndef SHM_H
#define SHM_H

#include <assert.h>
#include <stddef.h>
#include <stdint.h>

typedef struct CMemoryRegion {
    uint64_t start;      // 8 bytes: Start address of the memory region
    uint64_t end;        // 8 bytes: End address of the memory region
    char permissions[8]; // 4 chars + null terminator: Access permissions (r/w/x)
                                             // and flags (p/s), aligned to 8 bytes.
    uint64_t offset;     // 8 bytes: Offset in the mapped file
    char device[8]; // 5 chars + null terminator: Device number (major:minor) for
                                    // special files, aligned to 8 bytes.
    uint64_t inode; // 8 bytes: Inode number of the mapped file
    char pathname[256]; // Fixed-size buffer for path: Mapped file path or region
                                            // name (e.g., [heap])
    uint64_t flags;     // 8 bytes: Flags
} CMemoryRegion;

static_assert(
    sizeof(CMemoryRegion) == 312,
    "CMemoryRegion size does not match the expected size of 307 bytes.");

// Memory page management
extern const size_t PAGE_SIZE;
extern const size_t MAX_REGIONS_PER_PAGE;

void cleanup_pages();
void print_regions();
void parse_proc_self_maps();
void dump_allocated_pages();

size_t get_memory_regions_total_count();
void *get_memory_regions_page_base();
size_t get_memory_regions_page_count();

#endif // SHM_H