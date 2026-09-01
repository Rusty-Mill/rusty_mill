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
//! [`readiness_reporting::ReadinessReportingProduct::prepare`] and the
//! two consumers' [`process`](personnel_lifecycle) methods are the only
//! genuinely consumer-shaped pieces here, and both are scoped by the
//! same `rusty_kafka` `Fetch`/consumer-group gap
//! `rusty-meshed-sdk::consumer`'s own module doc explains: `process()`
//! (the event-derivation business logic, DOM-023/024) is fully built
//! and tested, since it needs only `DataProductProducerBase::publish`;
//! there is no poll loop to drive it from yet, and
//! `ReadinessReportingProduct::run`'s `asyncio.gather`-driven
//! concurrent polling (DOM-025) isn't built for the same reason.

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
