use skillopt_core::{ChatBackend, Message};
use skillopt_model::MockBackend;

#[tokio::test]
async fn echo_mock_returns_last_message_content() {
    let backend = MockBackend::echo("test");
    let out = backend
        .chat(&[Message::system("sys"), Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(out, "hello");
}

#[tokio::test]
async fn scripted_mock_cycles_then_holds_last_response() {
    let backend = MockBackend::new("test", vec!["a".into(), "b".into()]);
    assert_eq!(backend.chat(&[Message::user("x")]).await.unwrap(), "a");
    assert_eq!(backend.chat(&[Message::user("x")]).await.unwrap(), "b");
    assert_eq!(backend.chat(&[Message::user("x")]).await.unwrap(), "b");
}
