use alloc::boxed::Box;
use vdev_if::VirtDeviceOp;

use crate::VDeviceId;

pub struct VDevice {
    id: VDeviceId,
    raw: Box<dyn VirtDeviceOp>,
}

impl VDevice {
    pub fn new(id: VDeviceId, raw: impl VirtDeviceOp) -> Self {
        Self {
            id,
            raw: Box::new(raw),
        }
    }

    pub fn id(&self) -> VDeviceId {
        self.id
    }
}
