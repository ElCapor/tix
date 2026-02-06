#[repr(u64)]
pub enum Flag {
    None = 0x0,
}


impl From<u64> for Flag {
    fn from(value: u64) -> Self {
        match value {
            0x0 => Flag::None,
            _ => panic!("Invalid Flag value"),
        }
    }
}
