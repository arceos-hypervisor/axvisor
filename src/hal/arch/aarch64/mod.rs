mod api;
pub mod cache;

pub fn inject_interrupt(irq: usize) {
    debug!("Injecting virtual interrupt: {}", irq);

    let mut gic = rdrive::get_one::<rdrive::driver::Intc>()
        .expect("Failed to get GIC driver")
        .lock()
        .unwrap();
    if let Some(gic) = gic.typed_mut::<arm_gic_driver::v2::Gic>() {
        use arm_gic_driver::{
            IntId,
            v2::{VirtualInterruptConfig, VirtualInterruptState},
        };

        let gich = gic.hypervisor_interface().expect("Failed to get GICH");
        gich.enable();
        gich.set_virtual_interrupt(
            0,
            VirtualInterruptConfig::software(
                unsafe { IntId::raw(irq as _) },
                None,
                0,
                VirtualInterruptState::Pending,
                false,
                true,
            ),
        );
        return;
    }

    if let Some(_gic) = gic.typed_mut::<arm_gic_driver::v3::Gic>() {
        inject_interrupt_gic_v3(irq as _);
        return;
    }

    panic!("no gic driver found")
}

pub fn inject_interrupt_gic_v3(vector: usize) {
    use arm_gic_driver::v3::*;

    debug!("Injecting virtual interrupt: vector={}", vector);
    let elsr = ICH_ELRSR_EL2.read(ICH_ELRSR_EL2::STATUS);
    let lr_num = ICH_VTR_EL2.read(ICH_VTR_EL2::LISTREGS) as usize + 1;

    let mut free_lr = -1 as isize;

    // First, check if this interrupt is already pending/active
    for i in 0..lr_num {
        // find a free list register
        if (1 << i) & elsr > 0 {
            if free_lr == -1 {
                free_lr = i as isize;
            }
            continue;
        }
        let lr_val = ich_lr_el2_get(i);

        if lr_val.read(ICH_LR_EL2::VINTID) == vector as u64 {
            if lr_val.matches_any(&[ICH_LR_EL2::STATE::Pending, ICH_LR_EL2::STATE::Active]) {}
            debug!(
                "Virtual interrupt {} already pending/active in LR{}, skipping",
                vector, i
            );
            // If the interrupt is already pending or active, we can skip injecting it again.
            // This is important to avoid duplicate injections.
            return; // already injected
        }
    }

    debug!("use free lr {} to inject irq {}", free_lr, vector);

    if free_lr == -1 {
        warn!(
            "No free list register to inject IRQ {}, checking ICH_HCR_EL2",
            vector
        );

        // Try to find and reuse an inactive LR
        for i in 0..lr_num {
            let lr_val = ich_lr_el2_get(i);
            if lr_val.matches_any(&[ICH_LR_EL2::STATE::Invalid]) {
                debug!("Reusing inactive LR{} for IRQ {}", i, vector);
                free_lr = i as isize;

                break;
            }
        }

        if free_lr == -1 {
            panic!("No free list register to inject IRQ {}", vector);
        }
    }

    ich_lr_el2_write(
        free_lr as _,
        ICH_LR_EL2::VINTID.val(vector as u64) + ICH_LR_EL2::STATE::Pending + ICH_LR_EL2::GROUP::SET,
    );

    // Ensure the virtual interrupt interface is enabled
    let en = ICH_HCR_EL2.is_set(ICH_HCR_EL2::EN);
    if !en {
        // Check EN bit
        warn!("Virtual interrupt interface not enabled, enabling now");
        ICH_HCR_EL2.modify(ICH_HCR_EL2::EN::SET);
    }

    debug!(
        "Virtual interrupt {} injected successfully in LR{}",
        vector, free_lr
    );
}
