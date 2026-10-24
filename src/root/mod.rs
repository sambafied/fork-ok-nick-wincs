mod connect;
mod fault;
mod session;
mod sync_root_id;
mod sync_root_info;

pub use connect::{Connection, RetainedShutdown, ShutdownStatus, WatcherState};
#[cfg(feature = "fault-injection")]
pub use fault::FaultPlan;
pub use session::{ConnectFailure, Session};
pub use sync_root_id::{active_roots, is_supported, SecurityId, SyncRootId, SyncRootIdBuilder};
pub use sync_root_info::{
    HydrationPolicy, HydrationType, PopulationType, ProtectionMode, SupportedAttribute,
    SyncRootInfo,
};
