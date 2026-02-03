use alloc::boxed::Box;
use vdev_if::VirtDeviceOp;

pub struct VDevice {
    raw: Box<dyn VirtDeviceOp>,
}

impl <T: VirtDeviceOp>From<T> for VDevice {
    fn from(dev:  T) -> Self {
        Self { raw: Box::new(dev) }
    }
}
