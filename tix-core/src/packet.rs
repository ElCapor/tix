use std::fmt::Debug;

use crate::MessageType;
use crate::Command;
use crate::Flag;
use crate::TixHeader;
use crate::header;
use crate::header::TixHeaderBytes;

pub struct Packet {
    header: TixHeader,
    payload: Vec<u8>,
}

impl Packet {
    pub fn heartbeat() -> Self {
        Self {
            header: TixHeader::new(0, MessageType::Command as u32, Flag::None as u64, Command::Ping as u64, 0, 0),
            payload: Vec::new(),
        }
    }
    pub fn new_command(request_id: u64, command: Command, payload: Vec<u8>) -> Result<Self, crate::Error> {
        if payload.len() > crate::MAX_PAYLOAD_SIZE {
            return Err(crate::Error::from("Payload length exceeds maximum allowed size".to_string()));
        }

        let mut header = TixHeader::new(0, MessageType::Command as u32, Flag::None as u64, command as u64, request_id, payload.len() as u64);

        // Calculate payload checksum with blake3
        let checksum = blake3::hash(&payload);
        header.set_checksum(u32::from_le_bytes(checksum.as_bytes()[0..4].try_into().expect("Failed to convert to bytes")));
        Ok(Self {
            header,
            payload,
        })
    }

    pub fn new_response(request_id: u64, command: Command, payload: Vec<u8>) -> Result<Self, crate::Error> {
        if payload.len() > crate::MAX_PAYLOAD_SIZE {
            return Err(crate::Error::from("Payload length exceeds maximum allowed size".to_string()));
        }

        let mut header = TixHeader::new(0, MessageType::Response as u32, Flag::None as u64, command as u64, request_id, payload.len() as u64);

        if payload.len() > 0
        {
            // Calculate payload checksum with blake3
            let checksum = blake3::hash(&payload);
            header.set_checksum(u32::from_le_bytes(checksum.as_bytes()[0..4].try_into().expect("Failed to convert to bytes")));
        }

        
        Ok(Self {
            header,
            payload,
        })
    }

    pub fn get_header(&self) -> &TixHeader {
        &self.header
    }

    pub fn get_payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn get_checksum(&self) -> u32 {
        self.header.get_checksum()
    }

    pub fn get_message_type(&self) -> MessageType {
        MessageType::from(self.header.get_message_type())
    }

    pub fn get_command(&self) -> Command {
        Command::from(self.header.get_command_id())
    }

    pub fn get_flags(&self) -> Flag {
        Flag::from(self.header.get_flags())
    }

    pub fn get_request_id(&self) -> u64 {
        self.header.get_request_id()
    }

    pub fn get_payload_length(&self) -> u64 {
        self.header.get_payload_length()
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::Error> {
        // In case it got tampered with
        if self.get_payload_length() > crate::MAX_PAYLOAD_SIZE as u64 {
            return Err(crate::Error::from("Payload length exceeds maximum allowed size".to_string()));
        }

        let mut packet = self.header.to_bytes().to_vec();
        packet.extend_from_slice(&self.payload);
        Ok(packet)
    }
    ///? Optimization : Take pre made header
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::Error> {
        if bytes.len() < header::HEADER_LENGTH {
            return Err(crate::Error::from("Packet bytes length is less than header length".to_string()));  
        }
        let header_bytes : TixHeaderBytes = bytes[0..header::HEADER_LENGTH].try_into().map_err(|e: std::array::TryFromSliceError| e.to_string())?;
        let header = TixHeader::from_bytes(header_bytes).map_err(|e| e.to_string())?;

        if (bytes.len() as u64) < (header::HEADER_LENGTH as u64 + header.get_payload_length()) {
            return Err(crate::Error::from("Packet bytes length is less than header length plus payload length".to_string()));  
        } else if (bytes.len() as u64) > (header::HEADER_LENGTH as u64 + header.get_payload_length()) {
            return Err(crate::Error::from("Packet bytes length is greater than header length plus payload length".to_string()));  
        }

        // In case it got tampered with
        if header.get_payload_length() > crate::MAX_PAYLOAD_SIZE as u64 {
            return Err(crate::Error::from("Payload length exceeds maximum allowed size".to_string()));
        }

        let payload = bytes[header::HEADER_LENGTH..].to_vec();
        Ok(Self {
            header,
            payload,
        })
    }

    pub fn validate(&self) -> Result<bool, crate::Error> {
        let checksum = self.get_checksum();
        let payload_checksum = blake3::hash(&self.payload);
        let payload_checksum = u32::from_le_bytes(payload_checksum.as_bytes()[0..4].try_into().expect("Failed to convert to bytes"));
        Ok(checksum == payload_checksum)
    }                                                                                                                                                                                                                                                                                                                                                       

}


impl Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packet")
            .field("header", &self.header)
            .field("payload", &self.payload)
            .finish()
    }
}


impl Clone for Packet {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            payload: self.payload.clone(),
        }
    }
}