//! The three manpower data products -- the Rust port of
//! `meshed.domains.products` (DOM-012..025): `PersonnelLifecycleProducer`,
//! `PositionManagementProducer`, and `ReadinessAssessmentProducer` plus
//! its two derivation consumers, composed by `ReadinessReportingProduct`.
//!
//! None of these are subclasses -- Rust has no class-attribute
//! inheritance for `DataProductProducerBase`/`DataProductConsumerBase`
//! to override (see those types' own module docs in
//! `rusty-meshed-sdk`). Each product here is a thin, mostly zero-sized
//! type carrying its metadata as associated consts plus an
//! `output_ports()`/`connect()` pair, wrapping a
//! `DataProductProducerBase`/`DataProductConsumerBase` the way the
//! source's subclasses wrap the base class's inherited behavior.
//!
//! [`readiness_reporting::ReadinessReportingProduct`] is the only
//! genuinely consumer-shaped piece here (DOM-023..025), and it's fully
//! built: `startup()` (source-parity name, joins both consumers'
//! groups and resolves starting offsets sequentially) and `run()`
//! (both consumers' poll loops driven concurrently via
//! `rusty_tokio::try_join!`, matching the source's own `asyncio.gather`
//! -- see that module's own doc for why) both build on
//! `rusty-meshed-sdk::consumer::DataProductConsumerBase`'s `startup`/
//! `run`.

mod personnel_lifecycle;
mod position_management;
mod readiness_reporting;

pub use personnel_lifecycle::{
    PersonnelLifecycleProducer, PersonnelLifecyclePublishError, PersonnelLifecycleStartupError,
};
pub use position_management::PositionManagementProducer;
pub use readiness_reporting::{
    PersonnelAssignmentConsumer, PositionFillConsumer, ReadinessAssessmentProducer,
    ReadinessReportingError, ReadinessReportingProduct,
};
