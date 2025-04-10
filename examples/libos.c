#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include "include/hypercall.h"
#include "include/shm.h"

static void in_guest()
{
	printf("Execute VMCALL OK.\n");
	printf("You are in the Guest mode.\n");
}

static void in_host()
{
	printf("Execute VMCALL failed.\n");
	printf("You are in the Host mode.\n");
	exit(1);
}

static void sig_handler(int signum)
{
	printf("Caught signal %d\n", signum);
	in_host();
}

static void create_instance()
{
	int res;

	res = hypercall_4(
		HVC_CREATE_INSTANCE, getpid(), get_memory_regions_total_count(),
		(uint64_t)get_memory_regions_page_base(),
		get_memory_regions_page_count());

	if (res == 0)
	{
		printf("Create instance success.\n");

		if (!hypercall_2(HVC_CREATE_INIT_PROCESS, getpid(), 0))
		{
			printf("Create init process success.\n");
		}
		else
		{
			printf("Failed to create init process.\n");
		}
	}
	else if (res > 0)
	{
		for (;;)
		{
		}
		printf("Create instance res %d > 0.\n", res);
	}
	else
	{
		for (;;)
		{
		}
		printf("Create instance res %d < 0.\n", res);
	}
	cleanup_pages();
}

int main()
{
	signal(SIGSEGV, sig_handler);
	signal(SIGILL, sig_handler);
	int ret = hypercall(HVC_DEBUG);
	if (ret == HVC_DEBUG)
	{
		in_guest();
	}
	else
	{
		in_host();
	}

	parse_proc_self_maps();
	print_regions();
	create_instance();

	return 0;
}
