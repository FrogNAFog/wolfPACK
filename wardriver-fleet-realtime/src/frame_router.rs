//! ESP-NOW ingress frame classification helpers (host-testable).

use wardriver_proto::FrameType;

/// Best-effort `sniffer_id` peek for roster gating (non-hello frames only).
#[must_use]
pub fn peek_ingress_sniffer_id(data: &[u8]) -> Option<u32> {
    if data.len() < 14 {
        return None;
    }
    match data[1] {
        x if x == FrameType::SnifferAck as u8 => {
            Some(u32::from_le_bytes(data.get(10..14)?.try_into().ok()?))
        }
        x if x == FrameType::DetectionAlert as u8 || x == FrameType::DetectionBleAlert as u8 => {
            Some(u32::from_le_bytes(data.get(10..14)?.try_into().ok()?))
        }
        x if x == FrameType::ScanFrag as u8
            || x == FrameType::BleScanFrag as u8
            || x == FrameType::ProbeReqFrag as u8 =>
        {
            if data.len() < 15 {
                return None;
            }
            Some(u32::from_le_bytes(data.get(11..15)?.try_into().ok()?))
        }
        x if x == FrameType::ReconWifiFrag as u8 => {
            if data.len() < 18 {
                return None;
            }
            Some(u32::from_le_bytes(data.get(14..18)?.try_into().ok()?))
        }
        x if x == FrameType::TimeSyncPong as u8 => {
            if data.len() < 22 {
                return None;
            }
            Some(u32::from_le_bytes(data.get(18..22)?.try_into().ok()?))
        }
        x if x == FrameType::SsidWatchAlert as u8 || x == FrameType::RemoteIdAlert as u8 => {
            Some(u32::from_le_bytes(data.get(10..14)?.try_into().ok()?))
        }
        _ => None,
    }
}

/// True when frame type bypasses roster-id peek (hello carries id in payload body).
#[must_use]
pub fn roster_bypass_frame_type(ft: u8) -> bool {
    matches!(
        ft,
        x if x == FrameType::SnifferHello as u8 || x == FrameType::PeerJoin as u8
    )
}

/// Detection-priority frames processed before bulk scan fragments in a batch.
#[must_use]
pub fn detection_priority_frame_type(ft: u8) -> bool {
    matches!(
        ft,
        x if x == FrameType::DetectionAlert as u8
            || x == FrameType::DetectionBleAlert as u8
            || x == FrameType::SsidWatchAlert as u8
            || x == FrameType::RemoteIdAlert as u8
    )
}

/// Order ingress batch: detection alerts first, then remaining frames in arrival order.
pub fn sort_ingress_batch<T>(frames: &mut [T], frame_type: impl Fn(&T) -> u8) {
    frames.sort_by_key(|f| u8::from(!detection_priority_frame_type(frame_type(f))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_ack_sniffer_id() {
        let mut data = vec![0u8; 20];
        data[1] = FrameType::SnifferAck as u8;
        data[10..14].copy_from_slice(&42u32.to_le_bytes());
        assert_eq!(peek_ingress_sniffer_id(&data), Some(42));
    }

    #[test]
    fn detection_priority_sort() {
        let mut types = [
            FrameType::ScanFrag as u8,
            FrameType::DetectionAlert as u8,
            FrameType::BleScanFrag as u8,
        ];
        sort_ingress_batch(&mut types, |t| *t);
        assert_eq!(types[0], FrameType::DetectionAlert as u8);
    }
}
