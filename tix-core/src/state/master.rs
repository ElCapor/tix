use std::collections::HashMap;

use crate::Packet;

pub struct TixMasterState {
    requests: HashMap<u64, Packet>,
}
