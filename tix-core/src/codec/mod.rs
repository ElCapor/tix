pub struct TixCodec {

}

impl tokio_util::codec::Decoder for TixCodec{
    type Item = crate::Packet;
    type Error = crate::Error;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() > crate::MAX_FRAME_SIZE {
            return Err(crate::Error::from("Frame size exceeded"));
        }
        if src.len() < crate::HEADER_LENGTH {
            return Ok(None);
        } 

        let header = crate::TixHeader::from_bytes(src[..crate::HEADER_LENGTH].try_into().expect("Failed to build header"));
        if header.is_err() {
            return Err(header.err().expect("Failed to unwrap error"));
        }
        let header = header.unwrap();
        if src.len() < crate::HEADER_LENGTH + header.get_payload_length() as usize {
            return Ok(None);
        }
        if header.get_payload_length() > 0 && header.get_checksum() == 0 {
            return Err(crate::Error::from("Checksum must be non-zero"));
        }

        if header.get_payload_length() as usize > crate::MAX_PAYLOAD_SIZE {
            return Err(crate::Error::from("Payload length must be between 0 and MAX_PAYLOAD_SIZE"));
        }

        let payload = src.split_to(crate::HEADER_LENGTH + header.get_payload_length() as usize);
        let packet = crate::Packet::from_bytes(&payload).map_err(|e| crate::Error::from(e.to_string()))?;

        // only if there's a payload
        if packet.get_payload_length() > 0
        {
            //? Validate checksum before returning, disabled for now for debug purpose
            let valid = packet.validate().expect("Failed to validate packet");
            if !valid {
                return Err(crate::Error::from("Invalid checksum"));
            }
        }
        
        Ok(Some(packet))
    }

}

impl tokio_util::codec::Encoder<crate::Packet> for TixCodec {
    type Error = crate::Error;

    fn encode(&mut self, item: crate::Packet, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        let packet = item.to_bytes().map_err(|e| crate::Error::from(e.to_string()))?;
        dst.extend_from_slice(&packet);
        Ok(())
    }
}
