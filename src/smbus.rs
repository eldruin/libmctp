//! The SMBus specific protocol implementation.
//!
//! This should be used when you want to communicate via MCTP over SMBus/I2C.
//!
//! In order to use this you first need to crete the main context struct. Then
//! the `get_request()`/`get_response()` functions can be used to issue raw
//! commands.
//!
//! The libmctp library will not send the packets, instead it will create a
//! buffer containing the data to be sent. This allows you to use your own
//! SMBus/I2C implementation.
//!
//! ```rust
//!     use libmctp::control_packet::MCTPVersionQuery;
//!     use libmctp::smbus::MCTPSMBusContext;
//!
//!     const MY_ID: u8 = 0x23;
//!     let ctx = MCTPSMBusContext::new(MY_ID);
//!
//!     let mut buf: [u8; 32] = [0; 32];
//!
//!     const DEST_ID: u8 = 0x34;
//!     let len = ctx.get_request().get_mctp_version_support(
//!         DEST_ID,
//!         MCTPVersionQuery::MCTPBaseSpec,
//!         &mut buf,
//!     );
//!
//!     // Send the buf of length len via SMBus
//! ```
//!
//! Packets can be decoded using the `decode_packet()` function.

use crate::base_packet::{
    MCTPMessageBody, MCTPMessageBodyHeader, MCTPTransportHeader, MessageType,
};
use crate::control_packet::{CompletionCode, MCTPControlMessageHeader};
use crate::errors::{ControlMessageError, DecodeError};
use crate::mctp_traits::MCTPControlMessageRequest;
use crate::smbus_proto::{MCTPSMBusHeader, MCTPSMBusPacket};
use crate::smbus_request::MCTPSMBusContextRequest;
use crate::smbus_response::MCTPSMBusContextResponse;

/// The global context for MCTP SMBus operations
pub struct MCTPSMBusContext {
    request: MCTPSMBusContextRequest,
    response: MCTPSMBusContextResponse,
}

impl MCTPSMBusContext {
    /// Create a new SBMust context
    ///
    /// `address`: The source address of this device
    pub fn new(address: u8) -> Self {
        Self {
            request: MCTPSMBusContextRequest::new(address),
            response: MCTPSMBusContextResponse::new(address),
        }
    }

    /// Get the underlying request protocol struct.
    /// This can be used to generate specific packets
    pub fn get_request(&self) -> &MCTPSMBusContextRequest {
        &self.request
    }

    /// Get the underlying response protocol struct.
    /// This can be used to generate specific packets
    pub fn get_response(&self) -> &MCTPSMBusContextResponse {
        &self.response
    }

    fn get_smbus_headers<'a>(
        &self,
        packet: &'a [u8],
    ) -> Result<
        (
            MCTPSMBusHeader<[u8; 4]>,
            MCTPTransportHeader<[u8; 4]>,
            MCTPMessageBodyHeader<[u8; 1]>,
        ),
        (MessageType, DecodeError),
    > {
        // packet is a MCTPSMBusPacket
        let mut smbus_header_buf: [u8; 4] = [0; 4];
        smbus_header_buf.copy_from_slice(&packet[0..4]);
        let smbus_header = MCTPSMBusHeader::new_from_buf(smbus_header_buf);

        let mut base_header_buf: [u8; 4] = [0; 4];
        base_header_buf.copy_from_slice(&packet[4..8]);
        let base_header = MCTPTransportHeader::new_from_buf(base_header_buf);

        let body_header = MCTPMessageBodyHeader::new_from_buf([packet[8]]);

        Ok((smbus_header, base_header, body_header))
    }

    /// Decodes a MCTP packet
    pub fn decode_packet<'a>(
        &self,
        packet: &'a [u8],
    ) -> Result<(MessageType, &'a [u8]), (MessageType, DecodeError)> {
        let (smbus_header, base_header, body_header) = self.get_smbus_headers(packet)?;

        match body_header.msg_type().into() {
            MessageType::MCtpControl => {
                self.decode_mctp_control(&smbus_header, &base_header, &body_header, &packet[9..])
            }
            MessageType::VendorDefinedPCI => unimplemented!(),
            MessageType::VendorDefinedIANA => unimplemented!(),
            _ => Err((MessageType::Invalid, DecodeError::Unknown)),
        }
    }

    fn get_mctp_control_packet<'a, 'b>(
        &self,
        _smbus_header: &MCTPSMBusHeader<[u8; 4]>,
        _base_header: &MCTPTransportHeader<[u8; 4]>,
        body_header: &'b MCTPMessageBodyHeader<[u8; 1]>,
        packet: &'a [u8],
    ) -> Result<MCTPMessageBody<'a, 'b>, (MessageType, DecodeError)> {
        // Decode the header
        let mut control_message_header_buf: [u8; 2] = [0; 2];
        control_message_header_buf.copy_from_slice(&packet[0..2]);
        let control_message_header =
            MCTPControlMessageHeader::new_from_buf(control_message_header_buf);

        let (payload_offset, body_additional_header_len) = match control_message_header.rq() {
            1 => {
                // Request
                (2, control_message_header.get_request_data_len())
            }
            0 => {
                // Response
                if packet[2] != CompletionCode::Success as u8 {
                    return Err((
                        MessageType::MCtpControl,
                        DecodeError::ControlMessage(
                            ControlMessageError::UnsuccessfulCompletionCode(packet[2].into()),
                        ),
                    ));
                }
                (3, control_message_header.get_response_data_len())
            }
            _ => {
                return Err((
                    MessageType::MCtpControl,
                    DecodeError::ControlMessage(ControlMessageError::InvalidControlHeader),
                ));
            }
        };

        let data = &packet[payload_offset..];

        if data.len() != body_additional_header_len {
            return Err((
                MessageType::MCtpControl,
                DecodeError::ControlMessage(ControlMessageError::InvalidRequestDataLength),
            ));
        }

        Ok(MCTPMessageBody::new(
            body_header,
            Some(&packet[0..2]),
            data,
            None,
        ))
    }

    /// Decodes a MCTP request packet
    fn decode_mctp_control<'a, 'b>(
        &self,
        smbus_header: &MCTPSMBusHeader<[u8; 4]>,
        base_header: &MCTPTransportHeader<[u8; 4]>,
        body_header: &'b MCTPMessageBodyHeader<[u8; 1]>,
        packet: &'a [u8],
    ) -> Result<(MessageType, &'a [u8]), (MessageType, DecodeError)> {
        let body = self.get_mctp_control_packet(smbus_header, base_header, body_header, packet)?;
        Ok((MessageType::MCtpControl, body.data))
    }

    /// This function first decodes the packet supplied in the `packet` argument.
    /// This is done using the `decode_packet()` function.
    /// If this packet is a request then the `response_buf` is populated with
    /// a response to the request.
    ///
    /// On success the first two arguments in the `Ok()` result are the same as
    /// the return from the `decode_packet()` function. The third argument is
    /// an option. If `None` then `response_buf` wasn't changed because the
    /// `packet` was not a request. If `Some` it contains the length of the
    /// data written in the `response_buf`.
    pub fn process_packet<'a, 'b>(
        &self,
        packet: &'a [u8],
        _response_buf: &'b [u8],
    ) -> Result<(MessageType, &'a [u8], Option<usize>), (MessageType, DecodeError)> {
        let (msg_type, payload) = self.decode_packet(packet)?;

        match msg_type {
            MessageType::MCtpControl => {
                let (mut smbus_header, base_header, body_header) =
                    self.get_smbus_headers(packet)?;
                let body = self.get_mctp_control_packet(
                    &smbus_header,
                    &base_header,
                    &body_header,
                    packet,
                )?;
                let _packet = MCTPSMBusPacket::new(&mut smbus_header, &base_header, &body);

                Ok((msg_type, payload, None))
            }
            MessageType::VendorDefinedPCI => {
                // Vendor defined, we don't know what to do
                Ok((msg_type, payload, None))
            }
            MessageType::VendorDefinedIANA => {
                // Vendor defined, we don't know what to do
                Ok((msg_type, payload, None))
            }
            _ => Err((MessageType::Invalid, DecodeError::Unknown)),
        }
    }
}

#[cfg(test)]
mod smbus_tests {
    use super::*;
    use crate::control_packet::MCTPVersionQuery;

    #[test]
    fn test_decode_request() {
        const DEST_ID: u8 = 0x23;
        const SOURCE_ID: u8 = 0x23;

        let ctx = MCTPSMBusContext::new(SOURCE_ID);
        let mut buf: [u8; 12] = [0; 12];

        let _len = ctx.get_request().get_mctp_version_support(
            DEST_ID,
            MCTPVersionQuery::MCTPBaseSpec,
            &mut buf,
        );

        let (msg_type, payload) = ctx.decode_packet(&buf).unwrap();

        assert_eq!(msg_type, MessageType::MCtpControl);
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0], MCTPVersionQuery::MCTPBaseSpec as u8);
    }

    #[test]
    fn test_decode_response() {
        const DEST_ID: u8 = 0x23;
        const SOURCE_ID: u8 = 0x23;

        let ctx = MCTPSMBusContext::new(SOURCE_ID);
        let mut buf: [u8; 17] = [0; 17];

        let _len = ctx
            .get_response()
            .get_mctp_version_support(DEST_ID, &mut buf);

        let (msg_type, payload) = ctx.decode_packet(&buf).unwrap();

        assert_eq!(msg_type, MessageType::MCtpControl);
        assert_eq!(payload.len(), 5);
        assert_eq!(payload[0], 1 as u8);
        // Major Version 1
        assert_eq!(payload[1], 0xF1 as u8);
        // Minor Version 3
        assert_eq!(payload[2], 0xF3 as u8);
        // Update 1
        assert_eq!(payload[3], 0xF1 as u8);
    }

    #[test]
    fn test_decode_invalid_response() {
        const DEST_ID: u8 = 0x23;
        const SOURCE_ID: u8 = 0x23;

        let ctx = MCTPSMBusContext::new(SOURCE_ID);
        let mut buf: [u8; 17] = [0; 17];

        let _len = ctx
            .get_response()
            .get_mctp_version_support(DEST_ID, &mut buf);

        // Set the packet as invalid
        buf[11] = CompletionCode::ErrorInvalidData as u8;

        let error = ctx.decode_packet(&buf);

        match error {
            Ok(_) => {
                panic!("Didn't get the error we expect");
            }
            Err((msg_type, decode_error)) => {
                assert_eq!(msg_type, MessageType::MCtpControl);
                assert_eq!(
                    decode_error,
                    DecodeError::ControlMessage(ControlMessageError::UnsuccessfulCompletionCode(
                        CompletionCode::ErrorInvalidData
                    ))
                );
            }
        }
    }
}
