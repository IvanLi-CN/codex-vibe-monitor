mod internal {
    use crate::*;

    include!("proxy/request_entry.rs");
    include!("proxy/failover.rs");
    include!("proxy/route_selection.rs");
    include!("proxy/dispatch.rs");
    include!("proxy/stream_gate.rs");
    include!("proxy/usage_persistence.rs");
    include!("proxy/raw_capture.rs");
    include!("proxy/payload_utils.rs");
}

pub(crate) use internal::*;
