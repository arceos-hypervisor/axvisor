use core::sync::atomic::{AtomicUsize, Ordering};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, MappingFlags};
use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;

use equation_defs::{GuestMappingType, InstanceType};
use equation_defs::{get_pgcache_region_by_instance_id, get_scf_queue_buff_region_by_instance_id};
use memory_addr::{
    MemoryAddr, PAGE_SIZE_1G, PAGE_SIZE_2M, PAGE_SIZE_4K, align_up, align_up_4k, is_aligned,
};
use page_table_multiarch::PageSize;

use crate::libos::def::get_contents_from_shared_pages;
use crate::libos::instance;
use crate::vmm::{VCpuRef, VMRef};

pub struct HyperCall {
    vcpu: VCpuRef,
    vm: VMRef,
    code: HyperCallCode,
    args: [u64; 6],
}

impl HyperCall {
    pub fn new(vcpu: VCpuRef, vm: VMRef, code: u64, args: [u64; 6]) -> AxResult<Self> {
        let code = HyperCallCode::try_from(code as u32).map_err(|e| {
            warn!("Invalid hypercall code: {} e {:?}", code, e);
            ax_err_type!(InvalidInput)
        })?;

        Ok(Self {
            vcpu,
            vm,
            code,
            args,
        })
    }

    pub fn execute(&self) -> HyperCallResult {
        // First, check if the vcpu is allowed to execute the hypercall.
        // if self.code.is_privileged() ^ self.vcpu.get_arch_vcpu().guest_is_privileged() {
        //     warn!(
        //         "{} vcpu trying to execute {} hypercall {:?}",
        //         if self.vcpu.get_arch_vcpu().guest_is_privileged() {
        //             "Privileged"
        //         } else {
        //             "Unprivileged"
        //         },
        //         if self.code.is_privileged() {
        //             "privileged"
        //         } else {
        //             "unprivileged"
        //         },
        //         self.code
        //     );
        //     return ax_err!(PermissionDenied);
        // }
        debug!("VMM Hypercall: {:?} args: {:x?}", self.code, self.args);

        if self.vcpu.get_arch_vcpu().guest_is_privileged() {
            self.execute_privileged()
        } else {
            self.execute_unprivileged()
        }
    }

    fn execute_privileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HypervisorDisable => self.hypervisor_disable(),
            HyperCallCode::HyperVisorDebug => self.debug(),
            HyperCallCode::HCreateInstance => self.create_instance(
                self.args[0].into(),
                self.args[1].into(),
                self.args[2] as usize,
                self.args[3] as usize,
                self.args[4] as usize,
                self.args[5] as usize,
            ),
            HyperCallCode::HIVCGet => self.ivc_get(
                self.args[0] as u32,
                self.args[1] as usize,
                self.args[2] as usize,
                self.args[3] as usize,
            ),
            HyperCallCode::HIVCDt => self.ivc_dt(self.args[0] as u32),
            _ => {
                unimplemented!()
            }
        }
    }

    fn execute_unprivileged(&self) -> HyperCallResult {
        match self.code {
            HyperCallCode::HDebug => self.debug(),
            HyperCallCode::HInitShim => self.init_shim(),
            HyperCallCode::HSetupInstance => self.setup_instance(
                self.args[0] as usize,
                self.args[1] as usize,
                self.args[2] as usize,
                self.args[3] as usize,
            ),
            HyperCallCode::HIVCSHMAt => self.ivc_shm_at(
                self.args[0] as usize,
                self.args[1] as u32,
                self.args[2] as usize,
                self.args[3] as usize,
                self.args[4] as usize,
            ),
            _ => {
                unimplemented!();
            }
        }
    }
}

impl HyperCall {
    fn hypervisor_disable(&self) -> HyperCallResult {
        info!("HypervisorDisable");

        let reserved_cpus = crate::vmm::config::get_reserved_cpus();

        static TRY_DISABLED_CPUS: AtomicUsize = AtomicUsize::new(0);

        if TRY_DISABLED_CPUS.fetch_add(1, Ordering::SeqCst) == 0 {
            // We need to disable virtualization on CPUs belonging to ArceOS,
            // then shutdown these CPUs.
            crate::hal::disable_virtualization_on_remaining_cores()?;

            // Add `1` to TRY_DISABLED_CPUS to indicate that virtualization on other CPUs
            // has been disabled.
            TRY_DISABLED_CPUS.fetch_add(1, Ordering::SeqCst);
        }

        // Wait for all CPUs to trgger the hypervisor disable HVC from Linux.
        // Wait for all other CPUs to disable virtualization.
        while TRY_DISABLED_CPUS.load(Ordering::SeqCst) < reserved_cpus + 1 {
            core::hint::spin_loop();
        }

        crate::hal::disable_virtualization(self.vcpu.clone(), 0)?;

        unreachable!("HypervisorDisable should not reach here");
    }

    fn debug(&self) -> HyperCallResult {
        info!("HDebug {:#x?}", self.args);

        self.vcpu.get_arch_vcpu().dump();

        Ok(HyperCallCode::HDebug as usize)
    }

    fn init_shim(&self) -> HyperCallResult {
        instance::init_shim()?;
        Ok(0)
    }

    fn create_instance(
        &self,
        instance_type: InstanceType,
        mapping_type: GuestMappingType,
        scf_base_gpa_ptr: usize,
        scf_size_gpa_ptr: usize,
        pgcache_base_gpa_ptr: usize,
        pgcache_size_gpa_ptr: usize,
    ) -> HyperCallResult {
        info!(
            "HCreateInstance type {:?}, mapping type {:?}, pgcache_base_gpa_ptr {:#x}",
            instance_type, mapping_type, pgcache_base_gpa_ptr
        );
        let instance_id = instance::create_instance(instance_type, mapping_type)?;
        let instance_ref = instance::get_instances_by_id(instance_id).ok_or_else(|| {
            warn!("Instance with ID {} not found", instance_id);
            ax_err_type!(InvalidInput, "Instance not found")
        })?;

        let scf_region_base = get_scf_queue_buff_region_by_instance_id(instance_id);
        let scf_region_base_gpa = GuestPhysAddr::from_usize(scf_region_base);
        let (scf_region_base_hpa, scf_region_size) =
            instance_ref.get_scf_queue_region().ok_or_else(|| {
                warn!(
                    "Failed to get SCF queue region for instance {}",
                    instance_id
                );
                ax_err_type!(InvalidInput, "Failed to get SCF queue region")
            })?;
        // Map the SCF buffer region to the host Linux.
        let _ = self
            .vm
            .map_region(
                scf_region_base_gpa,
                scf_region_base_hpa,
                scf_region_size,
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )
            .map_err(|e| {
                warn!("Failed to map SCF buffer region: {:?}", e);
                ax_err_type!(InvalidInput, "Failed to map SHM region")
            });
        let scf_base_gpa_ptr = GuestPhysAddr::from_usize(scf_base_gpa_ptr);
        let scf_size_gpa_ptr = GuestPhysAddr::from_usize(scf_size_gpa_ptr);
        self.vm
            .write_to_guest_of(scf_base_gpa_ptr, &scf_region_base)?;
        self.vm
            .write_to_guest_of(scf_size_gpa_ptr, &scf_region_size)?;

        let pgcache_base = get_pgcache_region_by_instance_id(instance_id);

        let pgcache_base_gpa = GuestPhysAddr::from_usize(pgcache_base);
        let (pgcache_base_hpa, pgcache_size) =
            instance_ref.get_page_cache_region().ok_or_else(|| {
                warn!(
                    "Failed to get page cache region for instance {}",
                    instance_id
                );
                ax_err_type!(InvalidInput, "Failed to get page cache region")
            })?;
        // Map the page cache region to the host Linux.
        let _ = self
            .vm
            .map_region(
                pgcache_base_gpa,
                pgcache_base_hpa,
                pgcache_size,
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )
            .map_err(|e| {
                warn!("Failed to map page cache region: {:?}", e);
                ax_err_type!(InvalidInput, "Failed to map page cache region")
            });

        info!(
            "Instance [{instance_id}] host pgcache region at [{:#x}~{:#x}]",
            pgcache_base,
            pgcache_base + pgcache_size
        );
        let pgcache_base_gpa_ptr = GuestPhysAddr::from_usize(pgcache_base_gpa_ptr);
        let pgcache_size_gpa_ptr = GuestPhysAddr::from_usize(pgcache_size_gpa_ptr);
        self.vm
            .write_to_guest_of(pgcache_base_gpa_ptr, &pgcache_base)?;
        self.vm
            .write_to_guest_of(pgcache_size_gpa_ptr, &pgcache_size)?;

        Ok(instance_id)
    }

    fn setup_instance(
        &self,
        instance_id: usize,
        file_size: usize,
        shared_pages_base_gva: usize,
        shared_pages_num: usize,
    ) -> HyperCallResult {
        info!(
            "HSetupInstance instance_id {}, file_size {}, shared_pages_base_gva {:#x}, shared_pages_num {}",
            instance_id, file_size, shared_pages_base_gva, shared_pages_num
        );

        let raw_args = get_contents_from_shared_pages(
            file_size,
            shared_pages_base_gva,
            shared_pages_num,
            &self.vcpu,
            &self.vm,
        )?;

        instance::get_instances_by_id(instance_id)
            .ok_or_else(|| {
                warn!("Instance with ID {} not found", instance_id);
                ax_err_type!(InvalidInput, "Instance not found")
            })?
            .setup_init_task(&raw_args)?;

        Ok(0)
    }
}

use crate::vmm::ivc::{self, IVCChannel, ShmFlags};

/// IVC related hypercalls.
impl HyperCall {
    /// Register a new IVC channel connected with the shm region registed by host Linux.
    /// This function will create a new IVC channel with the given key and size.
    ///
    /// The shmid is obtained by axcli daemon in host Linux,
    /// and the shm region is registered by axcli daemon in host Linux.
    ///
    /// Currently this IVC channel is used to setup a shared memory region between
    /// the host Linux and a guest instance, so a `instance_id` is required to identify the guest instance.
    fn ivc_shm_at(
        &self,
        instance_id: usize,
        shmkey: u32,
        addr: usize,
        size: usize,
        shmflg: usize,
    ) -> HyperCallResult {
        let host_gva = GuestVirtAddr::from_usize(addr);
        let (host_gpa, _gflags, pg_size) = self
            .vcpu
            .get_arch_vcpu()
            .guest_page_table_query(host_gva)
            .map_err(|err| {
                warn!(
                    "Failed to query guest page table for host GVA {:#x}: {:?}",
                    host_gva, err
                );
                ax_err_type!(InvalidInput, "Invalid guest virtual address")
            })?;

        if !is_aligned(size, pg_size as usize) {
            warn!(
                "Size {:#x} is not aligned to {:?} for gva {:?}",
                size, pg_size, host_gva
            );
            return ax_err!(InvalidInput, "Size is not aligned to page size");
        }

        let (hpa, hflags, _hpgsize) =
            self.vm.guest_phys_to_host_phys(host_gpa).ok_or_else(|| {
                warn!(
                    "Failed to convert guest physical address {:#x} to host physical address",
                    host_gpa
                );
                ax_err_type!(InvalidData, "Invalid guest physical address")
            })?;

        let flags = ShmFlags::from_bits_retain(shmflg);

        let alignment = if flags.contains(ShmFlags::SHM_HUGETLB) {
            if flags.contains(ShmFlags::SHM_HUGE_1GB) {
                // Huge pages are always 1GB, so we align the size to 1GB.
                PageSize::Size1G
            } else if flags.contains(ShmFlags::SHM_HUGE_2MB) {
                // Huge pages are always 2MB, so we align the size to 2MB.
                PageSize::Size2M
            } else {
                return ax_err!(InvalidInput, "Invalid huge page size for IVC channel");
            }
        } else {
            // Regular pages are 4KB, so we align the size to 4KB.
            PageSize::Size4K
        };

        if !host_gva.is_aligned(alignment as usize) {
            warn!(
                "Host GVA {:#x} is not aligned to {:?} for IVC channel",
                host_gva, alignment
            );
            return ax_err!(InvalidInput, "Host GVA is not aligned to page size");
        }

        // We allow that the host GVA page size is larger than the alignment.
        if alignment as usize > pg_size as usize {
            warn!(
                "Host GVA {:#x} page size {:#?} does not match alignment {:?}",
                host_gva, pg_size, alignment
            );
            return ax_err!(
                InvalidInput,
                "Host GVA page size does not match IVC channel alignment"
            );
        }

        let instance_ref = instance::get_instances_by_id(instance_id).ok_or_else(|| {
            warn!("Instance with ID {} not found", instance_id);
            ax_err_type!(InvalidInput, "Instance not found")
        })?;

        // Construct the IVC channel from the host shared memory region.
        let channel = IVCChannel::construct_from_shm(shmkey, size, hpa)?;
        // Insert the IVC channel into the global map.
        ivc::insert_channel(shmkey, channel, true)?;

        // Sync the shm mapping to the instance.
        let instance_gpa = instance_ref.init_ivc_shm_sync(shmkey, hflags, size, alignment)?;

        Ok(instance_gpa.as_usize())
    }

    fn ivc_get(
        &self,
        key: u32,
        size: usize,
        flags: usize,
        shm_base_gpa_ptr: usize,
    ) -> HyperCallResult {
        let vm_id = self.vm.id();
        let flags = ShmFlags::from_bits_retain(flags);

        let (size, alignment) = if flags.contains(ShmFlags::SHM_HUGETLB) {
            if flags.contains(ShmFlags::SHM_HUGE_1GB) {
                // Huge pages are always 1GB, so we align the size to 1GB.
                (align_up(size, PAGE_SIZE_1G), PAGE_SIZE_1G)
            } else if flags.contains(ShmFlags::SHM_HUGE_2MB) {
                // Huge pages are always 2MB, so we align the size to 2MB.
                (align_up(size, PAGE_SIZE_2M), PAGE_SIZE_2M)
            } else {
                return ax_err!(InvalidInput, "Invalid huge page size for IVC channel");
            }
        } else {
            // Regular pages are 4KB, so we align the size to 4KB.
            (align_up_4k(size), PAGE_SIZE_4K)
        };

        let shm_base_gpa_ptr = GuestPhysAddr::from_usize(shm_base_gpa_ptr);
        info!(
            "HIVCGet VM [{}], key {:#x}, size {:#x} alignment {:#x}",
            vm_id, key, size, alignment
        );
        info!(
            "HIVCGet VM [{}], flags {:?} raw_flags {:#x}",
            vm_id,
            flags,
            flags.bits()
        );

        let shm_base_gpa = self.vm.alloc_one_shm_region(size, alignment)?;

        if !shm_base_gpa.is_aligned(alignment) {
            error!(
                "HIVCGet VM [{}] failed to allocate shared memory region at {:#x} with size {:#x} alignment {:#x}",
                vm_id, shm_base_gpa, size, alignment
            );
            return ax_err!(NoMemory, "Failed to allocate shared memory region");
        }

        // Try to create a new IVC channel.
        if flags.contains(ShmFlags::IPC_CREAT) && !ivc::contains_channel(key) {
            // Create a new IVC channel.
            let mut channel = IVCChannel::allocate(key, size)?;

            self.vm.map_region(
                shm_base_gpa,
                channel.base_hpa(),
                channel.size(),
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )?;

            channel.add_subscriber(vm_id, shm_base_gpa, size);

            ivc::insert_channel(key, channel, false)?;
        } else {
            if flags.contains(ShmFlags::IPC_EXCL) && ivc::contains_channel(key) {
                warn!("IVC channel with key {:#x} already exists", key);
                return ax_err!(AlreadyExists, "IVC channel already exists");
            }
            // Subcribe to an existing IVC channel.
            let (base_hpa, actual_size) =
                ivc::subscribe_to_channel(key, vm_id, shm_base_gpa, size)?;
            self.vm.map_region(
                shm_base_gpa,
                base_hpa,
                actual_size,
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )?;
        }

        // Write the base GPA to the guest.
        self.vm
            .write_to_guest_of(shm_base_gpa_ptr, &shm_base_gpa.as_usize())?;

        Ok(0)
    }

    fn ivc_dt(&self, key: u32) -> HyperCallResult {
        info!("HIVCDt VM [{}], key {:#x}", self.vm.id(), key);

        let vm_id = self.vm.id();

        // Unsubscribe from the IVC channel.
        let (base_gpa, size) = ivc::unsubscribe_from_channel(key, vm_id)?;

        // Unmap the shared memory region from the guest.
        self.vm.unmap_region(base_gpa, size)?;

        Ok(0)
    }
}
#[allow(unused)]
mod vm_flags {
    use axaddrspace::MappingFlags;

    /*
     * vm_flags in vm_area_struct, see mm_types.h.
     * When changing, update also include/trace/events/mmflags.h
     * #define VM_NONE		0x00000000
     * #define VM_READ		0x00000001	/* currently active flags */
     * #define VM_WRITE	    0x00000002
     * #define VM_EXEC		0x00000004
     * #define VM_SHARED	0x00000008
     */
    const VM_NONE: usize = 0x00000000;
    const VM_READ: usize = 0x00000001;
    const VM_WRITE: usize = 0x00000002;
    const VM_EXEC: usize = 0x00000004;
    const VM_SHARED: usize = 0x00000008;

    pub const MAP_FILE: usize = 0x0000;
    pub const MAP_SHARED: usize = 0x0001;
    pub const MAP_PRIVATE: usize = 0x0002;
    pub const MAP_FIXED: usize = 0x0010;

    pub const PROT_NONE: usize = 0;
    pub const PROT_READ: usize = 1;
    pub const PROT_WRITE: usize = 2;
    pub const PROT_EXEC: usize = 4;

    pub fn linux_mm_flags_map_fixed(flags: usize) -> bool {
        flags & MAP_FIXED != 0
    }

    pub fn linux_mm_flags_map_shared(flags: usize) -> bool {
        flags & MAP_SHARED != 0
    }

    pub fn linux_mm_flags_map_private(flags: usize) -> bool {
        flags & MAP_PRIVATE != 0
    }

    pub fn linux_page_prot_to_mapping_flags(prot: usize) -> MappingFlags {
        let mut mapping_flags = MappingFlags::from_bits_retain(PROT_NONE);
        if prot & PROT_READ != 0 {
            mapping_flags |= MappingFlags::READ;
        }
        if prot & PROT_WRITE != 0 {
            mapping_flags |= MappingFlags::WRITE;
        }
        if prot & PROT_EXEC != 0 {
            mapping_flags |= MappingFlags::EXECUTE;
        }

        mapping_flags |= MappingFlags::USER;

        mapping_flags
    }
}
