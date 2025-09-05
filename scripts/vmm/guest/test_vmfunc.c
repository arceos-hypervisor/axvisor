#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>

#define HYPERCALL "vmcall"

static void in_guest() {
  printf("Execute VMCALL OK.\n");
  printf("You are in the Guest mode.\n");
}

static void in_host() {
  printf("Execute VMCALL failed.\n");
  printf("You are in the Host mode.\n");
  exit(1);
}

static void sig_handler(int signum) {
  printf("Caught signal %d\n", signum);
  in_host();
}

static inline long hypercall(int num) {
  long ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num) : "memory");
  return ret;
}

static inline long hypercall_ext(int num, unsigned long a1, unsigned long a2,
                                 unsigned long a3, unsigned long a4,
                                 unsigned long a5, unsigned long a6) {
  long ret;
  asm volatile(HYPERCALL
               : "=a"(ret)
               : "a"(num), "D"(a1), "S"(a2), "d"(a3), "c"(a4), "r"(a5), "r"(a6)
               : "memory");
  return ret;
}

static inline void vmfunc_call(uint64_t function_id, uint64_t param) {
  asm volatile("vmfunc" : : "a"(function_id), "c"(param) : "memory");
}

void switch_eptp(uint64_t eptp_value) { vmfunc_call(0, eptp_value); }

static inline uint64_t rdtsc_begin(void) {
    unsigned lo, hi;
    __asm__ __volatile__ (
        "lfence\n\t"      // serialize before RDTSC
        "rdtsc\n\t"
        : "=a"(lo), "=d"(hi)
        :
        : "%rbx", "%rcx");
    return ((uint64_t)hi << 32) | lo;
}

static inline uint64_t rdtsc_end(void) {
    unsigned lo, hi;
    __asm__ __volatile__ (
        "rdtscp\n\t"
        : "=a"(lo), "=d"(hi)
        :
        : "%rcx");
    __asm__ __volatile__("lfence" ::: "memory");  // ensure serialization
    return ((uint64_t)hi << 32) | lo;
}

int main() {
  signal(SIGSEGV, sig_handler);
  signal(SIGILL, sig_handler);
  //   int ret = hypercall(0xc0000000);

  int hvc_code = 0xe0000000;

  int value = 0x2333;

  size_t page_size = sysconf(_SC_PAGESIZE);
  void *mem = mmap(NULL, page_size, PROT_READ | PROT_WRITE,
                   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);

  if (mem == MAP_FAILED) {
    perror("mmap failed");
    return 1;
  }

  if ((uintptr_t)mem % page_size != 0) {
    fprintf(stderr, "Unaligned address: %p\n", mem);
    munmap(mem, page_size);
    return 1;
  }

  int *val = (int *)mem;
  *val = 0x2333;
  printf("Value at %p: 0x%x\n", mem, *val);

  int ret = hypercall_ext(hvc_code, 2333, (unsigned long)mem, 0, 0, 0, 0);
  if (ret == hvc_code) {
    in_guest();
  } else {
    in_host();
  }

  for (int i = 0; i < 2; i++) {
    printf("Switch EPTP %d\n", i);
    switch_eptp(i);
    printf("Switch EPTP %d success\n", i);

    printf("Value at %p: 0x%x\n", mem, *val);
  }

  const int rounds = 10000;
  uint64_t total = 0;

  for (int i = 0; i < rounds; i++) {
    uint64_t entry = 0;
    if (i % 2 == 0) {
      entry = 1;
    } else {
      entry = 0;
    }

    uint64_t start = rdtsc_begin();
    switch_eptp(entry);
    uint64_t end = rdtsc_end();

    if (i % 100 == 0) {
      printf("Round %d: switch to EPTP %lu took %lu cycles\n", i, entry,
             end - start);
    }

    total += (end - start);
  }

  printf("VMFUNC benchmark: avg = %lu cycles over %d rounds\n", total / rounds,
         rounds);

  munmap(mem, page_size);

  exit(0);

  return 0;
}
