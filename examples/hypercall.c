#include <stdint.h>

#include "include/hypercall.h"

inline int64_t hypercall(int num) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num) : "memory");
  return ret;
}

inline int64_t hypercall_1(int num, uint64_t a1) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num), "D"(a1) : "memory");
  return ret;
}

inline int64_t hypercall_2(int num, uint64_t a1, uint64_t a2) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num), "D"(a1), "S"(a2) : "memory");
  return ret;
}

inline int64_t hypercall_3(int num, uint64_t a1, uint64_t a2, uint64_t a3) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num), "D"(a1), "S"(a2), "d"(a3)
               : "memory");
  return ret;
}

inline int64_t hypercall_4(int num, uint64_t a1, uint64_t a2, uint64_t a3,
                           uint64_t a4) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num), "D"(a1), "S"(a2), "d"(a3),
               "c"(a4)
               : "memory");
  return ret;
}

inline int64_t hypercall_5(int num, uint64_t a1, uint64_t a2, uint64_t a3,
                           uint64_t a4, uint64_t a5) {
  int64_t ret;
  asm volatile(HYPERCALL : "=a"(ret) : "a"(num), "D"(a1), "S"(a2), "d"(a3),
               "c"(a4), "r"(a5)
               : "memory");
  return ret;
}

inline int64_t hypercall_6(int num, uint64_t a1, uint64_t a2, uint64_t a3,
                           uint64_t a4, uint64_t a5, uint64_t a6) {
  int64_t ret;
  asm volatile(HYPERCALL
               : "=a"(ret)
               : "a"(num), "D"(a1), "S"(a2), "d"(a3), "c"(a4), "r"(a5), "r"(a6)
               : "memory");
  return ret;
}
