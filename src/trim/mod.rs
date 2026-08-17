//! Turning a full `data.raw` dump into the small slice a consumer asked for.
//!
//! Every stage here is a pure function over `serde_json::Value`, so the whole
//! module is testable with no Factorio present. The allowlists arrive from the
//! caller: see [`spec::TrimSpec`].

pub mod canonical;
pub mod prototypes;
pub mod spec;
