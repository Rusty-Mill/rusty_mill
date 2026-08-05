//! Wire types for the Agent Communication Protocol, version 0.2.0.
//!
//! Every type here maps one-to-one onto a schema in the ACP OpenAPI document
//! and round-trips through `serde` in the protocol's JSON representation.

pub mod agent;
pub mod error;
pub mod event;
pub mod message;
pub mod run;
pub mod session;

pub use agent::{
    content_type_matches, AgentDependency, AgentManifest, AgentName, AgentsListResponse,
    Capability, DependencyType, Link, LinkType, Metadata, Person, Status, Tag,
};
pub use error::{Error, ErrorCode};
pub use event::Event;
pub use message::{
    CitationMetadata, ContentEncoding, Message, MessagePart, PartMetadata, Role,
    TrajectoryMetadata, DEFAULT_CONTENT_TYPE,
};
pub use run::{
    AwaitRequest, AwaitResume, Run, RunCreateRequest, RunEventsListResponse, RunId, RunMode,
    RunResumeRequest, RunStatus,
};
pub use session::{Session, SessionId};
