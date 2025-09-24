use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicUsize, Ordering};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, GuestVirtAddrRange, MappingFlags};
use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;

use equation_defs::{GuestMappingType, InstanceType};
use equation_defs::{get_pgcache_region_by_instance_id, get_scf_queue_buff_region_by_instance_id};
use memory_addr::{MemoryAddr, is_aligned};
use page_table_multiarch::PageSize;

use crate::libos::def::get_contents_from_shared_pages;
use crate::libos::instance;
use crate::libos::npt_mapping::GuestNestedMapping;
use crate::vmm::{VCpuRef, VM, VMRef};

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
            HyperCallCode::CreateVM => self.create_vm(self.args[0] as usize),
            HyperCallCode::BootVM => self.boot_vm(self.args[0] as usize),
            HyperCallCode::HCreateInstance => self.create_instance(
                self.args[0].into(),
                self.args[1].into(),
                self.args[2] as usize,
                self.args[3] as usize,
                self.args[4] as usize,
                self.args[5] as usize,
            ),
            _ => {
                error!("Privileged hypercall {:?} not implemented", self.code);
                Err(ax_err_type!(InvalidInput, "Hypercall not implemented"))
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
                error!("Unprivileged hypercall {:?} not implemented", self.code);
                Err(ax_err_type!(InvalidInput, "Hypercall not implemented"))
            }
        }
    }
}

impl HyperCall {
    fn hypervisor_disable(&self) -> HyperCallResult {
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

    fn create_vm(&self, arg_base_gpa: usize) -> HyperCallResult {
        use axhvc::AxHVCCreateVMArg;
        use axvm::config::{AxVMConfig, AxVMCrateConfig};
        use page_table_multiarch::PagingHandler;
        use std::os::arceos::modules::axhal::paging::PagingHandlerImpl;

        use crate::vmm::vm_list::push_vm;

        let arg_base_hpa = self
            .vm
            .guest_phys_to_host_phys(GuestPhysAddr::from_usize(arg_base_gpa))
            .ok_or_else(|| {
                warn!(
                    "Failed to convert guest physical address {:#x} to host physical address",
                    arg_base_gpa
                );
                ax_err_type!(InvalidData, "Invalid guest physical address")
            })?
            .0;

        let arg_base_hva = PagingHandlerImpl::phys_to_virt(arg_base_hpa);

        let vm_create_arg = unsafe { arg_base_hva.as_mut_ptr_of::<AxHVCCreateVMArg>().as_mut() }
            .ok_or_else(|| {
                error!(
                    "Failed to get mutable reference to AxHVCCreateVMArg at HVA {:#x}",
                    arg_base_hva
                );
                ax_err_type!(InvalidData, "Invalid VM create argument")
            })?;

        info!("Create VM with arg: {:#x?}", vm_create_arg);

        let config_file = self.vm.read_from_guest_of_slice::<u8>(
            GuestPhysAddr::from_usize(vm_create_arg.cfg_file_gpa as usize),
            vm_create_arg.cfg_file_size as usize,
        )?;

        let config_file_str = core::str::from_utf8(&config_file).map_err(|e| {
            warn!("Failed to parse VM config file as UTF-8: {:?}", e);
            ax_err_type!(InvalidData, "Invalid VM config file")
        })?;

        let vm_create_config =
            AxVMCrateConfig::from_toml(config_file_str).expect("Failed to resolve VM config");

        info!("VM Create Config: {:#x?}", vm_create_config);

        let vm_config = AxVMConfig::from(vm_create_config.clone());

        info!("Creating VM [{}] {:#x?}", vm_config.id(), vm_config);

        let vm = VM::new(vm_config).expect("Failed to create VM");
        push_vm(vm.clone());

        info!(
            "VM[{}] created success, setup EPT mapping for image loading",
            vm.id()
        );

        // Setup EPT mapping for loading kernel, bios and ramdisk.
        let kernel_image_gpa_base =
            GuestPhysAddr::from_usize(vm_create_config.kernel.kernel_load_addr as usize);
        let kernel_load_hpa_pairs = vm.translate_guest_memory_range(
            kernel_image_gpa_base,
            vm_create_arg.kernel_image_size as usize,
        )?;

        let host_kernel_img_load_gpa_base = GuestPhysAddr::from(kernel_load_hpa_pairs[0].0);
        let mut gpa_base = host_kernel_img_load_gpa_base;
        for (hpa_base, size) in &kernel_load_hpa_pairs {
            let gpa_base_aligned = gpa_base.align_down_4k();
            let hpa_base_aligned = hpa_base.align_down_4k();
            let gpa_end = gpa_base.add(*size);

            let gpa_end_aligned = gpa_end.align_up_4k();
            let aligned_size = gpa_end_aligned.as_usize() - gpa_base_aligned.as_usize();
            warn!(
                "Mapping kernel image region: gpa {:?} -> hpa {:?}, size {:#x}",
                gpa_base_aligned, hpa_base_aligned, aligned_size
            );
            self.vm.map_region(
                gpa_base_aligned,
                hpa_base_aligned,
                aligned_size,
                MappingFlags::READ | MappingFlags::WRITE,
                true, // Allow huge pages
            )?;
            gpa_base = gpa_base.add(*size);
        }

        let host_bios_img_load_gpa_base =
            if let Some(bios_load_addr) = vm_create_config.kernel.bios_load_addr {
                let bios_load_gpa_base = GuestPhysAddr::from_usize(bios_load_addr);
                let bios_load_hpa_pairs = vm.translate_guest_memory_range(
                    bios_load_gpa_base,
                    vm_create_arg.bios_image_size as usize,
                )?;

                if bios_load_hpa_pairs.is_empty() {
                    return ax_err!(InvalidInput, "No BIOS image mapping found");
                }

                let host_bios_img_load_gpa_base = GuestPhysAddr::from(bios_load_hpa_pairs[0].0);
                let mut gpa_base = host_bios_img_load_gpa_base;
                for (hpa_base, size) in &bios_load_hpa_pairs {
                    warn!(
                        "Mapping bios image region: gpa {:?} -> hpa {:?}, size {:#x}",
                        gpa_base, hpa_base, *size
                    );

                    let gpa_base_aligned = gpa_base.align_down_4k();
                    let hpa_base_aligned = hpa_base.align_down_4k();

                    let gpa_end = gpa_base.add(*size);

                    let gpa_end_aligned = gpa_end.align_up_4k();
                    let aligned_size = gpa_end_aligned.as_usize() - gpa_base_aligned.as_usize();

                    warn!(
                        "Mapping bios image region aligned: gpa {:?} -> hpa {:?}, size {:#x}",
                        gpa_base_aligned, hpa_base_aligned, aligned_size
                    );
                    self.vm.map_region(
                        gpa_base_aligned,
                        hpa_base_aligned,
                        aligned_size,
                        MappingFlags::READ | MappingFlags::WRITE,
                        true, // Allow huge pages
                    )?;
                    gpa_base = gpa_base.add(*size);
                }
                host_bios_img_load_gpa_base
            } else {
                GuestPhysAddr::from_usize(0)
            };

        let host_ramdisk_img_load_gpa_base = if let Some(ramdisk_load_addr) =
            vm_create_config.kernel.ramdisk_load_addr
        {
            let ramdisk_load_gpa_base = GuestPhysAddr::from_usize(ramdisk_load_addr);

            let ramdisk_load_hpa_pairs = vm.translate_guest_memory_range(
                ramdisk_load_gpa_base,
                vm_create_arg.ramdisk_image_size as usize,
            )?;

            let host_ramdisk_img_load_gpa_base = GuestPhysAddr::from(ramdisk_load_hpa_pairs[0].0);
            let mut gpa_base = host_ramdisk_img_load_gpa_base;
            for (hpa_base, size) in &ramdisk_load_hpa_pairs {
                let gpa_base_aligned = gpa_base.align_down_4k();
                let hpa_base_aligned = hpa_base.align_down_4k();
                let gpa_end = gpa_base.add(*size);

                let gpa_end_aligned = gpa_end.align_up_4k();
                let aligned_size = gpa_end_aligned.as_usize() - gpa_base_aligned.as_usize();
                warn!(
                    "Mapping ramdisk image region: gpa {:?} -> hpa {:?}, size {:#x}",
                    gpa_base_aligned, hpa_base_aligned, aligned_size
                );
                self.vm.map_region(
                    gpa_base_aligned,
                    hpa_base_aligned,
                    aligned_size,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true, // Allow huge pages
                )?;
                gpa_base = gpa_base.add(*size);
            }
            host_ramdisk_img_load_gpa_base
        } else {
            GuestPhysAddr::from_usize(0)
        };

        vm_create_arg.vm_id = vm.id() as u64;
        vm_create_arg.kernel_load_gpa = host_kernel_img_load_gpa_base.as_usize() as u64;
        vm_create_arg.bios_load_gpa = host_bios_img_load_gpa_base.as_usize() as u64;
        vm_create_arg.ramdisk_load_gpa = host_ramdisk_img_load_gpa_base.as_usize() as u64;

        warn!("VM Create Arg after setup: {:#x?}", vm_create_arg);

        info!("VM[{}] created success", vm.id());

        Ok(vm.id())
    }

    fn boot_vm(&self, vm_id: usize) -> HyperCallResult {
        crate::vmm::boot_vm(vm_id)?;

        Ok(0)
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
        info!(
            "HIVCSHMAt instance_id {}, shmkey {:#x}, host_gva {:#x}, size {:#x}, shmflg {:#x}",
            instance_id, shmkey, addr, size, shmflg
        );

        let host_gva = GuestVirtAddr::from_usize(addr);
        let flags = ShmFlags::from_bits_retain(shmflg);

        // Get alignment from `shmflg`.
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

        // Check alignment.
        if !host_gva.is_aligned(alignment as usize) {
            warn!(
                "Host GVA {:#x} is not aligned to {:?} for IVC channel",
                host_gva, alignment
            );
            return ax_err!(InvalidInput, "Host GVA is not aligned to page size");
        }

        let mut npt_mappings = BTreeMap::new();

        let base_gva = GuestVirtAddr::from_usize(addr);
        let end_gva = GuestVirtAddr::from_usize(addr + size);
        let mut gva = base_gva;

        while gva < end_gva {
            let (gpa, _gflags, gpgsize) = self
                .vcpu
                .get_arch_vcpu()
                .guest_page_table_query(gva)
                .map_err(|err| {
                    error!(
                        "Failed to query guest page table for host GVA {:#x}: {:?}",
                        gva, err
                    );
                    ax_err_type!(InvalidInput, "Invalid guest virtual address")
                })?;

            if !is_aligned(gpa.as_usize(), alignment as usize) {
                error!(
                    "Host GVA {:?} map tp {:?} does not match alignment {:?}",
                    gva, gpa, alignment
                );
            }
            let (hpa, _hflags, hpgsize) =
                self.vm.guest_phys_to_host_phys(gpa).ok_or_else(|| {
                    warn!(
                        "Failed to convert guest physical address {:#x} to host physical address",
                        gpa
                    );
                    ax_err_type!(InvalidData, "Invalid guest physical address")
                })?;

            let npt_mapping = GuestNestedMapping::new(gva, gpa, gpgsize, hpa, hpgsize);
            npt_mappings.insert(gva, npt_mapping);

            if gva.add(gpgsize as usize) == end_gva {
                // The full range is mapped.
                break;
            } else if gva.add(gpgsize as usize) > end_gva {
                error!(
                    "Host GVA range [{:?}~{:?}] exceeds the mapping range [{:?}~{:?}]",
                    gva,
                    gva.add(gpgsize as usize),
                    base_gva,
                    end_gva
                );
                break;
            }
            gva = gva.add(gpgsize as usize);
        }

        let instance_ref = instance::get_instances_by_id(instance_id).ok_or_else(|| {
            warn!("Instance with ID {} not found", instance_id);
            ax_err_type!(InvalidInput, "Instance not found")
        })?;

        let host_gva_range = GuestVirtAddrRange::from_start_size(base_gva, size);

        // Construct the IVC channel from the host shared memory region.
        let channel = IVCChannel::construct_from_shm(shmkey, host_gva_range, size, npt_mappings)?;
        // Insert the IVC channel into the global map.
        ivc::insert_channel(shmkey, channel, true)?;

        // Sync the shm mapping to the instance.
        let instance_gpa = instance_ref.init_ivc_shm_sync(shmkey, alignment)?;

        Ok(instance_gpa.as_usize())
    }

    // fn ivc_dt(&self, key: u32) -> HyperCallResult {
    //     info!("HIVCDt VM [{}], key {:#x}", self.vm.id(), key);

    //     let vm_id = self.vm.id();

    //     // Unsubscribe from the IVC channel.
    //     let (base_gpa, size) = ivc::unsubscribe_from_channel(key, vm_id)?;

    //     // Unmap the shared memory region from the guest.
    //     self.vm.unmap_region(base_gpa, size)?;

    //     Ok(0)
    // }
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
