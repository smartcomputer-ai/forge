//! Conversation policy: when a message becomes a bot event (activation),
//! who may take a turn or steer the conversation (access), and the two
//! workflow-owned control commands.
//!
//! Batching is the chat trigger's coalescing; silent rooms (ambient context
//! without runs) are not a mode. A group message that does not activate is
//! dropped.

use api::{
    ChannelInbound, ChatAccess, ChatActivation, ChatGroupActivation, ChatScope, ChatTurnAccess,
};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::inbound::ChannelAuthorization;

/// Prefixes that activate a group message when the trigger declares none.
pub const DEFAULT_TRIGGER_PREFIXES: [&str; 2] = ["/ask", "/lightspeed"];

/// The effective activation of a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivationMode {
    /// Direct conversations are always active.
    Dm,
    Mention,
    Always,
}

impl ActivationMode {
    pub fn resolve(scope: ChatScope, group: ChatGroupActivation) -> Self {
        match (scope, group) {
            (ChatScope::Direct, _) => Self::Dm,
            (ChatScope::Group, ChatGroupActivation::Mention) => Self::Mention,
            (ChatScope::Group, ChatGroupActivation::Always) => Self::Always,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::Mention => "mention",
            Self::Always => "always",
        }
    }
}

/// The group mode a conversation starts in; `/activation` changes it later.
pub fn initial_group_activation(activation: &ChatActivation) -> ChatGroupActivation {
    activation.group.unwrap_or_default()
}

/// Trimmed, de-duplicated trigger prefixes; the defaults when the trigger
/// declares none.
pub fn trigger_prefixes(activation: &ChatActivation) -> Vec<String> {
    if activation.trigger_prefixes.is_empty() {
        unique_non_empty(
            DEFAULT_TRIGGER_PREFIXES
                .iter()
                .map(|prefix| (*prefix).to_owned()),
        )
    } else {
        unique_non_empty(activation.trigger_prefixes.iter().cloned())
    }
}

/// Trimmed, de-duplicated mention names (with or without a leading `@`).
pub fn mention_names(activation: &ChatActivation) -> Vec<String> {
    unique_non_empty(activation.mention_names.iter().cloned())
}

fn unique_non_empty(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && !unique.iter().any(|existing| existing == trimmed) {
            unique.push(trimmed.to_owned());
        }
    }
    unique
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// Neither text nor media.
    Empty,
    /// A bare trigger prefix with nothing after it.
    EmptyTrigger,
    /// Group traffic not addressed to the bot.
    Ambient,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Classification {
    /// The message becomes a bot event with this (activated) text.
    Emit {
        text: String,
    },
    Drop {
        reason: DropReason,
    },
}

/// Decide whether an inbound message activates the bot and with what text
/// (prefix or mention stripped). `current_group_mode` is the conversation's
/// live group mode, which `/activation` may have changed since the trigger
/// was configured.
pub fn classify_inbound(
    scope: ChatScope,
    activation: &ChatActivation,
    current_group_mode: ChatGroupActivation,
    inbound: &ChannelInbound,
) -> Classification {
    let text = inbound.text.trim();
    let has_media = !inbound.media.is_empty();
    if text.is_empty() && !has_media {
        return Classification::Drop {
            reason: DropReason::Empty,
        };
    }
    let prefixes = trigger_prefixes(activation);
    let names = mention_names(activation);
    let triggered = if text.is_empty() {
        None
    } else {
        extract_triggered_text(text, &prefixes, &names)
    };
    if let Some(triggered) = triggered {
        return if triggered.is_empty() && !has_media {
            Classification::Drop {
                reason: DropReason::EmptyTrigger,
            }
        } else {
            Classification::Emit { text: triggered }
        };
    }
    let mode = ActivationMode::resolve(scope, current_group_mode);
    if inbound.is_direct || matches!(mode, ActivationMode::Dm | ActivationMode::Always) {
        return Classification::Emit {
            text: text.to_owned(),
        };
    }
    if inbound.mentioned_bot || inbound.is_reply_to_bot {
        return Classification::Emit {
            text: strip_named_mention(text, &names),
        };
    }
    Classification::Drop {
        reason: DropReason::Ambient,
    }
}

/// The text after a leading trigger prefix (`/ask`, `/ask@bot`) or a
/// leading mention (`@name`, `@name:`), or `None` when the message starts
/// with neither.
pub fn extract_triggered_text(
    text: &str,
    prefixes: &[String],
    mention_names: &[String],
) -> Option<String> {
    for prefix in prefixes {
        let Some(rest) = strip_prefix_ignore_case(text, prefix) else {
            continue;
        };
        let rest = strip_bot_suffix(rest);
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return Some(rest.trim().to_owned());
        }
    }
    for name in mention_names {
        let name = name.strip_prefix('@').unwrap_or(name);
        let Some(rest) = strip_prefix_ignore_case(text, "@")
            .and_then(|rest| strip_prefix_ignore_case(rest, name))
        else {
            continue;
        };
        if rest.is_empty() {
            return Some(String::new());
        }
        let rest = rest.strip_prefix([':', ',']).unwrap_or(rest);
        if rest.starts_with(char::is_whitespace) {
            return Some(rest.trim().to_owned());
        }
    }
    None
}

/// Remove the first `@name` (with an optional `:`/`,` and following
/// whitespace) for each mention name and collapse whitespace; the original
/// text when nothing would remain.
pub fn strip_named_mention(text: &str, names: &[String]) -> String {
    let mut result = text.to_owned();
    for name in names {
        let name = name.strip_prefix('@').unwrap_or(name);
        if name.is_empty() {
            continue;
        }
        if let Some((start, end)) = find_mention(&result, name) {
            result.replace_range(start..end, " ");
        }
    }
    let collapsed = result.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        text.to_owned()
    } else {
        collapsed
    }
}

/// Byte range of the first `@name\b[:,]?\s*` in `text`, case-insensitively.
fn find_mention(text: &str, name: &str) -> Option<(usize, usize)> {
    let last_is_word = name.chars().next_back().is_some_and(is_word_char);
    for (start, ch) in text.char_indices() {
        if ch != '@' {
            continue;
        }
        let Some(rest) = strip_prefix_ignore_case(&text[start + 1..], name) else {
            continue;
        };
        let next_is_word = rest.chars().next().is_some_and(is_word_char);
        if last_is_word == next_is_word {
            continue;
        }
        let rest = rest.strip_prefix([':', ',']).unwrap_or(rest);
        let rest = rest.trim_start();
        return Some((start, text.len() - rest.len()));
    }
    None
}

/// Strip `prefix` from the start of `text` comparing characters
/// case-insensitively; the remainder of `text` on success.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut chars = text.chars();
    for expected in prefix.chars() {
        let actual = chars.next()?;
        if actual != expected && !actual.to_lowercase().eq(expected.to_lowercase()) {
            return None;
        }
    }
    Some(chars.as_str())
}

/// Strip a Telegram-style `@bot_name` command suffix, when present.
fn strip_bot_suffix(text: &str) -> &str {
    let Some(rest) = text.strip_prefix('@') else {
        return text;
    };
    let word_len = rest
        .chars()
        .take_while(|ch| is_word_char(*ch))
        .map(char::len_utf8)
        .sum::<usize>();
    if word_len == 0 {
        text
    } else {
        &rest[word_len..]
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// `YYYY-MM-DD HH:MMZ` for a millisecond timestamp.
pub fn format_timestamp(timestamp_ms: i64) -> String {
    match DateTime::from_timestamp_millis(timestamp_ms) {
        Some(time) => time.format("%Y-%m-%d %H:%MZ").to_string(),
        None => format!("{timestamp_ms}ms"),
    }
}

/// The one-line rendering of a message for the model: who, when, what.
/// Never a provider id.
pub fn format_message_line(sender_name: &str, timestamp_ms: i64, text: &str) -> String {
    format!("{sender_name} ({}): {text}", format_timestamp(timestamp_ms))
}

/// Authorize a sender by provider handle against the trigger's access
/// policy: turns are open or allowlisted; control commands are for the
/// listed controllers only.
pub fn authorize_sender(access: &ChatAccess, sender_id: &str) -> ChannelAuthorization {
    let listed = |handles: &[String]| handles.iter().any(|handle| handle == sender_id);
    ChannelAuthorization {
        turn_allowed: match access.turn {
            ChatTurnAccess::Anyone => true,
            ChatTurnAccess::Listed => listed(&access.allowed),
        },
        control_allowed: listed(&access.controllers),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlCommand {
    /// `/activation mention|always`.
    Activation { mode: ChatGroupActivation },
    /// `/activation` with no or an unknown mode: explain the two that exist.
    ActivationHelp,
    /// `/status`.
    Status,
}

/// Parse a workflow-owned control command, tolerating a `@bot` suffix.
pub fn parse_control_command(text: &str) -> Option<ControlCommand> {
    let trimmed = text.trim();
    if let Some(rest) = strip_prefix_ignore_case(trimmed, "/activation") {
        let rest = strip_bot_suffix(rest);
        if rest.is_empty() {
            return Some(ControlCommand::ActivationHelp);
        }
        if rest.starts_with(char::is_whitespace) {
            let argument = rest.trim_start();
            if !argument.is_empty() && argument.chars().all(is_word_char) {
                return Some(match argument.to_ascii_lowercase().as_str() {
                    "mention" => ControlCommand::Activation {
                        mode: ChatGroupActivation::Mention,
                    },
                    "always" => ControlCommand::Activation {
                        mode: ChatGroupActivation::Always,
                    },
                    _ => ControlCommand::ActivationHelp,
                });
            }
        }
    }
    if let Some(rest) = strip_prefix_ignore_case(trimmed, "/status")
        && strip_bot_suffix(rest).is_empty()
    {
        return Some(ControlCommand::Status);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ChannelInboundMedia, ChannelMediaKind};

    fn group_inbound() -> ChannelInbound {
        ChannelInbound {
            message_id: "42".to_owned(),
            chat_id: "-100123".to_owned(),
            thread_id: None,
            sender_id: "7".to_owned(),
            sender_name: "Lukas".to_owned(),
            timestamp_ms: 1_700_000_000_000,
            text: "ordinary group traffic".to_owned(),
            media: Vec::new(),
            is_direct: false,
            mentioned_bot: false,
            is_reply_to_bot: false,
        }
    }

    fn activation(
        group: Option<ChatGroupActivation>,
        prefixes: &[&str],
        names: &[&str],
    ) -> ChatActivation {
        ChatActivation {
            group,
            trigger_prefixes: prefixes.iter().map(|value| (*value).to_owned()).collect(),
            mention_names: names.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn emit(text: &str) -> Classification {
        Classification::Emit {
            text: text.to_owned(),
        }
    }

    fn drop(reason: DropReason) -> Classification {
        Classification::Drop { reason }
    }

    #[test]
    fn forces_direct_conversations_active_and_ignores_group_only_settings() {
        assert_eq!(
            ActivationMode::resolve(ChatScope::Direct, ChatGroupActivation::Always),
            ActivationMode::Dm
        );
        assert_eq!(
            ActivationMode::resolve(ChatScope::Group, ChatGroupActivation::Mention),
            ActivationMode::Mention
        );
        let settings = activation(Some(ChatGroupActivation::Always), &[], &[]);
        assert_eq!(trigger_prefixes(&settings), vec!["/ask", "/lightspeed"]);
        assert!(mention_names(&settings).is_empty());
        assert_eq!(
            initial_group_activation(&settings),
            ChatGroupActivation::Always
        );
        assert_eq!(
            initial_group_activation(&ChatActivation::default()),
            ChatGroupActivation::Mention
        );
        let messy = activation(None, &[" /ask ", "/ask", "", "/go"], &["@bot", "bot ", ""]);
        assert_eq!(trigger_prefixes(&messy), vec!["/ask", "/go"]);
        assert_eq!(mention_names(&messy), vec!["@bot", "bot"]);
    }

    #[test]
    fn drops_ambient_group_traffic_and_activates_native_mentions() {
        let settings = activation(Some(ChatGroupActivation::Mention), &[], &["lightspeed"]);
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &group_inbound()
            ),
            drop(DropReason::Ambient)
        );
        let mentioned = ChannelInbound {
            text: "@lightspeed, help please".to_owned(),
            mentioned_bot: true,
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &mentioned
            ),
            emit("help please")
        );
        let always = activation(Some(ChatGroupActivation::Always), &[], &[]);
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &always,
                ChatGroupActivation::Always,
                &group_inbound()
            ),
            emit("ordinary group traffic")
        );
        // The live mode wins over the configured one.
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Always,
                &group_inbound()
            ),
            emit("ordinary group traffic")
        );
        assert_eq!(
            classify_inbound(
                ChatScope::Direct,
                &settings,
                ChatGroupActivation::Mention,
                &group_inbound()
            ),
            emit("ordinary group traffic")
        );
    }

    #[test]
    fn strips_mid_text_mentions_on_replies_and_mentions() {
        let settings = activation(None, &[], &["lightspeed"]);
        let reply = ChannelInbound {
            text: "hey @LightSpeed: what's up".to_owned(),
            is_reply_to_bot: true,
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &reply
            ),
            emit("hey what's up")
        );
        let names = vec!["lightspeed".to_owned()];
        assert_eq!(strip_named_mention("@lightspeed", &names), "@lightspeed");
        assert_eq!(
            strip_named_mention("ping @lightspeedy now", &names),
            "ping @lightspeedy now"
        );
        assert_eq!(strip_named_mention("a  @lightspeed,   b", &names), "a b");
        let at_names = vec!["@lightspeed".to_owned()];
        assert_eq!(strip_named_mention("x @lightspeed y", &at_names), "x y");
    }

    #[test]
    fn allows_explicit_prefixes_and_keeps_media_only_messages() {
        let settings = activation(None, &["/ask"], &[]);
        let ask = ChannelInbound {
            text: "/ask investigate".to_owned(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &ask
            ),
            emit("investigate")
        );
        let bare = ChannelInbound {
            text: "/ask".to_owned(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &bare
            ),
            drop(DropReason::EmptyTrigger)
        );
        let suffixed = ChannelInbound {
            text: "/ASK@lightspeed_bot   look".to_owned(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &suffixed
            ),
            emit("look")
        );
        let not_a_prefix = ChannelInbound {
            text: "/asking around".to_owned(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &not_a_prefix
            ),
            drop(DropReason::Ambient)
        );
        let photo = ChannelInbound {
            text: String::new(),
            media: vec![ChannelInboundMedia {
                file_id: "f".to_owned(),
                kind: ChannelMediaKind::Image,
                mime: "image/jpeg".to_owned(),
                name: None,
                byte_size: None,
            }],
            is_direct: true,
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Direct,
                &settings,
                ChatGroupActivation::Mention,
                &photo
            ),
            emit("")
        );
        let empty = ChannelInbound {
            text: "   ".to_owned(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Direct,
                &settings,
                ChatGroupActivation::Mention,
                &empty
            ),
            drop(DropReason::Empty)
        );
        let bare_with_media = ChannelInbound {
            text: "/ask".to_owned(),
            media: photo.media.clone(),
            ..group_inbound()
        };
        assert_eq!(
            classify_inbound(
                ChatScope::Group,
                &settings,
                ChatGroupActivation::Mention,
                &bare_with_media
            ),
            emit("")
        );
    }

    #[test]
    fn extracts_leading_mentions_exactly() {
        let prefixes = Vec::new();
        let names = vec!["lightspeed".to_owned()];
        assert_eq!(
            extract_triggered_text("@lightspeed: go", &prefixes, &names).as_deref(),
            Some("go")
        );
        assert_eq!(
            extract_triggered_text("@lightspeed", &prefixes, &names).as_deref(),
            Some("")
        );
        assert_eq!(
            extract_triggered_text("@lightspeed,", &prefixes, &names),
            None
        );
        assert_eq!(
            extract_triggered_text("@lightspeed:go", &prefixes, &names),
            None
        );
        assert_eq!(
            extract_triggered_text("@lightspeedy go", &prefixes, &names),
            None
        );
    }

    #[test]
    fn renders_the_message_line_without_provider_ids() {
        let line = format_message_line("Lukas", 1_700_000_000_000, "ordinary group traffic");
        assert_eq!(line, "Lukas (2023-11-14 22:13Z): ordinary group traffic");
        assert!(!line.contains("-100123"));
        assert!(!line.contains("#42"));
        assert_eq!(format_timestamp(0), "1970-01-01 00:00Z");
    }

    #[test]
    fn authorizes_by_handle_allowlists() {
        let open = ChatAccess::default();
        assert_eq!(
            authorize_sender(&open, "7"),
            ChannelAuthorization {
                turn_allowed: true,
                control_allowed: false,
            }
        );
        let listed = ChatAccess {
            turn: ChatTurnAccess::Listed,
            allowed: vec!["7".to_owned()],
            controllers: vec!["9".to_owned()],
        };
        assert_eq!(
            authorize_sender(&listed, "7"),
            ChannelAuthorization {
                turn_allowed: true,
                control_allowed: false,
            }
        );
        assert_eq!(
            authorize_sender(&listed, "9"),
            ChannelAuthorization {
                turn_allowed: false,
                control_allowed: true,
            }
        );
        assert_eq!(
            authorize_sender(&listed, "8"),
            ChannelAuthorization::default()
        );
        let controller_only = ChatAccess {
            controllers: vec!["7".to_owned()],
            ..ChatAccess::default()
        };
        assert_eq!(
            authorize_sender(&controller_only, "7"),
            ChannelAuthorization {
                turn_allowed: true,
                control_allowed: true,
            }
        );
    }

    #[test]
    fn parses_workflow_owned_control_commands_including_bot_suffixes() {
        assert_eq!(
            parse_control_command("/activation@lightspeed_bot always"),
            Some(ControlCommand::Activation {
                mode: ChatGroupActivation::Always,
            })
        );
        assert_eq!(
            parse_control_command("  /Activation MENTION "),
            Some(ControlCommand::Activation {
                mode: ChatGroupActivation::Mention,
            })
        );
        // Silent rooms are not a mode; the help text explains the two that are.
        assert_eq!(
            parse_control_command("/activation silent"),
            Some(ControlCommand::ActivationHelp)
        );
        assert_eq!(
            parse_control_command("/activation invalid"),
            Some(ControlCommand::ActivationHelp)
        );
        assert_eq!(
            parse_control_command("/activation"),
            Some(ControlCommand::ActivationHelp)
        );
        assert_eq!(
            parse_control_command("/activation@bot"),
            Some(ControlCommand::ActivationHelp)
        );
        assert_eq!(parse_control_command("/activation always now"), None);
        assert_eq!(parse_control_command("/activations"), None);
        assert_eq!(
            parse_control_command("/status"),
            Some(ControlCommand::Status)
        );
        assert_eq!(
            parse_control_command("/STATUS@bot"),
            Some(ControlCommand::Status)
        );
        assert_eq!(parse_control_command("/status now"), None);
        assert_eq!(parse_control_command("please /status"), None);
        assert_eq!(parse_control_command("hello"), None);
    }
}
