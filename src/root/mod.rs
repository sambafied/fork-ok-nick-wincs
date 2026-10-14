mod connect;
mod fault;
mod session;
mod sync_root_id;
mod sync_root_info;

pub use connect::{Connection, ShutdownStatus};
#[cfg(feature = "fault-injection")]
pub use fault::FaultPlan;
pub use session::Session;
pub use sync_root_id::{active_roots, is_supported, SecurityId, SyncRootId, SyncRootIdBuilder};
pub use sync_root_info::{
    HydrationPolicy, HydrationType, PopulationType, ProtectionMode, SupportedAttribute,
    SyncRootInfo,
};
