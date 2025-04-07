#ifndef HYPERCALL_H
#define HYPERCALL_H

#include <stdint.h>

#define HYPERCALL "vmcall"

#define HVC_DEBUG 0xc0000000
#define HVC_CREATE_INSTANCE 0xc0000001
#define HVC_CREATE_INIT_PROCESS 0xc0000002
#define HVC_MMAP 0xc0000003
#define HVC_CLONE 0xc0000004

int64_t hypercall(int num);
int64_t hypercall_1(int num, uint64_t a1);
int64_t hypercall_2(int num, uint64_t a1, uint64_t a2);
int64_t hypercall_3(int num, uint64_t a1, uint64_t a2, uint64_t a3);
int64_t hypercall_4(int num, uint64_t a1, uint64_t a2, uint64_t a3,
                    uint64_t a4);
int64_t hypercall_5(int num, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4,
                    uint64_t a5);
int64_t hypercall_6(int num, uint64_t a1, uint64_t a2, uint64_t a3, uint64_t a4,
                    uint64_t a5, uint64_t a6);

#endif // HYPERCALL_H