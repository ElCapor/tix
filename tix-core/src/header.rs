use crate::Error;

#[repr(C)]
pub struct TixHeader {
    magic: u32,
    checksum: u32,
    message_type: u32,
    flags: u64,
    command_id: u64,
    request_id: u64,
    payload_length: u64,
}

impl Clone for TixHeader {
    fn clone(&self) -> Self {
        Self {
            magic: self.magic,
            checksum: self.checksum,
            message_type: self.message_type,
            flags: self.flags,
            command_id: self.command_id,
            request_id: self.request_id,
            payload_length: self.payload_length,
        }
    }
}

pub type TixHeaderBytes = [u8; std::mem::size_of::<TixHeader>()];
pub const HEADER_LENGTH: usize = std::mem::size_of::<TixHeader>();

impl TixHeader {
    pub fn new(
        checksum: u32,
        message_type: u32,
        flags: u64,
        command_id: u64,
        request_id: u64,
        payload_length: u64,
    ) -> Self {
        Self {
            magic: u32::from_le_bytes(*b"TIX0"),
            checksum,
            message_type,
            flags,
            command_id,
            request_id,
            payload_length,
        }
    }

    pub fn to_bytes(&self) -> TixHeaderBytes {
        let mut packet: TixHeaderBytes = [0; std::mem::size_of::<TixHeader>()];
        packet[0..4].copy_from_slice(&self.magic.to_le_bytes());
        packet[4..8].copy_from_slice(&self.checksum.to_le_bytes());
        packet[8..12].copy_from_slice(&self.message_type.to_le_bytes());
        packet[12..20].copy_from_slice(&self.flags.to_le_bytes());
        packet[20..28].copy_from_slice(&self.command_id.to_le_bytes());
        packet[28..36].copy_from_slice(&self.request_id.to_le_bytes());
        packet[36..44].copy_from_slice(&self.payload_length.to_le_bytes());
        packet
    }

    pub fn from_bytes(bytes: TixHeaderBytes) -> Result<Self, Error> {
        if bytes[0..4] != *b"TIX0" {
            return Err(Error::new("Invalid magic bytes"));
        } else {
            Ok(Self {
                magic: u32::from_le_bytes(
                    bytes[0..4].try_into().expect("Failed to convert to bytes"),
                ),
                checksum: u32::from_le_bytes(
                    bytes[4..8].try_into().expect("Failed to convert to bytes"),
                ),
                message_type: u32::from_le_bytes(
                    bytes[8..12].try_into().expect("Failed to convert to bytes"),
                ),
                flags: u64::from_le_bytes(
                    bytes[12..20]
                        .try_into()
                        .expect("Failed to convert to bytes"),
                ),
                command_id: u64::from_le_bytes(
                    bytes[20..28]
                        .try_into()
                        .expect("Failed to convert to bytes"),
                ),
                request_id: u64::from_le_bytes(
                    bytes[28..36]
                        .try_into()
                        .expect("Failed to convert to bytes"),
                ),
                payload_length: u64::from_le_bytes(
                    bytes[36..44]
                        .try_into()
                        .expect("Failed to convert to bytes"),
                ),
            })
        }
    }

    pub fn get_checksum(&self) -> u32 {
        self.checksum
    }

    pub fn set_checksum(&mut self, checksum: u32) {
        self.checksum = checksum;
    }

    pub fn get_message_type(&self) -> u32 {
        self.message_type
    }

    pub fn get_flags(&self) -> u64 {
        self.flags
    }

    pub fn get_command_id(&self) -> u64 {
        self.command_id
    }

    pub fn get_payload_length(&self) -> u64 {
        self.payload_length
    }

    pub fn get_request_id(&self) -> u64 {
        self.request_id
    }
}


impl std::fmt::Debug for TixHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TixHeader")
            .field("message_type", &self.message_type)
            .field("flags", &self.flags)
            .field("command_id", &self.command_id)
            .field("request_id", &self.request_id)
            .field("payload_length", &self.payload_length)
            .field("checksum", &self.checksum)
            .finish()
    }
}