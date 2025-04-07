#include <sys/mman.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "include/shm.h"

// Memory page management
const size_t PAGE_SIZE = 4096;
const size_t MAX_REGIONS_PER_PAGE = PAGE_SIZE / sizeof(CMemoryRegion);

static void** allocated_pages = NULL;
static size_t allocated_pages_count = 0;
static size_t allocated_pages_capacity = 0;
static CMemoryRegion* current_page = NULL;
static size_t current_offset = 0;
static size_t total_count = 0;

size_t get_memory_regions_total_count() { return total_count; }
void* get_memory_regions_page_base() { return allocated_pages; }
size_t get_memory_regions_page_count() { return allocated_pages_count; }

void dump_allocated_pages() {
  printf("Address of allocated_pages array: %p\n", (void*)&allocated_pages);
  for (size_t i = 0; i < allocated_pages_count; ++i) {
    printf("Page %zu address: %p\n", i, allocated_pages[i]);
  }
}

void init_shared_page() {
  void* page = mmap(NULL, PAGE_SIZE, PROT_READ | PROT_WRITE,
                    MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
  if (page == MAP_FAILED) {
    perror("Failed to allocate page");
    exit(1);
  }

  if (allocated_pages_count == allocated_pages_capacity) {
    allocated_pages_capacity = allocated_pages_capacity == 0 ? 4 : allocated_pages_capacity * 2;
    allocated_pages = realloc(allocated_pages, allocated_pages_capacity * sizeof(void*));
    if (!allocated_pages) {
      perror("Failed to reallocate memory for allocated_pages");
      exit(1);
    }
  }

  allocated_pages[allocated_pages_count++] = page;
  current_page = (CMemoryRegion*)page;
  current_offset = 0;
}

void write_region(const CMemoryRegion* reg) {
  if (current_offset >= MAX_REGIONS_PER_PAGE) {
    init_shared_page();
  }

  memcpy(&current_page[current_offset], reg, sizeof(CMemoryRegion));
  current_offset++;
  total_count++;
}

void cleanup_pages() {
  for (size_t i = 0; i < allocated_pages_count; ++i) {
    munmap(allocated_pages[i], PAGE_SIZE);
  }
  free(allocated_pages);
  allocated_pages = NULL;
  allocated_pages_count = 0;
  allocated_pages_capacity = 0;
}

void parse_proc_self_maps() {
  printf("Parsing /proc/self/maps...\n");
  
  FILE* file = fopen("/proc/self/maps", "r");
  if (!file) {
    perror("Failed to open /proc/self/maps");
    exit(1);
  }

  init_shared_page();

  char line[512];
  while (fgets(line, sizeof(line), file)) {
    CMemoryRegion reg = {0};
    char address_range[64], perms[5], device[6], pathname[256] = {0};
    unsigned long long start, end, offset;
    unsigned int inode;

    if (sscanf(line, "%63s %4s %llx %5s %u %255[^\n]",
               address_range, perms, &offset, device, &inode, pathname) < 5) {
      continue;
    }

    sscanf(address_range, "%llx-%llx", &start, &end);
    reg.start = start;
    reg.end = end;
    strncpy(reg.permissions, perms, 4);
    reg.permissions[4] = '\0';
    reg.offset = offset;
    strncpy(reg.device, device, 5);
    reg.device[5] = '\0';
    reg.inode = inode;
    strncpy(reg.pathname, pathname, 255);
    reg.pathname[255] = '\0';

    reg.flags = 0;
    if (strchr(perms, 'r')) reg.flags |= 1 << 0;
    if (strchr(perms, 'w')) reg.flags |= 1 << 1;
    if (strchr(perms, 'x')) reg.flags |= 1 << 2;
    if (strncmp(pathname, "/dev/", 5) == 0) reg.flags |= 1 << 4;

    write_region(&reg);
  }

  fclose(file);
}

void print_regions() {
  printf("\n==== C Side Verification ====\n");

  if (allocated_pages_count == 0) {
    printf("No pages allocated!\n");
    return;
  }

  size_t count = 0;
  const size_t max_per_page = PAGE_SIZE / sizeof(CMemoryRegion);

  for (size_t page_idx = 0; page_idx < allocated_pages_count; ++page_idx) {
    CMemoryRegion* page = (CMemoryRegion*)allocated_pages[page_idx];
    size_t regions_in_page = (page_idx == allocated_pages_count - 1)
                                 ? current_offset
                                 : max_per_page;

    printf("── Page %zu (%zu regions) ── Base Address: 0x%016lx\n",
           page_idx + 1, regions_in_page, (unsigned long)page);

    for (size_t i = 0; i < regions_in_page; ++i) {
      CMemoryRegion* reg = &page[i];
      printf("[%zu] 0x%016lx-0x%016lx Perms: %s Path: %s Flags: 0x%lx\n",
             count++, reg->start, reg->end, reg->permissions, reg->pathname, reg->flags);
    }
  }

  printf("Total regions verified: %zu\n", count);
  printf("==== Verification Complete ====\n\n");
}
