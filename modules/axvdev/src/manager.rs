use alloc::collections::btree_map::BTreeMap;

use crate::VDevice;

#[derive(Default)]
pub struct VDeviceManager {
    devices: BTreeMap<usize, VDevice>,
}
