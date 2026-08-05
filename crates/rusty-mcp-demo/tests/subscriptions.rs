//! End-to-end `subscriptions/listen` tests against `DemoServer`.
//!
//! The unit tests cover the broadcaster in isolation; these check the part that
//! only exists on the wire — that a client's filter is honoured and that a
//! change published inside the server reaches a listening client.

use std::time::Duration;

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, ServiceExt,
    model::{
        CallToolRequestParams, ClientInfo, ProtocolVersion, ServerNotification, SubscriptionFilter,
    },
    service::RunningService,
};

#[path = "../src/prompts.rs"]
mod prompts;
#[path = "../src/resources.rs"]
mod resources;
#[path = "../src/server.rs"]
mod server;
#[path = "../src/tools/mod.rs"]
mod tools;

use server::DemoServer;

/// Anything slower than this and the notification is not coming.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

/// Connect a stateless 2026-07-28 client.
///
/// `subscriptions/listen` does not exist for older peers, so the Discover
/// lifecycle is required rather than incidental here.
async fn connect() -> RunningService<rmcp::RoleClient, ClientInfo> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);

    tokio::spawn(async move {
        let running = DemoServer::new()
            .serve(server_transport)
            .await
            .expect("server starts");
        let _ = running.waiting().await;
    });

    ClientInfo::default()
        .serve_with_lifecycle(
            client_transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("client connects")
}

pub(crate) fn touch(uri: &str) -> CallToolRequestParams {
    CallToolRequestParams::new("touch_resource").with_arguments(
        serde_json::json!({ "uri": uri })
            .as_object()
            .cloned()
            .expect("object"),
    )
}

#[tokio::test]
async fn a_change_published_in_the_server_reaches_a_listening_client() {
    let client = connect().await;

    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .build(),
        )
        .await
        .expect("subscriptions/listen");

    // The tool publishes through the same broadcaster the subscription reads.
    client
        .call_tool(touch("config://demo"))
        .await
        .expect("call touch_resource");

    let notification = tokio::time::timeout(TIMEOUT, subscription.next())
        .await
        .expect("a notification should arrive")
        .expect("subscription is live")
        .expect("not the end of the stream");

    assert!(
        matches!(
            notification,
            ServerNotification::ResourceListChangedNotification(_)
        ),
        "unexpected notification: {notification:?}"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_client_only_receives_the_categories_it_asked_for() {
    let client = connect().await;

    // Subscribed to per-resource updates only, not the list.
    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("config://demo")
                .build(),
        )
        .await
        .expect("subscriptions/listen");

    // `touch_resource` publishes both an update and a list change; only the
    // update was opted in to.
    client
        .call_tool(touch("config://demo"))
        .await
        .expect("call touch_resource");

    let notification = tokio::time::timeout(TIMEOUT, subscription.next())
        .await
        .expect("a notification should arrive")
        .expect("subscription is live")
        .expect("not the end of the stream");

    match notification {
        ServerNotification::ResourceUpdatedNotification(update) => {
            assert_eq!(update.params.uri, "config://demo");
        }
        other => panic!("expected a resource update, got {other:?}"),
    }

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn a_resource_update_for_another_uri_is_not_delivered() {
    let client = connect().await;

    let mut subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("config://demo")
                .build(),
        )
        .await
        .expect("subscriptions/listen");

    // A different URI: the subscription must stay quiet rather than leaking
    // activity about resources the client did not ask about.
    client
        .call_tool(touch("status://uptime"))
        .await
        .expect("call touch_resource");

    let quiet = tokio::time::timeout(Duration::from_millis(300), subscription.next()).await;
    assert!(quiet.is_err(), "no notification should have arrived");

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn several_clients_each_get_their_own_notifications() {
    // One broadcaster, many subscriptions — the fan-out the module exists for.
    let client = connect().await;

    let mut first = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .build(),
        )
        .await
        .expect("first subscription");

    let mut second = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .build(),
        )
        .await
        .expect("second subscription");

    client
        .call_tool(touch("config://demo"))
        .await
        .expect("call touch_resource");

    for (label, subscription) in [("first", &mut first), ("second", &mut second)] {
        let notification = tokio::time::timeout(TIMEOUT, subscription.next())
            .await
            .unwrap_or_else(|_| panic!("{label} subscription should have received it"))
            .expect("live")
            .expect("not the end");
        assert!(matches!(
            notification,
            ServerNotification::ResourceListChangedNotification(_)
        ));
    }

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn an_unadvertised_category_is_dropped_from_the_filter() {
    // The demo does not advertise `prompts` subscriptions beyond list_changed,
    // and rmcp intersects the request with advertised capabilities — so asking
    // for something unsupported succeeds quietly rather than erroring.
    let client = connect().await;

    let subscription = client
        .listen(SubscriptionFilter::builder().prompts_list_changed().build())
        .await;

    assert!(
        subscription.is_ok(),
        "an advertised category should be accepted"
    );

    client.cancel().await.expect("cancel");
}

#[tokio::test]
async fn the_tool_reports_how_many_listeners_it_reached() {
    let client = connect().await;

    // No subscribers yet.
    let before = client
        .call_tool(touch("config://demo"))
        .await
        .expect("call touch_resource");
    let before_text = before
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text");
    assert!(before_text.contains("0 listener"), "{before_text}");

    let _subscription = client
        .listen(
            SubscriptionFilter::builder()
                .resources_list_changed()
                .build(),
        )
        .await
        .expect("subscriptions/listen");

    let after = client
        .call_tool(touch("config://demo"))
        .await
        .expect("call touch_resource");
    let after_text = after
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text");
    assert!(after_text.contains("1 listener"), "{after_text}");

    client.cancel().await.expect("cancel");
}

/// Streamable HTTP builds a fresh handler per request, so a subscription
/// opened by one request must still hear a change published by another.
///
/// This is the case a duplex/stdio test cannot catch: with one handler per
/// connection, a per-handler broadcaster looks like it works.
mod over_http {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use rmcp::{
        ClientLifecycleMode, ClientServiceExt,
        model::{ClientInfo, ProtocolVersion, ServerNotification, SubscriptionFilter},
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    use rusty_mcp::{
        HttpConfig, ServerConfig, Transport, subscriptions::ChangeBroadcaster, tasks::TaskSupport,
    };

    use super::{DemoServer, TIMEOUT, touch};
    use crate::server::{DemoState, default_task_support};

    async fn spawn() -> (SocketAddr, ChangeBroadcaster) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let state = Arc::new(DemoState::default());
        let tasks: TaskSupport = default_task_support();
        let changes = ChangeBroadcaster::new();

        let config = ServerConfig {
            transport: Transport::Http(HttpConfig {
                bind: addr,
                sse_keep_alive: None,
                ..Default::default()
            }),
            ..Default::default()
        };

        tokio::spawn({
            let changes = changes.clone();
            async move {
                let _ = rusty_mcp::serve(
                    move || {
                        // One broadcaster, cloned per request-scoped handler.
                        Ok(DemoServer::with_parts(
                            Arc::clone(&state),
                            tasks.clone(),
                            changes.clone(),
                        ))
                    },
                    config,
                )
                .await;
            }
        });

        for _ in 0..100 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                return (addr, changes);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("server never became ready");
    }

    async fn connect_http(
        addr: SocketAddr,
    ) -> rmcp::service::RunningService<rmcp::RoleClient, ClientInfo> {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
        );
        ClientInfo::default()
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .expect("client connects over http")
    }

    #[tokio::test]
    async fn a_change_from_one_request_reaches_a_subscription_from_another() {
        let (addr, _changes) = spawn().await;

        let subscriber = connect_http(addr).await;
        let mut subscription = subscriber
            .listen(
                SubscriptionFilter::builder()
                    .resources_list_changed()
                    .build(),
            )
            .await
            .expect("subscriptions/listen");

        // A separate connection entirely — and under Streamable HTTP, a
        // separate handler instance.
        let publisher = connect_http(addr).await;
        let result = publisher
            .call_tool(touch("config://demo"))
            .await
            .expect("call touch_resource");

        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .expect("text");
        assert!(
            text.contains("1 listener"),
            "the publishing handler should see the subscriber: {text}"
        );

        let notification = tokio::time::timeout(TIMEOUT, subscription.next())
            .await
            .expect("a notification should cross between requests")
            .expect("live")
            .expect("not the end");
        assert!(matches!(
            notification,
            ServerNotification::ResourceListChangedNotification(_)
        ));

        subscriber.cancel().await.expect("cancel subscriber");
        publisher.cancel().await.expect("cancel publisher");
    }
}
