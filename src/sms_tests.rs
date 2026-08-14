use super::*;
use std::sync::Mutex as StdMutex;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct RecordingGateway {
    sent: StdMutex<Vec<(String, String)>>,
    marked: StdMutex<Vec<String>>,
    incoming: StdMutex<Vec<IncomingSms>>,
    send_attempts: StdMutex<usize>,
    failures_remaining: StdMutex<usize>,
}

#[async_trait]
impl SmsGateway for RecordingGateway {
    async fn unread_messages(&self) -> anyhow::Result<Vec<IncomingSms>> {
        Ok(self.incoming.lock().unwrap().clone())
    }

    async fn mark_read(&self, message_id: &str) -> anyhow::Result<()> {
        self.marked.lock().unwrap().push(message_id.to_string());
        Ok(())
    }

    async fn send_segment(&self, phone_number: &str, text: &str) -> anyhow::Result<()> {
        *self.send_attempts.lock().unwrap() += 1;
        let mut failures_remaining = self.failures_remaining.lock().unwrap();
        if *failures_remaining > 0 {
            *failures_remaining -= 1;
            anyhow::bail!("send failed");
        }
        drop(failures_remaining);
        self.sent
            .lock()
            .unwrap()
            .push((phone_number.to_string(), text.to_string()));
        Ok(())
    }
}

#[test]
fn sms_deduplicator_remembers_only_explicitly_processed_messages() {
    let message = IncomingSms {
        id: "42".into(),
        phone_number: "+49123".into(),
        text: "Hello".into(),
        modem_date: "2026-08-13 19:48:48".into(),
    };
    let mut deduplicator = SmsDeduplicator::default();

    assert!(!deduplicator.is_duplicate(&message));
    assert!(!deduplicator.is_duplicate(&message));
    deduplicator.remember(&message);
    assert!(deduplicator.is_duplicate(&message));

    let mut same_sms_different_storage_id = message.clone();
    same_sms_different_storage_id.id = "43".into();
    same_sms_different_storage_id.phone_number = "+49 123".into();
    assert!(deduplicator.is_duplicate(&same_sms_different_storage_id));

    let mut different_sms = message.clone();
    different_sms.text = "Another message".into();
    assert!(!deduplicator.is_duplicate(&different_sms));
}

#[test]
fn phone_numbers_are_normalized_for_mapping() {
    assert_eq!(normalize_phone_number("+49 170-123 45 67"), "491701234567");
    assert_eq!(normalize_phone_number("0049 (170) 1234567"), "491701234567");
    assert_eq!(normalize_phone_number("0170/1234567"), "01701234567");
    assert!(normalize_phone_number("no number").is_empty());
}

#[test]
fn telegram_sms_mirrors_have_a_reserved_display_only_envelope() {
    let incoming = telegram_incoming_mirror_text(&IncomingSms {
        id: "42".into(),
        phone_number: "+49123".into(),
        text: "Hello".into(),
        modem_date: "2026-08-14 09:00:00".into(),
    });
    assert_eq!(
        incoming,
        "[GlowBot SMS mirror]\nFrom +49123 (2026-08-14 09:00:00)\nHello"
    );
    assert!(is_telegram_sms_mirror_text(&incoming));

    let outgoing = telegram_outgoing_mirror_text("+49123", "Hi back");
    assert_eq!(outgoing, "[GlowBot SMS mirror]\nTo +49123\nHi back");
    assert!(is_telegram_sms_mirror_text(&outgoing));
    assert!(!is_telegram_sms_mirror_text(
        "A normal message mentioning [GlowBot SMS mirror]"
    ));
}

#[test]
fn avoidable_unicode_is_converted_without_damaging_gsm_characters() {
    let prepared = prepare_sms_text("“Grüße”—café… • test 😀 `x`");
    assert_eq!(prepared, "\"Grüße\"-café... - test  'x'");
    assert!(prepared
        .chars()
        .all(|character| gsm_units(character).is_some()));
}

#[test]
fn necessary_non_latin_text_is_preserved() {
    assert_eq!(prepare_sms_text("你好"), "你好");
}

#[test]
fn gsm_messages_split_at_160_septets() {
    assert_eq!(split_sms(&"a".repeat(160)), vec!["a".repeat(160)]);
    let chunks = split_sms(&"a".repeat(161));
    assert_eq!(chunks, vec!["a".repeat(160), "a".to_string()]);
}

#[test]
fn gsm_extension_characters_count_as_two_septets() {
    assert_eq!(split_sms(&"^".repeat(80)), vec!["^".repeat(80)]);
    let chunks = split_sms(&"^".repeat(81));
    assert_eq!(chunks, vec!["^".repeat(80), "^".to_string()]);
}

#[test]
fn unicode_messages_use_the_70_utf16_unit_limit() {
    let text = "界".repeat(71);
    let chunks = split_sms(&text);
    assert_eq!(chunks, vec!["界".repeat(70), "界".to_string()]);
    assert_eq!(chunks.concat(), text);
}

#[test]
fn splitting_prefers_whitespace_without_losing_text() {
    let text = "a short sentence ".repeat(20);
    let chunks = split_sms(&text);
    assert!(chunks.len() > 1);
    assert_eq!(chunks.concat(), text);
    assert!(chunks.iter().all(|chunk| {
        chunk
            .chars()
            .map(|character| gsm_units(character).unwrap())
            .sum::<usize>()
            <= GSM_SEGMENT_SEPTETS
    }));
    assert!(split_sms("").is_empty());
}

#[tokio::test]
async fn sms_reply_sender_sanitizes_splits_and_sends_every_segment() {
    let gateway = Arc::new(RecordingGateway::default());
    let sender = SmsReplySender::new(gateway.clone(), "+49123", None);
    sender
        .send_text(&format!("{}😀", "a".repeat(161)))
        .await
        .unwrap();

    let sent = gateway.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], ("+49123".into(), "a".repeat(160)));
    assert_eq!(sent[1], ("+49123".into(), "a".into()));
    assert_eq!(*gateway.send_attempts.lock().unwrap(), 2);
}

#[tokio::test]
async fn sms_reply_sender_retries_transient_multipart_send_failures() {
    let gateway = Arc::new(RecordingGateway {
        failures_remaining: StdMutex::new(1),
        ..Default::default()
    });
    let sender = SmsReplySender::new(gateway.clone(), "+49123", None);
    sender.send_text(&"a".repeat(161)).await.unwrap();

    assert_eq!(*gateway.send_attempts.lock().unwrap(), 3);
    let sent = gateway.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].1, "a".repeat(160));
    assert_eq!(sent[1].1, "a");
}

#[tokio::test]
async fn sms_reply_sender_reports_empty_and_gateway_errors() {
    let gateway = Arc::new(RecordingGateway {
        failures_remaining: StdMutex::new(SMS_SEND_ATTEMPTS),
        ..Default::default()
    });
    let sender = SmsReplySender::new(gateway.clone(), "+49123", None);
    assert!(sender.send_text("hello").await.is_err());
    assert!(sender.send_text("😀").await.is_err());
    assert_eq!(*gateway.send_attempts.lock().unwrap(), SMS_SEND_ATTEMPTS);
}

fn login_state(logged_in: bool) -> String {
    format!(
        "<response><password_type>4</password_type><extern_password_type>1</extern_password_type>\
         <history_login_flag>0</history_login_flag><State>{}</State>\
         <guidemodifypwdpageflag>0</guidemodifypwdpageflag><rsapadingtype>1</rsapadingtype>\
         <accounts_number>1</accounts_number><wifipwdsamewithwebpwd>0</wifipwdsamewithwebpwd>\
         <remainwaittime>0</remainwaittime><lockstatus>0</lockstatus><forceskipguide>0</forceskipguide>\
         <username>{}</username><firstlogin>0</firstlogin><userlevel>2</userlevel></response>",
        if logged_in { "0" } else { "-1" },
        if logged_in { "admin" } else { "" }
    )
}

async fn mount_logged_in_modem(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/user/state-login"))
        .respond_with(ResponseTemplate::new(200).set_body_string(login_state(true)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/webserver/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<response><token>test-token</token></response>"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn huawei_gateway_lists_marks_and_sends_text_sms() {
    let server = MockServer::start().await;
    mount_logged_in_modem(&server).await;
    let inbox = "<response><Count>3</Count><Messages>\
        <Message><Smstat>0</Smstat><Index>42</Index><Phone>+49123</Phone>\
        <Content>Hello</Content><Date>2026-08-11 10:00:00</Date><Sca></Sca>\
        <SaveType>0</SaveType><Priority>0</Priority><SmsType>1</SmsType></Message>\
        <Message><Smstat>0</Smstat><Index>43</Index><Phone>+49123</Phone>\
        <Content>A long multipart message</Content><Date>2026-08-11 10:01:00</Date>\
        <Sca></Sca><SaveType>0</SaveType><Priority>0</Priority><SmsType>2</SmsType></Message>\
        <Message><Smstat>0</Smstat><Index>44</Index><Phone>+49123</Phone>\
        <Content></Content><Date>2026-08-11 10:02:00</Date><Sca></Sca>\
        <SaveType>0</SaveType><Priority>0</Priority><SmsType>7</SmsType></Message>\
        </Messages></response>";
    Mock::given(method("POST"))
        .and(path("/api/sms/sms-list"))
        .and(body_string_contains("<BoxType>1</BoxType>"))
        .respond_with(ResponseTemplate::new(200).set_body_string(inbox))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/sms/sms-list"))
        .and(body_string_contains("<BoxType>4</BoxType>"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/sms/set-read"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<response>OK</response>"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/sms/send-sms"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<response>OK</response>"))
        .mount(&server)
        .await;

    let gateway = HuaweiSmsGateway::connect_to_url(&server.uri(), "secret")
        .await
        .unwrap();
    let messages = gateway.unread_messages().await.unwrap();
    assert_eq!(
        messages,
        vec![
            IncomingSms {
                id: "42".into(),
                phone_number: "+49123".into(),
                text: "Hello".into(),
                modem_date: "2026-08-11 10:00:00".into(),
            },
            IncomingSms {
                id: "43".into(),
                phone_number: "+49123".into(),
                text: "A long multipart message".into(),
                modem_date: "2026-08-11 10:01:00".into(),
            },
        ]
    );
    gateway.mark_read("42").await.unwrap();
    gateway
        .send_segment("+49123", "Reply & more")
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let send = requests
        .iter()
        .find(|request| request.url.path() == "/api/sms/send-sms")
        .unwrap();
    let body = String::from_utf8(send.body.clone()).unwrap();
    assert!(body.contains("<Phone>+49123</Phone>"));
    assert!(body.contains("<Content>Reply &amp; more</Content>"));
    assert!(body.contains("<Length>12</Length>"));

    let marked_status = requests.iter().any(|request| {
        request.url.path() == "/api/sms/set-read"
            && String::from_utf8_lossy(&request.body).contains("<Index>44</Index>")
    });
    assert!(marked_status, "delivery confirmation should be marked read");
}

#[tokio::test]
async fn huawei_gateway_logs_in_when_needed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/user/state-login"))
        .respond_with(ResponseTemplate::new(200).set_body_string(login_state(false)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/webserver/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<response><token>test-token</token></response>"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/user/login"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<response>OK</response>"))
        .expect(1)
        .mount(&server)
        .await;

    HuaweiSmsGateway::connect_to_url(&server.uri(), "secret")
        .await
        .unwrap();
}

#[tokio::test]
async fn huawei_gateway_rejects_unexpected_send_response() {
    let server = MockServer::start().await;
    mount_logged_in_modem(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/sms/send-sms"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<response>QUEUED</response>"))
        .mount(&server)
        .await;
    let gateway = HuaweiSmsGateway::connect_to_url(&server.uri(), "secret")
        .await
        .unwrap();
    assert!(gateway.send_segment("+49123", "hello").await.is_err());
}
