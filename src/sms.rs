use crate::bot_send::TextReplySender;
use crate::config::SmsConfig;
use anyhow::Context;
use async_trait::async_trait;
use huawei_dongle_api::models::{SmsBoxType, SmsListRequest, SmsSendRequest, SmsSortType, SmsType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use teloxide::types::ChatId;

const SMS_DEDUP_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const TELEGRAM_SMS_MIRROR_TAG: &str = "[GlowBot SMS mirror]";
const GSM_BASIC: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
const GSM_EXTENSION: &str = "^{}\\[~]|€";
const GSM_SEGMENT_SEPTETS: usize = 160;
const UCS2_SEGMENT_UNITS: usize = 70;

/// One unread text message returned by the modem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingSms {
    pub id: String,
    pub phone_number: String,
    pub text: String,
    pub modem_date: String,
}

/// Internal modem entry retaining the SMS type long enough to distinguish
/// user-authored text from delivery confirmations.
struct ModemInboxEntry {
    message: IncomingSms,
    sms_type: SmsType,
}

impl ModemInboxEntry {
    fn is_user_message(&self) -> bool {
        matches!(
            self.sms_type,
            SmsType::Single | SmsType::Multipart | SmsType::Unicode
        )
    }
}

/// Suppresses modem inbox entries that remain marked unread briefly after they
/// have already been processed. Entries are remembered only after successful
/// delivery, so genuine processing failures can still be retried.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SmsFingerprint {
    phone_number: String,
    text: String,
    modem_date: String,
}

impl From<&IncomingSms> for SmsFingerprint {
    fn from(message: &IncomingSms) -> Self {
        Self {
            phone_number: normalize_phone_number(&message.phone_number),
            text: message.text.clone(),
            modem_date: message.modem_date.clone(),
        }
    }
}

#[derive(Default)]
pub struct SmsDeduplicator {
    processed: HashMap<SmsFingerprint, Instant>,
}

impl SmsDeduplicator {
    pub fn is_duplicate(&mut self, message: &IncomingSms) -> bool {
        let now = Instant::now();
        self.processed
            .retain(|_, seen_at| now.duration_since(*seen_at) < SMS_DEDUP_RETENTION);
        self.processed.contains_key(&message.into())
    }

    pub fn remember(&mut self, message: &IncomingSms) {
        self.processed.insert(message.into(), Instant::now());
    }
}

/// Modem abstraction used by the SMS poller and reply sender.
#[async_trait]
pub trait SmsGateway: Send + Sync {
    async fn unread_messages(&self) -> anyhow::Result<Vec<IncomingSms>>;
    async fn mark_read(&self, message_id: &str) -> anyhow::Result<()>;
    async fn send_segment(&self, phone_number: &str, text: &str) -> anyhow::Result<()>;
}

/// Huawei HiLink implementation backed by `huawei-dongle-api`.
pub struct HuaweiSmsGateway {
    client: huawei_dongle_api::Client,
    password: String,
    operation_lock: tokio::sync::Mutex<()>,
}

impl HuaweiSmsGateway {
    pub async fn connect(config: &SmsConfig) -> anyhow::Result<Self> {
        let ip: std::net::IpAddr = config
            .ip_address
            .parse()
            .context("invalid Huawei modem IP address")?;
        let host = match ip {
            std::net::IpAddr::V4(address) => address.to_string(),
            std::net::IpAddr::V6(address) => format!("[{address}]"),
        };
        Self::connect_to_url(&format!("http://{host}"), &config.password).await
    }

    async fn connect_to_url(base_url: &str, password: &str) -> anyhow::Result<Self> {
        let api_config = huawei_dongle_api::Config::builder()
            .base_url(base_url)
            .timeout(std::time::Duration::from_secs(15))
            .max_retries(2)
            .build()
            .context("failed to configure Huawei modem client")?;
        let gateway = Self {
            client: huawei_dongle_api::Client::new(api_config)
                .context("failed to create Huawei modem client")?,
            password: password.to_string(),
            operation_lock: tokio::sync::Mutex::new(()),
        };
        gateway.ensure_authenticated().await?;
        Ok(gateway)
    }

    async fn ensure_authenticated(&self) -> anyhow::Result<()> {
        let state = self
            .client
            .auth()
            .state_login()
            .await
            .context("failed to read Huawei login state")?;
        if !state.is_logged_in() {
            self.client
                .auth()
                .login("admin", &self.password)
                .await
                .context("failed to log in to Huawei modem")?;
        }
        Ok(())
    }

    async fn list_inbox(&self, box_type: SmsBoxType) -> anyhow::Result<Vec<ModemInboxEntry>> {
        // Huawei B311 firmware rejects ReadCount values above 20 with error 100005.
        // Remaining unread messages are picked up on the next poll after this page is marked read.
        let request = SmsListRequest::new(1, 20, box_type, SmsSortType::ByTime, true, true);
        let response = self
            .client
            .sms()
            .list(&request)
            .await
            .context("failed to list Huawei SMS inbox")?;
        Ok(response
            .messages
            .messages
            .into_iter()
            .filter(|message| message.is_unread())
            .map(|message| ModemInboxEntry {
                sms_type: message.sms_type,
                message: IncomingSms {
                    id: message.index,
                    phone_number: message.phone,
                    text: message.content,
                    modem_date: message.date,
                },
            })
            .collect())
    }
}

#[async_trait]
impl SmsGateway for HuaweiSmsGateway {
    async fn unread_messages(&self) -> anyhow::Result<Vec<IncomingSms>> {
        let _guard = self.operation_lock.lock().await;
        self.ensure_authenticated().await?;
        let mut entries = self.list_inbox(SmsBoxType::LocalInbox).await?;
        match self.list_inbox(SmsBoxType::SimInbox).await {
            Ok(mut sim_entries) => entries.append(&mut sim_entries),
            Err(error) => log::debug!("Huawei SIM inbox unavailable: {error}"),
        }

        let mut messages = Vec::new();
        for entry in entries {
            if entry.is_user_message() {
                messages.push(entry.message);
                continue;
            }

            // Delivery confirmations are modem status events, not messages from
            // the contact. Feeding an empty receipt to the LLM can produce an
            // extra fallback reply after a multipart send.
            log::debug!(
                "Ignoring Huawei SMS status entry {} ({:?})",
                entry.message.id,
                entry.sms_type
            );
            if let Err(error) = self.client.sms().mark_read(&entry.message.id).await {
                log::warn!(
                    "Failed to mark Huawei SMS status entry {} as read: {error}",
                    entry.message.id
                );
            }
        }
        Ok(messages)
    }

    async fn mark_read(&self, message_id: &str) -> anyhow::Result<()> {
        let _guard = self.operation_lock.lock().await;
        self.ensure_authenticated().await?;
        self.client
            .sms()
            .mark_read(message_id)
            .await
            .context("failed to mark Huawei SMS as read")
    }

    async fn send_segment(&self, phone_number: &str, text: &str) -> anyhow::Result<()> {
        let _guard = self.operation_lock.lock().await;
        self.ensure_authenticated().await?;
        let request = SmsSendRequest::new(phone_number, text);
        self.client
            .sms()
            .send(&request)
            .await
            .context("failed to send Huawei SMS")
    }
}

/// Reply sender that delivers text as one or more SMS messages and optionally mirrors it.
pub struct SmsReplySender {
    gateway: Arc<dyn SmsGateway>,
    phone_number: String,
    telegram_forward: Option<(teloxide::Bot, ChatId)>,
}

impl SmsReplySender {
    pub fn new(
        gateway: Arc<dyn SmsGateway>,
        phone_number: impl Into<String>,
        telegram_forward: Option<(teloxide::Bot, ChatId)>,
    ) -> Self {
        Self {
            gateway,
            phone_number: phone_number.into(),
            telegram_forward,
        }
    }

    pub async fn forward_incoming(&self, message: &IncomingSms) {
        let Some((bot, chat_id)) = &self.telegram_forward else {
            return;
        };
        let text = telegram_incoming_mirror_text(message);
        crate::bot_send::send_plain_message(bot, *chat_id, &text).await;
    }
}

fn telegram_incoming_mirror_text(message: &IncomingSms) -> String {
    format!(
        "{TELEGRAM_SMS_MIRROR_TAG}\nFrom {} ({})\n{}",
        message.phone_number, message.modem_date, message.text
    )
}

fn telegram_outgoing_mirror_text(phone_number: &str, text: &str) -> String {
    format!("{TELEGRAM_SMS_MIRROR_TAG}\nTo {phone_number}\n{text}")
}

/// Whether Telegram text is a display-only envelope emitted by the SMS mirror.
///
/// The envelope is checked independently of Telegram sender metadata because
/// relays can present mirrored messages as if a human authored them.
pub fn is_telegram_sms_mirror_text(text: &str) -> bool {
    text.lines().next() == Some(TELEGRAM_SMS_MIRROR_TAG)
}

#[async_trait]
impl TextReplySender for SmsReplySender {
    async fn send_text(&self, text: &str) -> anyhow::Result<()> {
        let prepared = prepare_sms_text(text);
        anyhow::ensure!(
            !prepared.trim().is_empty(),
            "SMS reply is empty after formatting"
        );
        let segments = split_sms(&prepared);
        for segment in &segments {
            self.gateway
                .send_segment(&self.phone_number, segment)
                .await?;
        }

        if let Some((bot, chat_id)) = &self.telegram_forward {
            let forwarded = telegram_outgoing_mirror_text(&self.phone_number, &prepared);
            crate::bot_send::send_plain_message(bot, *chat_id, &forwarded).await;
        }
        Ok(())
    }
}

/// Normalize common phone-number spellings for configuration lookup.
pub fn normalize_phone_number(phone_number: &str) -> String {
    let digits: String = phone_number.chars().filter(char::is_ascii_digit).collect();
    digits.strip_prefix("00").unwrap_or(&digits).to_string()
}

/// Replace avoidable UCS-2 characters while preserving characters required for meaning.
pub fn prepare_sms_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        if gsm_units(character).is_some() {
            output.push(character);
            continue;
        }
        match character {
            '\t' | '\u{00a0}' => output.push(' '),
            '`' | '\u{2018}' | '\u{2019}' | '\u{201a}' => output.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201e}' => output.push('"'),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => output.push('-'),
            '\u{2026}' => output.push_str("..."),
            '\u{2022}' | '\u{25cf}' | '\u{25e6}' => output.push('-'),
            c if c.is_control() => {}
            c if is_latin_character(c) => {
                if let Some(ascii) = deunicode::deunicode_char(c) {
                    output.extend(ascii.chars().filter(|part| gsm_units(*part).is_some()));
                }
            }
            c if is_decorative_symbol(c) => {}
            c => output.push(c),
        }
    }
    output
}

/// Split into standalone GSM-7 (160 septets) or UCS-2 (70 UTF-16 units) messages.
pub fn split_sms(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let gsm = text.chars().all(|character| gsm_units(character).is_some());
    let limit = if gsm {
        GSM_SEGMENT_SEPTETS
    } else {
        UCS2_SEGMENT_UNITS
    };
    split_by_units(text, limit, |character| {
        if gsm {
            gsm_units(character).unwrap_or(1)
        } else {
            character.len_utf16()
        }
    })
}

fn split_by_units(text: &str, limit: usize, unit_len: impl Fn(char) -> usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    let mut last_boundary = None;

    for (index, character) in text.char_indices() {
        let next_units = units + unit_len(character);
        if next_units > limit {
            let split = last_boundary
                .filter(|boundary| *boundary > start)
                .unwrap_or(index);
            chunks.push(text[start..split].to_string());
            start = split;
            units = text[start..index].chars().map(&unit_len).sum();
            last_boundary = None;
        }
        units += unit_len(character);
        if character.is_whitespace() {
            last_boundary = Some(index + character.len_utf8());
        }
    }
    if start < text.len() {
        chunks.push(text[start..].to_string());
    }
    chunks
}

fn gsm_units(character: char) -> Option<usize> {
    if GSM_BASIC.contains(character) {
        Some(1)
    } else if GSM_EXTENSION.contains(character) {
        Some(2)
    } else {
        None
    }
}

fn is_latin_character(character: char) -> bool {
    matches!(character as u32, 0x00c0..=0x024f | 0x1e00..=0x1eff)
}

fn is_decorative_symbol(character: char) -> bool {
    matches!(character as u32, 0x2600..=0x27ff | 0x1f000..=0x1faff)
}

#[cfg(test)]
#[path = "sms_tests.rs"]
mod tests;
