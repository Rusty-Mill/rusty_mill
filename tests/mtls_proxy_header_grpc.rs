//! Covers the gRPC binding's side of `AgentServer::with_mtls_header`: the
//! configured header name is looked up as lowercased gRPC metadata, the
//! same convention every other security scheme's credential extraction
//! already follows on this binding (spec's gRPC metadata keys are
//! ASCII-lowercase-only).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rusty_a2a::error::{A2aError, Result};
use rusty_a2a::server::grpc::pb;
use rusty_a2a::server::{
    AgentExecutor, AgentServer, AuthContext, AuthVerifier, Credentials, EventSink, RequestContext,
};
use rusty_a2a::types::{
    AgentCard, AgentInterface, MutualTlsSecurityScheme, SecurityRequirement, SecurityScheme, StringList,
    TaskState,
};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};

struct EchoAgent;

#[async_trait]
impl AgentExecutor for EchoAgent {
    async fn execute(&self, _ctx: RequestContext, events: EventSink) -> Result<()> {
        events.status(TaskState::Working);
        events.status_with_message(TaskState::Completed, None);
        Ok(())
    }
}

struct MtlsProxyVerifier;

#[async_trait]
impl AuthVerifier for MtlsProxyVerifier {
    async fn verify(
        &self,
        _requirement: &SecurityRequirement,
        credentials: &Credentials,
    ) -> Result<AuthContext> {
        match credentials.0.get("mtls") {
            Some(v) if v == "SUCCESS" => Ok(AuthContext::new("proxy-verified-client")),
            _ => Err(A2aError::Unauthenticated(
                "client certificate not verified by the proxy".to_string(),
            )),
        }
    }
}

async fn spawn_test_server() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);

    let mut card = AgentCard::new(
        "gRPC mTLS Proxy Header Test Agent",
        "An A2A agent used for rusty_a2a's gRPC mtls-via-proxy-header tests.",
        "0.0.0",
        AgentInterface::json_rpc(format!("http://127.0.0.1:{port}")),
    );
    card.security_schemes.insert(
        "mtls".to_string(),
        SecurityScheme::MutualTls {
            mtls_security_scheme: MutualTlsSecurityScheme { description: None },
        },
    );
    card.security_requirements = vec![SecurityRequirement {
        schemes: HashMap::from([("mtls".to_string(), StringList { list: Vec::new() })]),
    }];

    let services = AgentServer::new(card, Arc::new(EchoAgent))
        .with_auth_verifier(Arc::new(MtlsProxyVerifier))
        .with_mtls_header("ssl-client-verify")
        .build();
    tokio::spawn(async move {
        services.serve_grpc(([127, 0, 0, 1], port)).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    format!("http://127.0.0.1:{port}")
}

type VersionedClient = pb::a2a_service_client::A2aServiceClient<
    InterceptedService<Channel, fn(Request<()>) -> std::result::Result<Request<()>, Status>>,
>;

/// Every request through this client carries `a2a-version: 1.0` gRPC
/// metadata - matching what `client::GrpcClient` always sends, and what
/// this crate's gRPC binding now enforces (spec Section 3.2.6/3.6.2). A
/// bare generated `tonic` client (like every test in this file drives)
/// sends no metadata at all unless told to.
fn add_version_metadata(mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
    req.metadata_mut().insert("a2a-version", "1.0".parse().unwrap());
    Ok(req)
}

async fn connect(url: String) -> VersionedClient {
    let channel = Channel::from_shared(url)
        .expect("valid uri")
        .connect()
        .await
        .expect("connect");
    pb::a2a_service_client::A2aServiceClient::with_interceptor(channel, add_version_metadata as fn(_) -> _)
}

fn user_message(text: &str) -> pb::Message {
    pb::Message {
        message_id: "m1".to_string(),
        role: pb::Role::User as i32,
        parts: vec![pb::Part {
            content: Some(pb::part::Content::Text(text.to_string())),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn grpc_reads_the_configured_header_as_lowercased_metadata() {
    let url = spawn_test_server().await;
    let mut client = connect(url).await;

    let mut verified = Request::new(pb::SendMessageRequest {
        message: Some(user_message("hi")),
        ..Default::default()
    });
    // gRPC metadata keys are lowercase-only regardless of what case the
    // configured header name uses - `with_mtls_header("ssl-client-verify")`
    // is already lowercase here, matching real proxy convention.
    verified
        .metadata_mut()
        .insert("ssl-client-verify", "SUCCESS".parse().unwrap());
    let accepted = client.send_message(verified).await;
    assert!(accepted.is_ok(), "expected success, got {accepted:?}");

    let unverified = Request::new(pb::SendMessageRequest {
        message: Some(user_message("hi")),
        ..Default::default()
    });
    let rejected = client.send_message(unverified).await.unwrap_err();
    assert_eq!(rejected.code(), tonic::Code::Unauthenticated);
}
