use super::super::*;

#[test]
fn websocket_message_conversion_preserves_payload_frames() {
    assert_eq!(
        axum_to_tungstenite_message(AxumWsMessage::Text("hello".into()))
            .expect("text")
            .into_text()
            .expect("text payload")
            .as_str(),
        "hello"
    );
    assert_eq!(
        tungstenite_to_axum_message(TungsteniteMessage::Text("hello".into())),
        Some(AxumWsMessage::Text("hello".into()))
    );
    assert_eq!(
        axum_to_tungstenite_message(AxumWsMessage::Binary(vec![1, 2, 3].into())),
        Some(TungsteniteMessage::Binary(vec![1, 2, 3].into()))
    );
    assert_eq!(
        tungstenite_to_axum_message(TungsteniteMessage::Binary(vec![1, 2, 3].into())),
        Some(AxumWsMessage::Binary(vec![1, 2, 3].into()))
    );
    assert_eq!(
        axum_to_tungstenite_message(AxumWsMessage::Ping(vec![4, 5].into())),
        Some(TungsteniteMessage::Ping(vec![4, 5].into()))
    );
    assert_eq!(
        tungstenite_to_axum_message(TungsteniteMessage::Ping(vec![4, 5].into())),
        Some(AxumWsMessage::Ping(vec![4, 5].into()))
    );
    assert_eq!(
        axum_to_tungstenite_message(AxumWsMessage::Pong(vec![6, 7].into())),
        Some(TungsteniteMessage::Pong(vec![6, 7].into()))
    );
    assert_eq!(
        tungstenite_to_axum_message(TungsteniteMessage::Pong(vec![6, 7].into())),
        Some(AxumWsMessage::Pong(vec![6, 7].into()))
    );
}
