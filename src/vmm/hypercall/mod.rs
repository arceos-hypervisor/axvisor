use core::sync::atomic::{AtomicUsize, Ordering};

use axaddrspace::{GuestPhysAddr, GuestVirtAddr, MappingFlags};
use axerrno::{AxResult, ax_err, ax_err_type};
use axhvc::{HyperCallCode, HyperCallResult};
use axvcpu::AxVcpuAccessGuestState;

use equation_defs::{GuestMappingType, InstanceType};
use memory_addr::PAGE_SIZE_4K;
use memory_addr::{MemoryAddr, PAGE_SIZE_2M};

use crate::libos::def::get_contents_from_shared_pages;
use crate::libos::instance;
use crate::libos::region;
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
            ),
            HyperCallCode::HLoadMMap => self.instance_load_mmap(
                self.args[0] as usize,
                self.args[1] as usize,
                self.args[2] as usize,
                self.args[3] as usize,
                self.args[4] as usize,
                self.args[5] as usize,
            ),
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
        shm_base_gpa_ptr: usize,
    ) -> HyperCallResult {
        info!(
            "HCreateInstance type {:?}, mapping type {:?}, shm_base_ptr {:#x}",
            instance_type, mapping_type, shm_base_gpa_ptr
        );
        let instance_id = instance::create_instance(instance_type, mapping_type)?;

        let shm_base = region::get_shm_region_by_instance_id(instance_id);

        info!("Instance [{instance_id}] host SHM region at {shm_base:#x}");
        let shm_base_gpa_ptr = GuestPhysAddr::from_usize(shm_base_gpa_ptr);
        self.vm.write_to_guest_of(shm_base_gpa_ptr, &shm_base)?;

        Ok(instance_id)
    }

    fn instance_load_mmap(
        &self,
        instance_id: usize,
        gva: usize,
        linux_gpa: usize,
        len: usize,
        flags: usize,
        prot: usize,
    ) -> HyperCallResult {
        info!(
            "Instance[{instance_id}] HLoadMmap addr:{gva:#x} gpa {linux_gpa:#x} len:{len:#x} flags:{flags:#x} prot:{prot:#x}",
        );
        let instance_ref = instance::get_instances_by_id(instance_id).ok_or_else(|| {
            warn!("Instance with ID {} not found", instance_id);
            ax_err_type!(InvalidInput, "Instance not found")
        })?;

        let page_index =
            (linux_gpa - region::get_shm_region_by_instance_id(instance_id)) / PAGE_SIZE_4K;
        let gpa_aligned = GuestPhysAddr::from_usize(linux_gpa).align_down(PAGE_SIZE_2M);
        // First, check if the 2MB region is mapped for the instance.
        let offset_of_granularity =
            region::count_2mb_region_offset(instance_id, gpa_aligned.as_usize())?;

        if !instance_ref.init_process_check_mm_region_allocated(offset_of_granularity) {
            let shm_base_hpa = instance_ref.init_process_alloc_mm_region()?;
            // Map the SHM region to the host Linux.
            self.vm
                .map_region(
                    gpa_aligned,
                    shm_base_hpa,
                    PAGE_SIZE_2M,
                    MappingFlags::READ | MappingFlags::WRITE,
                    true, // Allow huge pages
                )
                .map_err(|e| {
                    warn!("Failed to map SHM region: {:?}", e);
                    ax_err_type!(InvalidInput, "Failed to map SHM region")
                })?;
        }

        // TODO: handle map shared.

        let vm_flags = vm_flags::linux_mm_flags_to_mapping_flags(flags);
        let prot_flags = vm_flags::linux_page_prot_to_mapping_flags(prot);

        warn!(
            "Instance[{instance_id}] HLoadMmap vm_flags: {:?} prot_flags: {:?}",
            vm_flags, prot_flags
        );

        instance_ref.init_process_sync_mmap(
            GuestVirtAddr::from_usize(gva),
            page_index,
            len,
            prot_flags,
        )?;

        Ok(0)
    }

    fn setup_instance(&self, instance_id: usize, entry: usize, stack: usize) -> HyperCallResult {
        info!("HSetupInstance instance_id: {instance_id} entry: {entry:#x} stack: {stack:#x}");

        instance::get_instances_by_id(instance_id)
            .ok_or_else(|| {
                warn!("Instance with ID {} not found", instance_id);
                ax_err_type!(InvalidInput, "Instance not found")
            })?
            .setup_init_task(entry, stack)?;

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

    pub const PROT_NONE: usize = 0;
    pub const PROT_READ: usize = 1;
    pub const PROT_WRITE: usize = 2;
    pub const PROT_EXEC: usize = 4;

    pub fn linux_mm_flags_to_mapping_flags(flags: usize) -> MappingFlags {
        let mut mapping_flags = MappingFlags::from_bits_retain(VM_NONE);
        if flags & VM_READ != 0 {
            mapping_flags |= MappingFlags::READ;
        }
        if flags & VM_WRITE != 0 {
            mapping_flags |= MappingFlags::WRITE;
        }
        if flags & VM_EXEC != 0 {
            mapping_flags |= MappingFlags::EXECUTE;
        }

        mapping_flags |= MappingFlags::USER;

        mapping_flags
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
