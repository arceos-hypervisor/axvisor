use alloc::{collections::btree_map::BTreeMap, sync::Arc};

use id_arena::Arena;
use spin::Mutex;
use vdev_if::VirtDeviceOp;

use crate::VDevice;

pub type VDeviceId = id_arena::Id<VDevice>;

#[derive(Clone)]
pub struct VDeviceManager(Arc<Mutex<Inner>>);

impl VDeviceManager {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            devices: Arena::new(),
        })))
    }
}

struct Inner {
    devices: Arena<VDevice>,
}

impl VDeviceManager {
    pub fn add_device(&mut self, device: impl VirtDeviceOp) -> VDeviceId {
        self.0
            .lock()
            .devices
            .alloc_with_id(|id| VDevice::new(id, device))
    }
}
